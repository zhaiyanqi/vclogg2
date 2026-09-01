use std::{
    collections::BTreeSet,
    ffi::OsStr,
    fs::{self, File, OpenOptions},
    io::{self, Write as _},
    os::windows::{ffi::OsStrExt as _, process::CommandExt as _},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use anyhow::{Context as _, ensure};
use uuid::Uuid;
use windows_sys::Win32::{
    Foundation::{CloseHandle, ERROR_INVALID_PARAMETER, WAIT_FAILED},
    Storage::FileSystem::{MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW},
    System::Threading::{
        CREATE_NO_WINDOW, INFINITE, OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
    },
};
use zip::ZipArchive;

const HELPER_ARGUMENT: &str = "--vclogg2-native-update-helper";
const CLEANUP_PATH_ENV: &str = "VCLOGG2_UPDATE_HELPER_PATH";
const CLEANUP_PID_ENV: &str = "VCLOGG2_UPDATE_HELPER_PID";
const HELPER_DIRECTORY_NAME: &str = "vclogg2-update-helpers";
const HELPER_FILE_PREFIX: &str = "vclogg2-update-helper-";
const EXPECTED_ARCHIVE_ENTRIES: [&str; 3] = ["LICENSE", "README.md", "vclogg2.exe"];
const MAX_EXECUTABLE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_DOCUMENT_BYTES: u64 = 16 * 1024 * 1024;

pub(crate) fn run_if_requested() -> bool {
    let mut arguments = std::env::args_os();
    _ = arguments.next();
    if arguments.next().as_deref() != Some(OsStr::new(HELPER_ARGUMENT)) {
        return false;
    }

    let result = (|| {
        let archive_path = next_path(&mut arguments, "更新包路径")?;
        let install_directory = next_path(&mut arguments, "安装目录")?;
        let wait_for_process_id = arguments
            .next()
            .context("缺少待退出进程 ID")?
            .to_string_lossy()
            .parse::<u32>()
            .context("待退出进程 ID 无效")?;
        ensure!(arguments.next().is_none(), "更新助手收到了多余参数");
        apply_update(&archive_path, &install_directory, wait_for_process_id)
    })();
    if let Err(error) = result {
        eprintln!("VCLogg2 原生更新助手失败：{error:#}");
    }
    true
}

pub(crate) fn finish_previous_update() {
    let Some(helper_path) = std::env::var_os(CLEANUP_PATH_ENV).map(PathBuf::from) else {
        return;
    };
    let result = (|| {
        let helper_directory = std::env::temp_dir().join(HELPER_DIRECTORY_NAME);
        let helper_name = helper_path
            .file_name()
            .and_then(OsStr::to_str)
            .context("更新助手清理路径没有有效文件名")?;
        ensure!(
            helper_path.parent() == Some(helper_directory.as_path())
                && helper_name.starts_with(HELPER_FILE_PREFIX)
                && helper_name.ends_with(".exe"),
            "拒绝清理无效的更新助手路径：{}",
            helper_path.display()
        );
        let helper_process_id = std::env::var(CLEANUP_PID_ENV)
            .context("缺少更新助手进程 ID")?
            .parse::<u32>()
            .context("更新助手进程 ID 无效")?;
        wait_for_process(helper_process_id)?;
        if helper_path.is_file() {
            fs::remove_file(&helper_path)
                .with_context(|| format!("无法清理更新助手：{}", helper_path.display()))?;
        }
        let log_path = helper_path.with_extension("log");
        if log_path.is_file() {
            fs::remove_file(&log_path)
                .with_context(|| format!("无法清理更新日志：{}", log_path.display()))?;
        }
        Ok::<_, anyhow::Error>(())
    })();
    if let Err(error) = result {
        eprintln!("VCLogg2 更新助手清理失败：{error:#}");
    }
}

pub(crate) fn launch(
    current_executable: &Path,
    archive_path: &Path,
    install_directory: &Path,
) -> anyhow::Result<()> {
    ensure!(
        archive_path.is_file(),
        "更新包不存在：{}",
        archive_path.display()
    );
    let helper_directory = std::env::temp_dir().join(HELPER_DIRECTORY_NAME);
    fs::create_dir_all(&helper_directory)
        .with_context(|| format!("无法创建更新助手目录：{}", helper_directory.display()))?;
    cleanup_stale_helpers(&helper_directory);

    let helper_path = helper_directory.join(format!(
        "{HELPER_FILE_PREFIX}{}.exe",
        Uuid::new_v4().simple()
    ));
    copy_new_file(current_executable, &helper_path)
        .with_context(|| format!("无法创建原生更新助手：{}", helper_path.display()))?;

    let log_path = helper_path.with_extension("log");
    let log = File::create(&log_path)
        .with_context(|| format!("无法创建更新日志：{}", log_path.display()))?;
    let error_log = log.try_clone()?;
    let spawn_result = Command::new(&helper_path)
        .arg(HELPER_ARGUMENT)
        .arg(archive_path)
        .arg(install_directory)
        .arg(std::process::id().to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(error_log))
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
    if let Err(error) = spawn_result {
        _ = fs::remove_file(&helper_path);
        _ = fs::remove_file(&log_path);
        return Err(error).context("无法启动原生更新助手");
    }
    Ok(())
}

fn next_path(
    arguments: &mut impl Iterator<Item = std::ffi::OsString>,
    description: &str,
) -> anyhow::Result<PathBuf> {
    arguments
        .next()
        .map(PathBuf::from)
        .with_context(|| format!("缺少{description}"))
}

fn apply_update(
    archive_path: &Path,
    install_directory: &Path,
    wait_for_process_id: u32,
) -> anyhow::Result<()> {
    ensure!(
        archive_path.is_file(),
        "更新包不存在：{}",
        archive_path.display()
    );
    wait_for_process(wait_for_process_id).context("等待 VCLogg2 退出失败")?;

    let staging = TemporaryDirectory::create()?;
    extract_release(archive_path, staging.path())?;
    fs::create_dir_all(install_directory)
        .with_context(|| format!("无法创建安装目录：{}", install_directory.display()))?;

    for name in ["LICENSE", "README.md", "vclogg2.exe"] {
        install_file(&staging.path().join(name), &install_directory.join(name))?;
    }

    let installed_executable = install_directory.join("vclogg2.exe");
    let helper_path = std::env::current_exe().context("无法确定更新助手路径")?;
    Command::new(&installed_executable)
        .env(CLEANUP_PATH_ENV, &helper_path)
        .env(CLEANUP_PID_ENV, std::process::id().to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .with_context(|| format!("无法重新启动 VCLogg2：{}", installed_executable.display()))?;
    Ok(())
}

fn wait_for_process(process_id: u32) -> anyhow::Result<()> {
    // SAFETY: OpenProcess receives a process ID provided by the already-running
    // parent/helper. The returned owned handle is closed on every path below.
    let process = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, process_id) };
    if process.is_null() {
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(ERROR_INVALID_PARAMETER as i32) {
            return Ok(());
        }
        return Err(error).context("无法打开待退出进程");
    }
    // SAFETY: `process` is a valid synchronization handle and remains open for
    // the duration of this blocking wait.
    let wait_result = unsafe { WaitForSingleObject(process, INFINITE) };
    let wait_error = (wait_result == WAIT_FAILED).then(io::Error::last_os_error);
    // SAFETY: `process` is owned by this function and is closed exactly once.
    unsafe { CloseHandle(process) };
    if let Some(error) = wait_error {
        return Err(error).context("等待进程退出失败");
    }
    Ok(())
}

fn extract_release(archive_path: &Path, staging: &Path) -> anyhow::Result<()> {
    let archive_file = File::open(archive_path)
        .with_context(|| format!("无法打开更新包：{}", archive_path.display()))?;
    let mut archive = ZipArchive::new(archive_file).context("更新包不是有效的 ZIP 文件")?;
    ensure!(
        archive.len() == EXPECTED_ARCHIVE_ENTRIES.len(),
        "更新包必须且只能包含 LICENSE、README.md 与 vclogg2.exe"
    );

    let mut names = BTreeSet::new();
    for index in 0..archive.len() {
        let entry = archive.by_index(index).context("无法读取更新包目录")?;
        ensure!(!entry.is_dir(), "更新包不应包含目录：{}", entry.name());
        ensure!(
            names.insert(entry.name().to_string()),
            "更新包包含重复文件：{}",
            entry.name()
        );
    }
    let expected = EXPECTED_ARCHIVE_ENTRIES
        .into_iter()
        .map(str::to_string)
        .collect::<BTreeSet<_>>();
    ensure!(
        names == expected,
        "更新包必须且只能包含 LICENSE、README.md 与 vclogg2.exe"
    );

    for name in EXPECTED_ARCHIVE_ENTRIES {
        let mut entry = archive
            .by_name(name)
            .with_context(|| format!("更新包缺少 {name}"))?;
        let maximum_size = if name == "vclogg2.exe" {
            MAX_EXECUTABLE_BYTES
        } else {
            MAX_DOCUMENT_BYTES
        };
        ensure!(
            entry.size() <= maximum_size,
            "更新包中的 {name} 超过大小限制"
        );
        let destination = staging.join(name);
        let mut output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&destination)
            .with_context(|| format!("无法创建暂存文件：{}", destination.display()))?;
        let copied =
            io::copy(&mut entry, &mut output).with_context(|| format!("无法解压 {name}"))?;
        ensure!(copied == entry.size(), "更新包中的 {name} 长度无效");
        output.flush()?;
        output.sync_all()?;
    }
    Ok(())
}

fn install_file(source: &Path, target: &Path) -> anyhow::Result<()> {
    let file_name = target
        .file_name()
        .and_then(OsStr::to_str)
        .context("安装目标文件名无效")?;
    let replacement = target
        .parent()
        .context("安装目标没有父目录")?
        .join(format!(".{file_name}.update-{}", Uuid::new_v4().simple()));
    copy_new_file(source, &replacement)?;

    let replacement_wide = wide_path(&replacement);
    let target_wide = wide_path(target);
    // SAFETY: Both paths are owned, NUL-terminated UTF-16 buffers and point to
    // files in the same directory. MOVEFILE_REPLACE_EXISTING keeps the previous
    // target intact if the replacement cannot be committed.
    let moved = unsafe {
        MoveFileExW(
            replacement_wide.as_ptr(),
            target_wide.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        let error = io::Error::last_os_error();
        _ = fs::remove_file(&replacement);
        return Err(error).with_context(|| format!("无法替换 {}", target.display()));
    }
    Ok(())
}

fn copy_new_file(source: &Path, destination: &Path) -> io::Result<()> {
    let mut input = File::open(source)?;
    let mut output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(destination)?;
    io::copy(&mut input, &mut output)?;
    output.flush()?;
    output.sync_all()
}

fn wide_path(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn cleanup_stale_helpers(directory: &Path) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(OsStr::to_str) else {
            continue;
        };
        if path.is_file()
            && name.starts_with(HELPER_FILE_PREFIX)
            && (name.ends_with(".exe") || name.ends_with(".log"))
        {
            _ = fs::remove_file(path);
        }
    }
}

struct TemporaryDirectory(PathBuf);

impl TemporaryDirectory {
    fn create() -> anyhow::Result<Self> {
        let path = std::env::temp_dir().join(format!("vclogg2-update-{}", Uuid::new_v4().simple()));
        fs::create_dir(&path)
            .with_context(|| format!("无法创建更新暂存目录：{}", path.display()))?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        _ = fs::remove_dir_all(&self.0);
    }
}
