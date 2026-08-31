use std::{
    backtrace::Backtrace,
    cell::Cell,
    fs::{self, File, OpenOptions},
    io::{self, Write as _},
    panic::{self, PanicHookInfo},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_REPORTS: usize = 20;
const REPORT_FILE_ATTEMPTS: u64 = 32;

static REPORT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

thread_local! {
    static WRITING_REPORT: Cell<bool> = const { Cell::new(false) };
}

pub(crate) fn install_panic_hook() {
    let report_directory = prepare_report_directory();
    prune_old_reports(&report_directory);

    let previous_hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        let should_report = WRITING_REPORT.with(|writing| !writing.replace(true));
        if !should_report {
            return;
        }

        if let Err(error) = write_panic_report(&report_directory, info) {
            log::error!("VCLogg2 panic 报告未能写入：{error}");
        }
        previous_hook(info);
        WRITING_REPORT.with(|writing| writing.set(false));
    }));
}

fn prepare_report_directory() -> PathBuf {
    let preferred = crate::app_paths::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("VCLogg2")
        .join("crashes");
    if fs::create_dir_all(&preferred).is_ok() {
        return preferred;
    }

    let fallback = std::env::temp_dir().join("VCLogg2").join("crashes");
    let _ = fs::create_dir_all(&fallback);
    fallback
}

fn write_panic_report(directory: &Path, info: &PanicHookInfo<'_>) -> io::Result<PathBuf> {
    fs::create_dir_all(directory)?;
    let timestamp_millis = unix_timestamp_millis();
    let (mut file, path) = create_report_file(directory, timestamp_millis)?;
    let current_thread = thread::current();
    let thread_name = current_thread.name().unwrap_or("<unnamed>");
    let message = panic_message(info);

    writeln!(file, "VCLogg2 panic report")?;
    writeln!(file, "version: {}", env!("CARGO_PKG_VERSION"))?;
    writeln!(file, "os: {}", std::env::consts::OS)?;
    writeln!(file, "architecture: {}", std::env::consts::ARCH)?;
    writeln!(file, "timestamp_unix_ms: {timestamp_millis}")?;
    writeln!(file, "process_id: {}", std::process::id())?;
    writeln!(file, "thread_id: {:?}", current_thread.id())?;
    writeln!(file, "thread_name: {thread_name}")?;
    writeln!(file, "panic: {message}")?;
    if let Some(location) = info.location() {
        writeln!(
            file,
            "location: {}:{}:{}",
            location.file(),
            location.line(),
            location.column()
        )?;
    } else {
        writeln!(file, "location: <unknown>")?;
    }

    writeln!(file, "\nbacktrace:\n{}", Backtrace::force_capture())?;
    file.flush()?;
    file.sync_data()?;
    prune_old_reports(directory);
    log::error!("VCLogg2 panic 报告已写入：{}", path.display());
    Ok(path)
}

fn create_report_file(directory: &Path, timestamp_millis: u128) -> io::Result<(File, PathBuf)> {
    let process_id = std::process::id();
    let mut last_collision = None;
    for _ in 0..REPORT_FILE_ATTEMPTS {
        let sequence = REPORT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = directory.join(format!(
            "panic-{timestamp_millis}-{process_id}-{sequence}.log"
        ));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((file, path)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                last_collision = Some(error);
            }
            Err(error) => return Err(error),
        }
    }

    Err(last_collision.unwrap_or_else(|| io::Error::other("无法创建唯一的 panic 报告文件")))
}

fn panic_message(info: &PanicHookInfo<'_>) -> String {
    if let Some(message) = info.payload().downcast_ref::<&str>() {
        (*message).to_owned()
    } else if let Some(message) = info.payload().downcast_ref::<String>() {
        message.clone()
    } else {
        "<non-string panic payload>".to_owned()
    }
}

fn unix_timestamp_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn prune_old_reports(directory: &Path) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    let mut reports = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_name()?.to_str()?;
            if !name.starts_with("panic-") || !name.ends_with(".log") {
                return None;
            }
            let modified = entry.metadata().ok()?.modified().ok()?;
            Some((modified, path))
        })
        .collect::<Vec<_>>();
    reports.sort_unstable();

    let remove_count = reports.len().saturating_sub(MAX_REPORTS);
    for (_, path) in reports.into_iter().take(remove_count) {
        let _ = fs::remove_file(path);
    }
}
