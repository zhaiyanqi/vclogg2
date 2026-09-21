use std::path::PathBuf;

pub(crate) fn command() -> tokio::process::Command {
    let command = tokio::process::Command::new(executable());
    #[cfg(windows)]
    let command = {
        let mut command = command;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        command
    };
    command
}

fn executable() -> PathBuf {
    let name = if cfg!(windows) { "rg.exe" } else { "rg" };
    if let Ok(current_exe) = std::env::current_exe()
        && let Some(bundled) = bundled_executable(&current_exe, name)
    {
        return bundled;
    }
    PathBuf::from(name)
}

fn bundled_executable(current_exe: &std::path::Path, name: &str) -> Option<PathBuf> {
    let bundled = current_exe.parent()?.join(name);
    bundled.is_file().then_some(bundled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn falls_back_to_the_platform_command_name() {
        let path = executable();
        let command_name = PathBuf::from(if cfg!(windows) { "rg.exe" } else { "rg" });
        assert!(path.is_absolute() || path == command_name);
    }

    #[test]
    fn finds_a_bundled_executable_next_to_the_application() {
        let directory = tempfile::tempdir().unwrap();
        let current_exe = directory.path().join("vclogg2");
        let bundled = directory.path().join("rg");
        std::fs::write(&bundled, []).unwrap();
        assert_eq!(bundled_executable(&current_exe, "rg"), Some(bundled));
    }
}
