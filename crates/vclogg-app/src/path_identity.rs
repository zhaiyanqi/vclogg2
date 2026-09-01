use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

#[cfg(windows)]
use std::os::windows::ffi::OsStrExt as _;

pub(crate) use vclogg_data::{decode_persisted_path, encode_persisted_path};

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

/// Produces a stable directory-session identity without touching the filesystem. Dialogs usually
/// return canonical absolute paths, but persisted state may still contain `.` or `..` components.
pub(crate) fn normalized_path_match_key(path: &Path) -> PathMatchKey {
    use std::path::Component;

    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                let can_pop = normalized
                    .components()
                    .next_back()
                    .is_some_and(|last| matches!(last, Component::Normal(_)));
                if can_pop {
                    normalized.pop();
                } else {
                    normalized.push(component.as_os_str());
                }
            }
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }
    path_match_key(&normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn directory_session_keys_normalize_lexical_components() {
        assert_eq!(
            normalized_path_match_key(Path::new("logs/./archive/../today")),
            normalized_path_match_key(Path::new("logs/today"))
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
