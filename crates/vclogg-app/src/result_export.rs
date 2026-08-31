use std::{
    cmp::Reverse,
    collections::BinaryHeap,
    fs::{self, File, OpenOptions},
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    process,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::SystemTime,
};

use anyhow::{Context as _, Result, anyhow, bail};
use vclogg_core::{CompressedRows, LogDocument};

static UNIQUE_PATH_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug)]
pub(crate) struct TemporaryResultFile {
    pub(crate) path: PathBuf,
    pub(crate) size: u64,
    pub(crate) modified: Option<SystemTime>,
}

pub(crate) fn temporary_result_files() -> Result<Vec<TemporaryResultFile>> {
    let root = std::env::temp_dir();
    let mut files = Vec::new();
    for entry in match fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(files),
        Err(error) => return Err(error.into()),
    } {
        let entry = entry?;
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with("vclogg2-search-") || !entry.file_type()?.is_dir() {
            continue;
        }
        for child in fs::read_dir(entry.path())? {
            let child = child?;
            let metadata = child.metadata()?;
            if metadata.is_file() {
                files.push(TemporaryResultFile {
                    path: child.path(),
                    size: metadata.len(),
                    modified: metadata.modified().ok(),
                });
            }
        }
    }
    files.sort_by_key(|entry| (Reverse(entry.modified), entry.path.clone()));
    Ok(files)
}

pub(crate) fn remove_empty_temporary_result_parent(path: &Path) {
    let Some(parent) = path.parent() else {
        return;
    };
    let root = std::env::temp_dir();
    let valid_parent = parent.parent().is_some_and(|candidate| candidate == root)
        && parent
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with("vclogg2-search-"));
    if valid_parent {
        _ = fs::remove_dir(parent);
    }
}

pub(crate) fn is_temporary_result_path(path: &Path) -> bool {
    let root = std::env::temp_dir();
    path.parent().is_some_and(|parent| {
        parent.parent().is_some_and(|candidate| candidate == root)
            && parent
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("vclogg2-search-"))
    })
}

#[derive(Clone)]
pub(crate) struct ExportGroup {
    pub(crate) path: PathBuf,
    pub(crate) document: Arc<LogDocument>,
    pub(crate) rows: CompressedRows,
}

#[derive(Clone)]
pub(crate) enum ResultExport {
    Single {
        document: Arc<LogDocument>,
        rows: CompressedRows,
    },
    Global {
        groups: Arc<[ExportGroup]>,
    },
}

impl ResultExport {
    pub(crate) fn row_count(&self) -> usize {
        match self {
            Self::Single { rows, .. } => rows.len(),
            Self::Global { groups } => groups.iter().map(|group| group.rows.len()).sum(),
        }
    }
}

pub(crate) fn save(export: &ResultExport, target: &Path) -> Result<usize> {
    let parent = target
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).with_context(|| {
        crate::tr_args!(
            "无法访问保存目录 {}",
            "Couldn’t access the save directory {}",
            parent.display()
        )
    })?;
    let (staging_path, staging_file) = create_staging_file(parent, target.file_name())?;

    let write_result = write_export(export, staging_file).and_then(|row_count| {
        commit_staging(&staging_path, target)?;
        Ok(row_count)
    });
    if write_result.is_err() {
        let _ = fs::remove_file(&staging_path);
    }
    write_result
}

pub(crate) fn save_to_unique_temp(export: &ResultExport) -> Result<PathBuf> {
    save_to_unique_temp_with(export, "search-results.log", write_export)
}

struct TimestampMergeCursor {
    group: ExportGroup,
    next_result_ix: usize,
    last_result_row: Option<usize>,
    last_timestamp: Option<u64>,
}

pub(crate) fn save_timestamp_merged_to_unique_temp(export: &ResultExport) -> Result<PathBuf> {
    save_to_unique_temp_with(
        export,
        "global-search-merged-results.log",
        write_timestamp_merged_export,
    )
}

fn save_to_unique_temp_with(
    export: &ResultExport,
    file_name: &str,
    write: fn(&ResultExport, File) -> Result<usize>,
) -> Result<PathBuf> {
    let root = std::env::temp_dir();
    for _ in 0..128 {
        let id = UNIQUE_PATH_ID.fetch_add(1, Ordering::Relaxed);
        let directory = root.join(format!("vclogg2-search-{}-{id}", process::id()));
        match fs::create_dir(&directory) {
            Ok(()) => {
                let target = directory.join(file_name);
                let result = (|| {
                    let file = OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&target)?;
                    write(export, file)
                })();
                if let Err(error) = result {
                    let _ = fs::remove_dir_all(&directory);
                    return Err(error);
                }
                return Ok(target);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error).with_context(|| {
                    crate::tr_args!(
                        "无法创建临时结果目录 {}",
                        "Couldn’t create the temporary result directory {}",
                        directory.display()
                    )
                });
            }
        }
    }
    bail!(crate::tr!(
        "无法为搜索结果分配唯一临时目录",
        "Couldn’t allocate a unique temporary directory for search results"
    ))
}

fn write_timestamp_merged_export(export: &ResultExport, file: File) -> Result<usize> {
    let ResultExport::Global { groups } = export else {
        bail!(crate::tr!(
            "只有全局搜索结果可以按时间戳合并",
            "Only global search results can be merged by timestamp"
        ));
    };
    let mut cursors = groups
        .iter()
        .cloned()
        .map(|group| TimestampMergeCursor {
            group,
            next_result_ix: 0,
            last_result_row: None,
            last_timestamp: None,
        })
        .collect::<Vec<_>>();
    let mut heads = BinaryHeap::<Reverse<(u64, usize, usize)>>::new();
    for (source_ix, cursor) in cursors.iter_mut().enumerate() {
        if let Some(row) = cursor.group.rows.first() {
            let timestamp = resolve_row_timestamp(cursor, row)?;
            heads.push(Reverse((timestamp.unwrap_or(u64::MAX), source_ix, row)));
            cursor.next_result_ix = 1;
        }
    }

    let mut writer = BufWriter::with_capacity(256 * 1024, file);
    let mut row_count = 0usize;
    while let Some(Reverse((_, source_ix, row))) = heads.pop() {
        let cursor = &mut cursors[source_ix];
        let line = cursor.group.document.line(row).ok_or_else(|| {
            anyhow!(crate::tr_args!(
                "{} 的第 {} 行已不在合并快照中",
                "Line {} at row {} is no longer in the merge snapshot",
                cursor.group.document.path().display(),
                row + 1
            ))
        })?;
        writer.write_all(line.as_bytes())?;
        writer.write_all(b"\n")?;
        row_count = row_count.saturating_add(1);

        if let Some(next_row) = cursor.group.rows.get(cursor.next_result_ix) {
            cursor.next_result_ix += 1;
            let timestamp = resolve_row_timestamp(cursor, next_row)?;
            heads.push(Reverse((
                timestamp.unwrap_or(u64::MAX),
                source_ix,
                next_row,
            )));
        }
    }
    writer.flush().context(crate::tr!(
        "无法刷新时间戳合并结果",
        "Couldn’t flush the timestamp-merged results"
    ))?;
    writer.get_ref().sync_all().context(crate::tr!(
        "无法将时间戳合并结果同步到磁盘",
        "Couldn’t synchronize the timestamp-merged results to disk"
    ))?;
    Ok(row_count)
}

fn resolve_row_timestamp(cursor: &mut TimestampMergeCursor, row: usize) -> Result<Option<u64>> {
    let mut timestamp = cursor
        .group
        .document
        .line(row)
        .and_then(|line| match_log_timestamp(&line));
    if timestamp.is_none() {
        let first_unchecked = cursor
            .last_result_row
            .map_or(0, |row| row.saturating_add(1));
        for candidate in (first_unchecked..row).rev() {
            timestamp = cursor
                .group
                .document
                .line(candidate)
                .and_then(|line| match_log_timestamp(&line));
            if timestamp.is_some() {
                break;
            }
        }
        timestamp = timestamp.or(cursor.last_timestamp);
        if timestamp.is_none() {
            for candidate in row.saturating_add(1)..cursor.group.document.source_line_count() {
                timestamp = cursor
                    .group
                    .document
                    .line(candidate)
                    .and_then(|line| match_log_timestamp(&line));
                if timestamp.is_some() {
                    break;
                }
            }
        }
    }
    cursor.last_result_row = Some(row);
    cursor.last_timestamp = timestamp;
    Ok(timestamp)
}

fn match_log_timestamp(text: &str) -> Option<u64> {
    let bytes = text.as_bytes();
    if bytes.len() < 16
        || bytes.get(2) != Some(&b'-')
        || !matches!(bytes.get(5), Some(b' ' | b'T'))
        || bytes.get(8) != Some(&b':')
        || bytes.get(11) != Some(&b':')
        || bytes.get(14) != Some(&b'.')
    {
        return None;
    }
    let number = |start: usize| -> Option<u64> {
        let first = bytes.get(start)?.checked_sub(b'0')?;
        let second = bytes.get(start + 1)?.checked_sub(b'0')?;
        (first < 10 && second < 10).then(|| u64::from(first) * 10 + u64::from(second))
    };
    let month = number(0)?;
    let day = number(3)?;
    let hour = number(6)?;
    let minute = number(9)?;
    let second = number(12)?;
    let days_in_month = [31_u64, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    if !(1..=12).contains(&month)
        || day == 0
        || day > days_in_month[month as usize - 1]
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    let mut microseconds = 0_u64;
    let mut digits = 0;
    for byte in bytes.iter().skip(15).take(6) {
        if !byte.is_ascii_digit() {
            break;
        }
        microseconds = microseconds * 10 + u64::from(*byte - b'0');
        digits += 1;
    }
    if digits == 0 || bytes.get(15 + digits).is_some_and(u8::is_ascii_digit) {
        return None;
    }
    microseconds *= 10_u64.pow(6 - digits as u32);
    Some(
        (((((month * 32 + day) * 24 + hour) * 60 + minute) * 60 + second) * 1_000_000)
            + microseconds,
    )
}

fn create_staging_file(
    parent: &Path,
    target_name: Option<&std::ffi::OsStr>,
) -> Result<(PathBuf, File)> {
    let target_name = target_name
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "results.log".to_string());
    for _ in 0..128 {
        let id = UNIQUE_PATH_ID.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!(".{target_name}.vclogg2-{}-{id}.tmp", process::id()));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error).with_context(|| {
                    crate::tr_args!(
                        "无法创建暂存文件 {}",
                        "Couldn’t create the staging file {}",
                        path.display()
                    )
                });
            }
        }
    }
    bail!(crate::tr_args!(
        "无法在 {} 中分配唯一暂存文件",
        "Couldn’t allocate a unique staging file in {}",
        parent.display()
    ))
}

fn write_export(export: &ResultExport, file: File) -> Result<usize> {
    let mut writer = BufWriter::with_capacity(256 * 1024, file);
    let row_count = match export {
        ResultExport::Single { document, rows } => write_rows(&mut writer, document, rows)?,
        ResultExport::Global { groups } => {
            let mut row_count = 0usize;
            for group in groups.iter() {
                writeln!(writer, "===== {} =====", group.path.display()).context(crate::tr!(
                    "无法写入全局结果文件标题",
                    "Couldn’t write the global-results file header"
                ))?;
                row_count = row_count.saturating_add(write_rows(
                    &mut writer,
                    &group.document,
                    &group.rows,
                )?);
            }
            row_count
        }
    };
    writer.flush().context(crate::tr!(
        "无法刷新结果文件",
        "Couldn’t flush the result file"
    ))?;
    writer.get_ref().sync_all().context(crate::tr!(
        "无法将结果文件同步到磁盘",
        "Couldn’t synchronize the result file to disk"
    ))?;
    Ok(row_count)
}

fn write_rows(
    writer: &mut BufWriter<File>,
    document: &LogDocument,
    rows: &CompressedRows,
) -> Result<usize> {
    for row in rows.iter() {
        let line = document.line(row).ok_or_else(|| {
            anyhow!(crate::tr_args!(
                "{} 的第 {} 行已不在导出快照中",
                "Line {} at row {} is no longer in the export snapshot",
                document.path().display(),
                row + 1
            ))
        })?;
        writer.write_all(line.as_bytes()).with_context(|| {
            crate::tr_args!(
                "无法写入 {} 的第 {} 行",
                "Couldn’t write {} at line {}",
                document.path().display(),
                row + 1
            )
        })?;
        writer.write_all(b"\n").context(crate::tr!(
            "无法写入结果换行符",
            "Couldn’t write the result line break"
        ))?;
    }
    Ok(rows.len())
}

#[cfg(not(windows))]
fn commit_staging(staging: &Path, target: &Path) -> Result<()> {
    fs::rename(staging, target).with_context(|| {
        crate::tr_args!(
            "无法用暂存文件替换 {}",
            "Couldn’t replace {} with the staging file",
            target.display()
        )
    })
}

#[cfg(windows)]
fn commit_staging(staging: &Path, target: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt as _;
    use windows_sys::Win32::Storage::FileSystem::ReplaceFileW;

    if !target.exists() {
        return fs::rename(staging, target).with_context(|| {
            crate::tr_args!(
                "无法保存结果到 {}",
                "Couldn’t save results to {}",
                target.display()
            )
        });
    }

    let target_wide = target
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let staging_wide = staging
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    // SAFETY: Both paths are owned, NUL-terminated UTF-16 buffers that remain
    // alive for the duration of the synchronous Windows API call. Optional
    // backup/exclusion pointers are intentionally null.
    let replaced = unsafe {
        ReplaceFileW(
            target_wide.as_ptr(),
            staging_wide.as_ptr(),
            std::ptr::null(),
            0,
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    if replaced == 0 {
        return Err(std::io::Error::last_os_error()).with_context(|| {
            crate::tr_args!(
                "无法原子替换 {}",
                "Couldn’t atomically replace {}",
                target.display()
            )
        });
    }
    Ok(())
}
