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
use vclogg_core::{CompressedRows, LineReader, LogDocument};

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
    resolution: TimestampResolutionState,
    reader: LineReader,
}

#[derive(Default)]
struct TimestampResolutionState {
    last_result_row: Option<usize>,
    last_timestamp: Option<u64>,
    timestamps_absent: bool,
}

pub(crate) fn save_timestamp_merged_to_unique_temp(export: &ResultExport) -> Result<PathBuf> {
    let export = prepare_timestamp_merge_export(export)?;
    save_to_unique_temp_with(
        &export,
        "global-search-merged-results.log",
        write_timestamp_merged_export,
    )
}

fn prepare_timestamp_merge_export(export: &ResultExport) -> Result<ResultExport> {
    let ResultExport::Global { groups } = export else {
        bail!(crate::tr!(
            "只有全局搜索结果可以按时间戳合并",
            "Only global search results can be merged by timestamp"
        ));
    };
    let groups = prepare_timestamp_merge_groups(groups, |path| {
        let (document, pending_cache_write) =
            if let Some(cache_root) = crate::app_paths::cache_dir() {
                LogDocument::open_with_index_cache(path, cache_root.join("VCLogg2").join("index"))?
            } else {
                (LogDocument::open(path)?, None)
            };
        if let Some(cache_write) = pending_cache_write {
            _ = cache_write.persist();
        }
        Ok(document)
    })?;
    Ok(ResultExport::Global {
        groups: groups.into(),
    })
}

fn prepare_timestamp_merge_groups(
    groups: &[ExportGroup],
    mut open_complete: impl FnMut(&Path) -> Result<LogDocument>,
) -> Result<Vec<ExportGroup>> {
    groups
        .iter()
        .map(|group| {
            if group.document.has_complete_line_index() {
                return Ok(group.clone());
            }
            let document = open_complete(group.document.path()).with_context(|| {
                crate::tr_args!(
                    "无法为时间戳合并读取 {} 的完整行索引",
                    "Couldn’t read the complete line index for {} during timestamp merge",
                    group.path.display()
                )
            })?;
            if !group.document.same_source_snapshot(&document) {
                bail!(crate::tr_args!(
                    "{} 的内容在搜索后已改变，请重新搜索后再合并",
                    "{} changed after the search; search again before merging",
                    group.path.display()
                ));
            }
            Ok(ExportGroup {
                path: group.path.clone(),
                document: Arc::new(document),
                rows: group.rows.clone(),
            })
        })
        .collect()
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
            resolution: TimestampResolutionState::default(),
            reader: LineReader::default(),
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
        let line = cursor
            .reader
            .line(&cursor.group.document, row)
            .ok_or_else(|| {
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
    let TimestampMergeCursor {
        group,
        resolution,
        reader,
        ..
    } = cursor;
    let document = &group.document;
    resolve_row_timestamp_with(resolution, row, document.source_line_count(), |candidate| {
        let line = reader.line(document, candidate).ok_or_else(|| {
            anyhow!(crate::tr_args!(
                "{} 的第 {} 行已不在时间戳合并快照中",
                "Line {} at row {} is no longer in the timestamp-merge snapshot",
                document.path().display(),
                candidate + 1
            ))
        })?;
        Ok(match_log_timestamp(&line))
    })
}

fn resolve_row_timestamp_with(
    state: &mut TimestampResolutionState,
    row: usize,
    source_line_count: usize,
    mut timestamp_at: impl FnMut(usize) -> Result<Option<u64>>,
) -> Result<Option<u64>> {
    if state.timestamps_absent {
        state.last_result_row = Some(row);
        return Ok(None);
    }
    let mut timestamp = timestamp_at(row)?;
    if timestamp.is_none() {
        let first_unchecked = state.last_result_row.map_or(0, |row| row.saturating_add(1));
        for candidate in (first_unchecked..row).rev() {
            timestamp = timestamp_at(candidate)?;
            if timestamp.is_some() {
                break;
            }
        }
        timestamp = timestamp.or(state.last_timestamp);
        if timestamp.is_none() {
            for candidate in row.saturating_add(1)..source_line_count {
                timestamp = timestamp_at(candidate)?;
                if timestamp.is_some() {
                    break;
                }
            }
            if timestamp.is_none() {
                state.timestamps_absent = true;
            }
        }
    }
    state.last_result_row = Some(row);
    state.last_timestamp = timestamp;
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
    let mut reader = LineReader::default();
    for row in rows.iter() {
        let line = reader.line(document, row).ok_or_else(|| {
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

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, fs, path::PathBuf, sync::Arc, time::SystemTime};

    use vclogg_core::LogDocument;

    use super::{
        ExportGroup, TimestampResolutionState, prepare_timestamp_merge_groups,
        resolve_row_timestamp_with,
    };

    struct TemporaryFile(PathBuf);

    impl TemporaryFile {
        fn new(name: &str, contents: &[u8]) -> Self {
            let nonce = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .expect("测试时间应晚于 Unix epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "vclogg2-result-export-{name}-{}-{nonce}.log",
                std::process::id()
            ));
            fs::write(&path, contents).expect("应能创建结果导出测试日志");
            Self(path)
        }
    }

    impl Drop for TemporaryFile {
        fn drop(&mut self) {
            _ = fs::remove_file(&self.0);
        }
    }

    #[test]
    fn timestamp_resolution_scans_a_timestamp_free_source_only_once() {
        let mut state = TimestampResolutionState::default();
        let inspected = RefCell::new(Vec::new());

        assert_eq!(
            resolve_row_timestamp_with(&mut state, 1, 5, |row| {
                inspected.borrow_mut().push(row);
                Ok(None)
            })
            .expect("时间戳探测应完成"),
            None
        );
        assert_eq!(*inspected.borrow(), vec![1, 0, 2, 3, 4]);
        assert!(state.timestamps_absent);

        inspected.borrow_mut().clear();
        assert_eq!(
            resolve_row_timestamp_with(&mut state, 3, 5, |row| {
                inspected.borrow_mut().push(row);
                Ok(None)
            })
            .expect("已知无时间戳时应直接完成"),
            None
        );
        assert!(inspected.borrow().is_empty());
    }

    #[test]
    fn timestamp_resolution_propagates_unavailable_source_rows() {
        let mut state = TimestampResolutionState::default();

        let result = resolve_row_timestamp_with(&mut state, 2, 4, |row| {
            if row == 1 {
                anyhow::bail!("source row unavailable");
            }
            Ok(None)
        });

        assert!(result.is_err());
        assert!(!state.timestamps_absent);
    }

    #[test]
    fn timestamp_merge_materializes_sparse_sources_only_when_needed() {
        let file = TemporaryFile::new(
            "sparse-timestamp",
            b"01-01 00:00:00.001 first\ncontinuation\n01-01 00:00:00.002 second\n",
        );
        let complete = LogDocument::open(&file.0).expect("应能打开时间戳测试日志");
        let rows = [1, 2].into_iter().collect();
        let sparse = Arc::new(complete.project_source_rows(&rows));
        let groups = [ExportGroup {
            path: file.0.clone(),
            document: sparse,
            rows,
        }];
        let mut opens = 0;

        let prepared = prepare_timestamp_merge_groups(&groups, |path| {
            opens += 1;
            LogDocument::open(path)
        })
        .expect("稀疏结果应能重建同快照的完整索引");

        assert_eq!(opens, 1);
        assert!(prepared[0].document.has_complete_line_index());
        assert_eq!(
            prepared[0].document.line(0).as_deref(),
            Some("01-01 00:00:00.001 first")
        );
    }

    #[test]
    fn timestamp_merge_rejects_a_changed_sparse_source() {
        let file = TemporaryFile::new("changed-timestamp", b"first\nsecond\nthird\n");
        let complete = LogDocument::open(&file.0).expect("应能打开时间戳测试日志");
        let rows = [1].into_iter().collect();
        let groups = [ExportGroup {
            path: file.0.clone(),
            document: Arc::new(complete.project_source_rows(&rows)),
            rows,
        }];
        fs::write(&file.0, b"FIRST\nsecond\nthird\n").expect("应能修改时间戳测试日志");

        let result = prepare_timestamp_merge_groups(&groups, |path| LogDocument::open(path));

        assert!(result.is_err());
    }
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
