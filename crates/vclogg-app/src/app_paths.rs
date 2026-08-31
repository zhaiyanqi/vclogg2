use std::path::PathBuf;

#[cfg(debug_assertions)]
const DEVELOPMENT_DATA_DIRECTORY_ENV: &str = "VCLOGG2_DEV_DATA_DIR";

pub(crate) fn log_development_override() {
    if let Some(root) = development_root() {
        log::debug!(
            "Debug 数据与缓存已隔离：root={} data={} cache={}",
            root.display(),
            root.join("data").display(),
            root.join("cache").display(),
        );
    }
}

pub(crate) fn data_local_dir() -> Option<PathBuf> {
    development_root()
        .map(|root| root.join("data"))
        .or_else(dirs::data_local_dir)
}

pub(crate) fn cache_dir() -> Option<PathBuf> {
    development_root()
        .map(|root| root.join("cache"))
        .or_else(dirs::cache_dir)
}

#[cfg(debug_assertions)]
fn development_root() -> Option<PathBuf> {
    std::env::var_os(DEVELOPMENT_DATA_DIRECTORY_ENV)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

#[cfg(not(debug_assertions))]
fn development_root() -> Option<PathBuf> {
    None
}
