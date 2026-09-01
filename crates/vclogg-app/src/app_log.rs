use std::{
    collections::VecDeque,
    fs,
    path::Path,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicU8, Ordering},
    },
};

use anyhow::Context as _;
use chrono::Local;
use log::{Level, LevelFilter, Log, Metadata, Record};

const MAX_LOG_ENTRIES: usize = 20_000;
const MAX_LOG_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AppLogLevel {
    Off,
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl AppLogLevel {
    pub(crate) const ALL: [Self; 6] = [
        Self::Off,
        Self::Error,
        Self::Warn,
        Self::Info,
        Self::Debug,
        Self::Trace,
    ];

    pub(crate) const fn database_value(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Error => "error",
            Self::Warn => "warn",
            Self::Info => "info",
            Self::Debug => "debug",
            Self::Trace => "trace",
        }
    }

    pub(crate) fn from_database(value: &str) -> Self {
        match value {
            "off" => Self::Off,
            "warn" => Self::Warn,
            "info" => Self::Info,
            "debug" => Self::Debug,
            "trace" => Self::Trace,
            _ => Self::Error,
        }
    }

    pub(crate) fn select_index(self) -> usize {
        Self::ALL
            .iter()
            .position(|candidate| *candidate == self)
            .unwrap_or(1)
    }

    const fn as_u8(self) -> u8 {
        match self {
            Self::Off => 0,
            Self::Error => 1,
            Self::Warn => 2,
            Self::Info => 3,
            Self::Debug => 4,
            Self::Trace => 5,
        }
    }

    fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::Off,
            1 => Self::Error,
            2 => Self::Warn,
            3 => Self::Info,
            4 => Self::Debug,
            _ => Self::Trace,
        }
    }

    fn filter(self) -> LevelFilter {
        match self {
            Self::Off => LevelFilter::Off,
            Self::Error => LevelFilter::Error,
            Self::Warn => LevelFilter::Warn,
            Self::Info => LevelFilter::Info,
            Self::Debug => LevelFilter::Debug,
            Self::Trace => LevelFilter::Trace,
        }
    }

    fn accepts(self, level: Level) -> bool {
        match self {
            Self::Off => false,
            Self::Error => level <= Level::Error,
            Self::Warn => level <= Level::Warn,
            Self::Info => level <= Level::Info,
            Self::Debug => level <= Level::Debug,
            Self::Trace => true,
        }
    }
}

impl Default for AppLogLevel {
    fn default() -> Self {
        if cfg!(debug_assertions) {
            Self::Debug
        } else {
            Self::Error
        }
    }
}

#[derive(Default)]
struct LogBuffer {
    entries: VecDeque<String>,
    byte_size: usize,
}

impl LogBuffer {
    fn push(&mut self, line: String) {
        self.byte_size = self.byte_size.saturating_add(line.len() + 1);
        self.entries.push_back(line);
        while self.entries.len() > MAX_LOG_ENTRIES || self.byte_size > MAX_LOG_BYTES {
            let Some(removed) = self.entries.pop_front() else {
                break;
            };
            self.byte_size = self.byte_size.saturating_sub(removed.len() + 1);
        }
    }

    fn export_text(&self) -> String {
        let mut output = String::with_capacity(self.byte_size.saturating_add(256));
        output.push_str("VCLogg2 application log\n");
        output.push_str(&format!(
            "version={} profile={} target={} commit={}\n",
            crate::build_info::VERSION,
            env!("VCLOGG2_BUILD_PROFILE"),
            env!("VCLOGG2_BUILD_TARGET"),
            env!("VCLOGG2_BUILD_COMMIT"),
        ));
        output.push_str(&format!("exported_at={}\n\n", Local::now().to_rfc3339()));
        for entry in &self.entries {
            output.push_str(entry);
            output.push('\n');
        }
        output
    }
}

struct AppLogger {
    level: AtomicU8,
    buffer: Mutex<LogBuffer>,
}

impl AppLogger {
    fn new(level: AppLogLevel) -> Self {
        Self {
            level: AtomicU8::new(level.as_u8()),
            buffer: Mutex::new(LogBuffer::default()),
        }
    }

    fn level(&self) -> AppLogLevel {
        AppLogLevel::from_u8(self.level.load(Ordering::Relaxed))
    }

    fn set_level(&self, level: AppLogLevel) {
        self.level.store(level.as_u8(), Ordering::Relaxed);
        log::set_max_level(level.filter());
    }
}

impl Log for AppLogger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        self.level().accepts(metadata.level())
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let line = format!(
            "{} {:<5} [{}] {}",
            Local::now().format("%Y-%m-%d %H:%M:%S%.3f%:z"),
            record.level(),
            record.target(),
            record.args(),
        );
        eprintln!("{line}");
        if let Ok(mut buffer) = self.buffer.lock() {
            buffer.push(line);
        }
    }

    fn flush(&self) {}
}

static LOGGER: OnceLock<AppLogger> = OnceLock::new();

pub(crate) fn init() {
    let logger = LOGGER.get_or_init(|| AppLogger::new(AppLogLevel::default()));
    if log::set_logger(logger).is_ok() {
        log::set_max_level(logger.level().filter());
    }
}

pub(crate) fn set_level(level: AppLogLevel) {
    if let Some(logger) = LOGGER.get() {
        logger.set_level(level);
    }
}

pub(crate) fn entry_count() -> usize {
    LOGGER
        .get()
        .and_then(|logger| logger.buffer.lock().ok().map(|buffer| buffer.entries.len()))
        .unwrap_or_default()
}

pub(crate) fn export(path: &Path) -> anyhow::Result<usize> {
    let logger = LOGGER
        .get()
        .context("application logger is not initialized")?;
    let (text, count) = {
        let buffer = logger
            .buffer
            .lock()
            .map_err(|_| anyhow::anyhow!("application log buffer is unavailable"))?;
        (buffer.export_text(), buffer.entries.len())
    };
    fs::write(path, text)
        .with_context(|| format!("couldn't write application log to {}", path.display()))?;
    Ok(count)
}
