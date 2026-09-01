//! Lifecycle management for on-disk index cache entries.

use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use anyhow::{Context as _, Result};

const DEFAULT_MAX_CACHE_BYTES: u64 = 512 * 1024 * 1024;
const DEFAULT_RETENTION: Duration = Duration::from_secs(90 * 24 * 60 * 60);
const DEFAULT_TEMPORARY_RETENTION: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Clone, Debug)]
pub struct IndexCacheInfo {
    pub directory: PathBuf,
    pub file_count: usize,
    pub byte_size: u64,
}

#[derive(Clone, Debug)]
pub struct IndexCacheClearResult {
    pub info: IndexCacheInfo,
    pub removed_file_count: usize,
    pub removed_byte_size: u64,
}

#[derive(Clone, Debug)]
pub struct IndexCacheCleanupResult {
    pub removed_file_count: usize,
    pub removed_byte_size: u64,
    pub retained_byte_size: u64,
}

#[derive(Clone)]
struct ManagedCacheEntry {
    path: PathBuf,
    byte_size: u64,
    modified: Option<SystemTime>,
    temporary: bool,
}

pub fn index_cache_info(directory: impl AsRef<Path>) -> Result<IndexCacheInfo> {
    let directory = directory.as_ref().to_path_buf();
    let entries = managed_cache_entries(&directory)?;
    Ok(IndexCacheInfo {
        directory,
        file_count: entries.len(),
        byte_size: entries.iter().map(|entry| entry.byte_size).sum(),
    })
}

pub fn clear_index_cache(directory: impl AsRef<Path>) -> Result<IndexCacheClearResult> {
    let directory = directory.as_ref().to_path_buf();
    let entries = managed_cache_entries(&directory)?;
    let mut removed_file_count = 0;
    let mut removed_byte_size = 0_u64;
    for entry in entries {
        let Some(expected_modified) = entry.modified else {
            continue;
        };
        let Ok(metadata) = fs::metadata(&entry.path) else {
            continue;
        };
        if !metadata.is_file()
            || metadata.len() != entry.byte_size
            || metadata.modified().ok() != Some(expected_modified)
        {
            continue;
        }
        if fs::remove_file(&entry.path).is_ok() {
            removed_file_count += 1;
            removed_byte_size = removed_byte_size.saturating_add(entry.byte_size);
        }
    }
    Ok(IndexCacheClearResult {
        info: index_cache_info(&directory)?,
        removed_file_count,
        removed_byte_size,
    })
}

/// Remove abandoned temporary files, entries older than 90 days, then the
/// oldest remaining indexes until the managed cache is at most 512 MiB.
/// Every deletion revalidates the original size and modification time so a
/// concurrently replaced cache is never removed from an earlier snapshot.
pub fn cleanup_index_cache(directory: impl AsRef<Path>) -> Result<IndexCacheCleanupResult> {
    let entries = managed_cache_entries(directory.as_ref())?;
    let now = SystemTime::now();
    let mut remove = vec![false; entries.len()];
    for (index, entry) in entries.iter().enumerate() {
        let Some(modified) = entry.modified else {
            continue;
        };
        let retention = if entry.temporary {
            DEFAULT_TEMPORARY_RETENTION
        } else {
            DEFAULT_RETENTION
        };
        remove[index] = now
            .duration_since(modified)
            .is_ok_and(|age| age > retention);
    }

    let mut retained_regular_bytes = entries
        .iter()
        .enumerate()
        .filter(|(index, entry)| !entry.temporary && !remove[*index])
        .map(|(_, entry)| entry.byte_size)
        .sum::<u64>();
    let mut oldest = entries
        .iter()
        .enumerate()
        .filter(|(index, entry)| !entry.temporary && !remove[*index] && entry.modified.is_some())
        .map(|(index, entry)| (index, entry.modified.expect("filtered above")))
        .collect::<Vec<_>>();
    oldest.sort_by_key(|(_, modified)| *modified);
    for (index, _) in oldest {
        if retained_regular_bytes <= DEFAULT_MAX_CACHE_BYTES {
            break;
        }
        remove[index] = true;
        retained_regular_bytes = retained_regular_bytes.saturating_sub(entries[index].byte_size);
    }

    let mut removed_file_count = 0;
    let mut removed_byte_size = 0_u64;
    for (index, entry) in entries.iter().enumerate() {
        if !remove[index] || !entry_unchanged(entry) {
            continue;
        }
        if fs::remove_file(&entry.path).is_ok() {
            removed_file_count += 1;
            removed_byte_size = removed_byte_size.saturating_add(entry.byte_size);
        }
    }
    let total = entries.iter().map(|entry| entry.byte_size).sum::<u64>();
    Ok(IndexCacheCleanupResult {
        removed_file_count,
        removed_byte_size,
        retained_byte_size: total.saturating_sub(removed_byte_size),
    })
}

fn entry_unchanged(entry: &ManagedCacheEntry) -> bool {
    let Some(expected_modified) = entry.modified else {
        return false;
    };
    let Ok(metadata) = fs::metadata(&entry.path) else {
        return false;
    };
    metadata.is_file()
        && metadata.len() == entry.byte_size
        && metadata.modified().ok() == Some(expected_modified)
}

fn managed_cache_entries(directory: &Path) -> Result<Vec<ManagedCacheEntry>> {
    let read_dir = match fs::read_dir(directory) {
        Ok(read_dir) => read_dir,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("无法读取索引缓存目录：{}", directory.display()));
        }
    };
    let mut entries = Vec::new();
    for entry in read_dir.flatten() {
        let path = entry.path();
        let Some(temporary) = managed_cache_path_kind(&path) else {
            continue;
        };
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if metadata.is_file() {
            entries.push(ManagedCacheEntry {
                path,
                byte_size: metadata.len(),
                modified: metadata.modified().ok(),
                temporary,
            });
        }
    }
    Ok(entries)
}

fn managed_cache_path_kind(path: &Path) -> Option<bool> {
    if path
        .extension()
        .is_some_and(|extension| extension == "vclog-index")
    {
        return Some(false);
    }
    let name = path.file_name().and_then(|name| name.to_str())?;
    let (hash, temporary) = name.split_once(".tmp-")?;
    (matches!(hash.len(), 16 | 64)
        && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        && !temporary.is_empty()
        && temporary
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'-'))
    .then_some(true)
}

#[cfg(test)]
mod tests {
    use super::managed_cache_path_kind;
    use std::path::Path;

    #[test]
    fn recognizes_current_and_legacy_managed_cache_names() {
        assert_eq!(
            managed_cache_path_kind(Path::new(
                "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef.vclog-index"
            )),
            Some(false)
        );
        assert_eq!(
            managed_cache_path_kind(Path::new(
                "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef.tmp-12-34"
            )),
            Some(true)
        );
        assert_eq!(
            managed_cache_path_kind(Path::new("0123456789abcdef.tmp-12-34")),
            Some(true)
        );
        assert_eq!(managed_cache_path_kind(Path::new("short.tmp-12-34")), None);
    }
}
