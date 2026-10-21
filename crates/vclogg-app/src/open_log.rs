//! INFO diagnostics for external opens. Disk I/O stays on a dedicated worker.
use std::{
    fs::{self, File},
    io::{Read as _, Write as _},
    path::PathBuf,
    sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

pub(crate) const TARGET: &str = "vclogg2::open";
const FILE_LIMIT: u64 = 8 * 1024 * 1024;
const FILE_COUNT: usize = 64;
static SEQUENCE: AtomicU64 = AtomicU64::new(1);
static LOST: AtomicU64 = AtomicU64::new(0);
static WRITER: OnceLock<Option<mpsc::SyncSender<Message>>> = OnceLock::new();

enum Message {
    Line(String),
    Flush(mpsc::Sender<()>),
}

pub(crate) struct Stage {
    id: u64,
    name: &'static str,
    started: Instant,
}

impl Stage {
    pub(crate) fn new(name: &'static str, details: impl std::fmt::Display) -> Self {
        let stage = Self {
            id: SEQUENCE.fetch_add(1, Ordering::Relaxed),
            name,
            started: Instant::now(),
        };
        stage.event(format_args!("begin {details}"));
        stage
    }

    pub(crate) fn id(&self) -> u64 {
        self.id
    }

    pub(crate) fn event(&self, details: impl std::fmt::Display) {
        log::info!(target: TARGET, "pid={} thread={:?} stage={}-{} name={} elapsed_ms={:.3} {}",
            std::process::id(), std::thread::current().id(), std::process::id(), self.id,
            self.name, self.started.elapsed().as_secs_f64() * 1000.0, details);
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        // Scope exit may mean failure/cancellation; success is logged explicitly.
        self.event("scope_exit");
    }
}

pub(crate) struct FlushOnExit;
impl Drop for FlushOnExit {
    fn drop(&mut self) {
        flush();
    }
}

fn directory() -> Option<PathBuf> {
    crate::app_paths::application_data_dir().map(|root| root.join("logs").join("open"))
}

fn files(directory: &std::path::Path) -> Vec<PathBuf> {
    let mut files = fs::read_dir(directory)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry.file_name().to_string_lossy().starts_with("open-")
                && entry.path().extension().is_some_and(|ext| ext == "log")
        })
        .filter_map(|entry| Some((entry.metadata().ok()?.modified().ok()?, entry.path())))
        .collect::<Vec<_>>();
    files.sort_by(|a, b| b.0.cmp(&a.0));
    files.into_iter().map(|(_, path)| path).collect()
}

fn create_file() -> std::io::Result<(File, PathBuf)> {
    let directory =
        directory().ok_or_else(|| std::io::Error::other("data directory unavailable"))?;
    fs::create_dir_all(&directory)?;
    for path in files(&directory).into_iter().skip(FILE_COUNT - 1) {
        let _ = fs::remove_file(path);
    }
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%S%.9f");
    let path = directory.join(format!("open-{stamp}-{}.log", std::process::id()));
    let mut file = File::create(&path)?;
    writeln!(
        file,
        "version={} commit={} pid={}",
        crate::build_info::VERSION,
        env!("VCLOGG2_BUILD_COMMIT"),
        std::process::id()
    )?;
    Ok((file, path))
}

fn run(receiver: mpsc::Receiver<Message>) {
    let file = create_file();
    let mut file = match file {
        Ok(file) => Some(file),
        Err(error) => {
            eprintln!("Open diagnostic log unavailable: {error}");
            None
        }
    };
    let mut bytes = 0u64;
    for message in receiver {
        match message {
            Message::Line(line) => {
                // Another process may prune an idle primary's file. Reopen before
                // writing so subsequent diagnostics never go to an unlinked file.
                if bytes + line.len() as u64 + 1 > FILE_LIMIT
                    || file.as_ref().is_some_and(|(_, path)| !path.exists())
                {
                    file = create_file().ok();
                    bytes = 0;
                }
                if file.is_none() {
                    LOST.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
                if let Some((output, _)) = &mut file {
                    if writeln!(output, "{line}").is_err() {
                        LOST.fetch_add(1, Ordering::Relaxed);
                        file = None;
                    } else {
                        bytes += line.len() as u64 + 1;
                    }
                }
            }
            Message::Flush(done) => {
                if let Some((file, _)) = &mut file {
                    let _ = writeln!(
                        file,
                        "flush dropped_records={}",
                        LOST.load(Ordering::Relaxed)
                    );
                    let _ = file.flush();
                }
                let _ = done.send(());
            }
        }
    }
}

pub(crate) fn persist(line: &str) {
    let sender = WRITER.get_or_init(|| {
        let (sender, receiver) = mpsc::sync_channel(2048);
        std::thread::Builder::new()
            .name("open-diagnostic-log".into())
            .spawn(move || run(receiver))
            .ok()
            .map(|_| sender)
    });
    if sender
        .as_ref()
        .is_none_or(|sender| sender.try_send(Message::Line(line.to_owned())).is_err())
    {
        LOST.fetch_add(1, Ordering::Relaxed);
    }
}

pub(crate) fn flush() {
    let Some(Some(sender)) = WRITER.get() else {
        return;
    };
    let (done, receiver) = mpsc::channel();
    if sender.try_send(Message::Flush(done)).is_ok() {
        let _ = receiver.recv_timeout(Duration::from_secs(1));
    }
}

/// Called by the existing background export task, never from render or activation.
pub(crate) fn append_export(text: &mut String) {
    flush();
    text.push_str(&format!(
        "\nOpen diagnostics (current-process dropped records={}):\n",
        LOST.load(Ordering::Relaxed)
    ));
    let Some(directory) = directory() else {
        text.push_str("Data directory unavailable\n");
        return;
    };
    let mut remaining = 16 * 1024 * 1024;
    for path in files(&directory).into_iter().take(FILE_COUNT) {
        if remaining == 0 {
            text.push_str("Diagnostic export size limit reached\n");
            break;
        }
        text.push_str(&format!("\n--- {} ---\n", path.display()));
        match File::open(&path) {
            Ok(file) => {
                let mut bytes = Vec::new();
                match file.take(remaining as u64).read_to_end(&mut bytes) {
                    Ok(_) => {
                        remaining -= bytes.len();
                        text.push_str(&String::from_utf8_lossy(&bytes));
                    }
                    Err(error) => text.push_str(&format!("Read failed: {error}\n")),
                }
            }
            Err(error) => text.push_str(&format!("Open failed: {error}\n")),
        }
    }
}
