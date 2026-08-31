use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

#[cfg(not(windows))]
use std::path::PathBuf;

#[cfg(not(windows))]
pub(crate) type PathMatchKey = PathBuf;
#[cfg(windows)]
pub(crate) type PathMatchKey = String;

#[cfg(not(windows))]
pub(crate) fn path_match_key(path: &Path) -> PathMatchKey {
    path.to_path_buf()
}

#[cfg(windows)]
pub(crate) fn path_match_key(path: &Path) -> PathMatchKey {
    path.to_string_lossy().to_ascii_lowercase()
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
