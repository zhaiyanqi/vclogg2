//! Bounded file navigation summaries. Call these services on a background worker.

use crate::CancellationToken;
use anyhow::{Result, ensure};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct DirectoryEntry {
    path: PathBuf,
    directory: bool,
    symlink: bool,
    hidden: bool,
}

impl DirectoryEntry {
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn is_directory(&self) -> bool {
        self.directory
    }
    pub fn is_symlink(&self) -> bool {
        self.symlink
    }
    pub fn is_hidden(&self) -> bool {
        self.hidden
    }
}

/// Enumerate device roots without probing or traversing their contents.
pub fn navigation_roots(cancellation: &CancellationToken) -> Result<Option<Vec<PathBuf>>> {
    if cancellation.is_cancelled() {
        return Ok(None);
    }
    #[cfg(windows)]
    {
        // GetLogicalDrives only returns a bit mask and takes no pointers.
        let mask = unsafe { windows_sys::Win32::Storage::FileSystem::GetLogicalDrives() };
        if mask == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(Some(
            (0..26)
                .filter(|ix| mask & (1 << ix) != 0)
                .map(|ix| PathBuf::from(format!("{}:\\", (b'A' + ix) as char)))
                .collect(),
        ))
    }
    #[cfg(not(windows))]
    {
        Ok(Some(vec![PathBuf::from("/")]))
    }
}

/// Enumerate one directory, following a directory link only when explicitly requested.
/// Reject ancestor aliases so manual expansion cannot grow a cyclic link tree.
pub fn navigation_directory(
    path: &Path,
    cancellation: &CancellationToken,
) -> Result<Option<Vec<DirectoryEntry>>> {
    if cancellation.is_cancelled() {
        return Ok(None);
    }
    let resolved = std::fs::canonicalize(path)?;
    for ancestor in path
        .ancestors()
        .skip(1)
        .filter(|path| !path.as_os_str().is_empty())
    {
        if cancellation.is_cancelled() {
            return Ok(None);
        }
        ensure!(
            std::fs::canonicalize(ancestor).ok().as_ref() != Some(&resolved),
            "Directory link refers to an ancestor: {}",
            path.display()
        );
    }
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(path)? {
        if cancellation.is_cancelled() {
            return Ok(None);
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        let kind = match entry.file_type() {
            Ok(kind) => kind,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        let hidden = entry.file_name().as_encoded_bytes().starts_with(b".");
        #[cfg(windows)]
        let hidden = {
            use std::os::windows::fs::MetadataExt;
            hidden
                || entry
                    .metadata()
                    .is_ok_and(|metadata| metadata.file_attributes() & 0x2 != 0)
        };
        entries.push(DirectoryEntry {
            path: entry.path(),
            directory: kind.is_dir()
                || (kind.is_symlink()
                    && std::fs::metadata(entry.path()).is_ok_and(|metadata| metadata.is_dir())),
            symlink: kind.is_symlink(),
            hidden,
        });
    }
    entries.sort_by(|a, b| {
        b.directory
            .cmp(&a.directory)
            .then_with(|| a.path.file_name().cmp(&b.path.file_name()))
    });
    Ok(Some(entries))
}
