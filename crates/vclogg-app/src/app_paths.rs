use std::path::PathBuf;

#[cfg(any(windows, test))]
use std::path::Path;

const APPLICATION_DIRECTORY: &str = "VCLogg2";

#[cfg(all(debug_assertions, not(windows)))]
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
    #[cfg(windows)]
    {
        executable_directory()
    }
    #[cfg(not(windows))]
    {
        development_root()
            .map(|root| root.join("data"))
            .or_else(dirs::data_local_dir)
    }
}

pub(crate) fn cache_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        executable_directory()
    }
    #[cfg(not(windows))]
    {
        development_root()
            .map(|root| root.join("cache"))
            .or_else(dirs::cache_dir)
    }
}

pub(crate) fn application_data_dir() -> Option<PathBuf> {
    data_local_dir().map(application_data_dir_from_root)
}

pub(crate) fn index_cache_dir() -> Option<PathBuf> {
    cache_dir().map(|root| application_data_dir_from_root(root).join("index"))
}

pub(crate) fn temporary_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        application_data_dir().map(|root| root.join("temp"))
    }
    #[cfg(not(windows))]
    {
        Some(std::env::temp_dir())
    }
}

#[cfg(all(debug_assertions, not(windows)))]
fn development_root() -> Option<PathBuf> {
    std::env::var_os(DEVELOPMENT_DATA_DIRECTORY_ENV)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

#[cfg(any(not(debug_assertions), windows))]
fn development_root() -> Option<PathBuf> {
    None
}

#[cfg(windows)]
fn executable_directory() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .as_deref()
        .and_then(executable_parent)
}

#[cfg(any(windows, test))]
fn executable_parent(executable: &Path) -> Option<PathBuf> {
    executable.parent().map(Path::to_path_buf)
}

fn application_data_dir_from_root(root: PathBuf) -> PathBuf {
    root.join(APPLICATION_DIRECTORY)
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{application_data_dir_from_root, executable_parent};

    #[test]
    fn portable_windows_root_is_the_executable_parent() {
        let executable = Path::new("/portable/vclogg2.exe");
        let root = executable_parent(executable).expect("可执行文件应有父目录");

        assert_eq!(root, PathBuf::from("/portable"));
        assert_eq!(
            application_data_dir_from_root(root).join("temp"),
            PathBuf::from("/portable/VCLogg2/temp")
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_internal_paths_stay_below_the_running_executable() {
        use super::{
            application_data_dir, cache_dir, data_local_dir, index_cache_dir, temporary_dir,
        };

        let executable = std::env::current_exe().expect("应能读取测试可执行文件路径");
        let executable_dir = executable.parent().expect("测试可执行文件应有父目录");
        let application_dir = executable_dir.join("VCLogg2");

        assert_eq!(data_local_dir().as_deref(), Some(executable_dir));
        assert_eq!(cache_dir().as_deref(), Some(executable_dir));
        assert_eq!(application_data_dir(), Some(application_dir.clone()));
        assert_eq!(index_cache_dir(), Some(application_dir.join("index")));
        assert_eq!(temporary_dir(), Some(application_dir.join("temp")));
    }
}
