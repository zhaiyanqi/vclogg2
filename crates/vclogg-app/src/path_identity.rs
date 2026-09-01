use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

#[cfg(unix)]
use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};

#[cfg(windows)]
use std::os::windows::ffi::{OsStrExt as _, OsStringExt as _};

const ENCODED_PATH_PREFIX: &str = "\0vclogg-path-v1:";

#[cfg(not(windows))]
pub(crate) type PathMatchKey = PathBuf;
#[cfg(windows)]
pub(crate) type PathMatchKey = Vec<u16>;

#[cfg(not(windows))]
pub(crate) fn path_match_key(path: &Path) -> PathMatchKey {
    path.to_path_buf()
}

#[cfg(windows)]
pub(crate) fn path_match_key(path: &Path) -> PathMatchKey {
    path.as_os_str()
        .encode_wide()
        .map(|unit| match unit {
            0x41..=0x5a => unit + 0x20,
            _ => unit,
        })
        .collect()
}

#[cfg(not(windows))]
pub(crate) fn path_match_set_contains(paths: &BTreeSet<PathMatchKey>, path: &Path) -> bool {
    paths.contains(path)
}

#[cfg(windows)]
pub(crate) fn path_match_set_contains(paths: &BTreeSet<PathMatchKey>, path: &Path) -> bool {
    paths.contains(&path_match_key(path))
}

#[cfg(not(windows))]
pub(crate) fn path_match_map_get<'a, V>(
    paths: &'a BTreeMap<PathMatchKey, V>,
    path: &Path,
) -> Option<&'a V> {
    paths.get(path)
}

#[cfg(windows)]
pub(crate) fn path_match_map_get<'a, V>(
    paths: &'a BTreeMap<PathMatchKey, V>,
    path: &Path,
) -> Option<&'a V> {
    paths.get(&path_match_key(path))
}

pub(crate) fn path_buf_map_get<'a, V>(
    paths: &'a BTreeMap<PathBuf, V>,
    path: &Path,
) -> Option<&'a V> {
    #[cfg(not(windows))]
    {
        paths.get(path)
    }
    #[cfg(windows)]
    {
        paths
            .iter()
            .find_map(|(candidate, value)| paths_match(candidate, path).then_some(value))
    }
}

pub(crate) fn path_buf_map_insert<V>(
    paths: &mut BTreeMap<PathBuf, V>,
    path: PathBuf,
    value: V,
) -> Option<V> {
    #[cfg(not(windows))]
    {
        paths.insert(path, value)
    }
    #[cfg(windows)]
    {
        let previous_key = paths
            .keys()
            .find(|candidate| paths_match(candidate, &path))
            .cloned();
        let previous = previous_key.and_then(|previous_key| paths.remove(&previous_key));
        paths.insert(path, value);
        previous
    }
}

pub(crate) fn path_buf_map_remove<V>(paths: &mut BTreeMap<PathBuf, V>, path: &Path) -> Option<V> {
    #[cfg(not(windows))]
    {
        paths.remove(path)
    }
    #[cfg(windows)]
    {
        let key = paths
            .keys()
            .find(|candidate| paths_match(candidate, path))
            .cloned()?;
        paths.remove(&key)
    }
}

pub(crate) fn deduplicate_paths(paths: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    let mut seen = BTreeSet::new();
    paths
        .into_iter()
        .filter(|path| seen.insert(path_match_key(path)))
        .collect()
}

pub(crate) fn paths_match(left: &Path, right: &Path) -> bool {
    #[cfg(not(windows))]
    {
        left == right
    }
    #[cfg(windows)]
    {
        path_match_key(left) == path_match_key(right)
    }
}

/// Encode a path into a persistence-safe string while leaving ordinary Unicode paths unchanged.
pub(crate) fn encode_persisted_path(path: &Path) -> String {
    if let Some(path) = path.to_str()
        && !path.starts_with(ENCODED_PATH_PREFIX)
    {
        return path.to_owned();
    }

    #[cfg(unix)]
    let (platform, bytes) = ("u:", path.as_os_str().as_bytes().to_vec());
    #[cfg(windows)]
    let (platform, bytes) = (
        "w:",
        path.as_os_str()
            .encode_wide()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>(),
    );
    #[cfg(not(any(unix, windows)))]
    let (platform, bytes) = ("o:", path.to_string_lossy().as_bytes().to_vec());

    let mut encoded = String::with_capacity(ENCODED_PATH_PREFIX.len() + 2 + bytes.len() * 2);
    encoded.push_str(ENCODED_PATH_PREFIX);
    encoded.push_str(platform);
    push_hex(&mut encoded, &bytes);
    encoded
}

/// Decode a path written by [`encode_persisted_path`], accepting legacy plain Unicode strings.
pub(crate) fn decode_persisted_path(stored: &str) -> PathBuf {
    #[cfg(unix)]
    if let Some(encoded) = stored
        .strip_prefix(ENCODED_PATH_PREFIX)
        .and_then(|encoded| encoded.strip_prefix("u:"))
        && let Some(bytes) = decode_hex(encoded)
    {
        return PathBuf::from(std::ffi::OsString::from_vec(bytes));
    }
    #[cfg(windows)]
    if let Some(encoded) = stored
        .strip_prefix(ENCODED_PATH_PREFIX)
        .and_then(|encoded| encoded.strip_prefix("w:"))
        && let Some(bytes) = decode_hex(encoded)
    {
        let (units, remainder) = bytes.as_slice().as_chunks::<2>();
        if remainder.is_empty() {
            let wide = units
                .iter()
                .map(|bytes| u16::from_le_bytes(*bytes))
                .collect::<Vec<_>>();
            return PathBuf::from(std::ffi::OsString::from_wide(&wide));
        }
    }
    PathBuf::from(stored)
}

fn push_hex(destination: &mut String, bytes: &[u8]) {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    for byte in bytes {
        destination.push(DIGITS[usize::from(byte >> 4)] as char);
        destination.push(DIGITS[usize::from(byte & 0x0f)] as char);
    }
}

fn decode_hex(encoded: &str) -> Option<Vec<u8>> {
    if !encoded.len().is_multiple_of(2) {
        return None;
    }
    let (pairs, remainder) = encoded.as_bytes().as_chunks::<2>();
    debug_assert!(remainder.is_empty());
    pairs
        .iter()
        .map(|digits| {
            let high = (digits[0] as char).to_digit(16)?;
            let low = (digits[1] as char).to_digit(16)?;
            Some(((high << 4) | low) as u8)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    use std::ffi::OsString;

    #[cfg(windows)]
    use std::{ffi::OsString, os::windows::ffi::OsStringExt as _};

    #[test]
    fn indexes_and_direct_matching_share_platform_path_identity() {
        let stored = Path::new("logs/a.log");
        let key = path_match_key(stored);
        let set = BTreeSet::from([key.clone()]);
        let map = BTreeMap::from([(key, 7)]);

        assert!(path_match_set_contains(&set, stored));
        assert_eq!(path_match_map_get(&map, stored), Some(&7));
        assert!(paths_match(stored, stored));

        let differently_cased = Path::new("LOGS/A.LOG");
        assert_eq!(
            path_match_set_contains(&set, differently_cased),
            cfg!(windows)
        );
        assert_eq!(paths_match(stored, differently_cased), cfg!(windows));
    }

    #[test]
    fn path_buf_maps_and_batch_deduplication_share_platform_identity() {
        let stored = PathBuf::from("logs/a.log");
        let differently_cased = PathBuf::from("LOGS/A.LOG");
        let mut map = BTreeMap::from([(stored.clone(), 7)]);

        assert_eq!(
            path_buf_map_get(&map, &differently_cased),
            cfg!(windows).then_some(&7)
        );
        assert_eq!(
            deduplicate_paths([stored, differently_cased]).len(),
            if cfg!(windows) { 1 } else { 2 }
        );

        let replacement = PathBuf::from("LOGS/A.LOG");
        assert_eq!(
            path_buf_map_insert(&mut map, replacement.clone(), 8),
            cfg!(windows).then_some(7)
        );
        assert_eq!(
            path_buf_map_remove(&mut map, Path::new("logs/a.log")),
            if cfg!(windows) { Some(8) } else { Some(7) }
        );
        assert_eq!(
            path_buf_map_get(&map, &replacement),
            if cfg!(windows) { None } else { Some(&8) }
        );
    }

    #[cfg(unix)]
    #[test]
    fn persisted_paths_preserve_non_utf8_identity() {
        let first = PathBuf::from(OsString::from_vec(b"source-\x80.log".to_vec()));
        let second = PathBuf::from(OsString::from_vec(b"source-\x81.log".to_vec()));

        assert_eq!(first.to_string_lossy(), second.to_string_lossy());
        assert_ne!(
            encode_persisted_path(&first),
            encode_persisted_path(&second)
        );
        assert_eq!(decode_persisted_path(&encode_persisted_path(&first)), first);
        assert_eq!(
            decode_persisted_path(&encode_persisted_path(&second)),
            second
        );
    }

    #[cfg(windows)]
    #[test]
    fn runtime_keys_preserve_unpaired_utf16_identity() {
        let first = PathBuf::from(OsString::from_wide(&[
            b'L' as u16,
            b'O' as u16,
            b'G' as u16,
            0xd800,
        ]));
        let same_ascii_case_fold = PathBuf::from(OsString::from_wide(&[
            b'l' as u16,
            b'o' as u16,
            b'g' as u16,
            0xd800,
        ]));
        let distinct_surrogate = PathBuf::from(OsString::from_wide(&[
            b'l' as u16,
            b'o' as u16,
            b'g' as u16,
            0xd801,
        ]));

        assert_eq!(
            path_match_key(&first),
            path_match_key(&same_ascii_case_fold)
        );
        assert_ne!(path_match_key(&first), path_match_key(&distinct_surrogate));
    }
}
