//! Cold-start capture begins before platform, fonts, storage, or window initialization.
use super::*;
use std::ffi::OsString;

pub(crate) struct Arguments {
    pub(crate) paths: Vec<OsString>,
    output: Option<PathBuf>,
    duration: Duration,
}

impl Arguments {
    pub(crate) fn parse() -> anyhow::Result<Self> {
        let mut arguments = std::env::args_os().skip(1);
        let mut paths = Vec::new();
        let mut output = None;
        let mut seconds = None;
        while let Some(argument) = arguments.next() {
            if argument == "--" {
                paths.extend(arguments);
                break;
            } else if argument == "--trace-startup" {
                anyhow::ensure!(
                    output.is_none(),
                    "--trace-startup may only be specified once"
                );
                let path = arguments.next().ok_or_else(|| {
                    anyhow::anyhow!("--trace-startup requires an output JSON path")
                })?;
                anyhow::ensure!(
                    !path.is_empty()
                        && path != "--trace-duration"
                        && path != "--trace-startup"
                        && path != "--",
                    "--trace-startup requires an output JSON path"
                );
                output = Some(PathBuf::from(path));
            } else if argument == "--trace-duration" {
                anyhow::ensure!(
                    seconds.is_none(),
                    "--trace-duration may only be specified once"
                );
                let value = arguments
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--trace-duration requires seconds (1–600)"))?;
                let value = value
                    .to_str()
                    .and_then(|value| value.parse::<u64>().ok())
                    .filter(|value| (1..=600).contains(value))
                    .ok_or_else(|| {
                        anyhow::anyhow!("--trace-duration must be an integer from 1 to 600")
                    })?;
                seconds = Some(value);
            } else if !argument.to_string_lossy().starts_with('-') {
                paths.push(argument);
            }
        }
        anyhow::ensure!(
            seconds.is_none() || output.is_some(),
            "--trace-duration requires --trace-startup"
        );
        Ok(Self {
            paths,
            output,
            duration: Duration::from_secs(seconds.unwrap_or(15)),
        })
    }

    pub(crate) fn begin(&self) -> anyhow::Result<Option<StartupTrace>> {
        let Some(output) = self.output.clone() else {
            return Ok(None);
        };
        let capture = Arc::new(Capture {
            start: Instant::now(),
            accepting: AtomicBool::new(true),
            end: Mutex::new(None),
            slices: Mutex::new(Vec::with_capacity(MAX_EVENTS)),
            lost: AtomicU64::new(0),
            active: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
        });
        let deadline = capture.start + self.duration;
        let foreground = thread::current().id();
        let was_enabled = profiler::trace_enabled();
        profiler::set_trace_enabled(true);
        let (stop, receiver) = mpsc::channel();
        let (attachment, attached) = mpsc::channel();
        let data = capture.clone();
        *ACTIVE.lock().unwrap_or_else(|e| e.into_inner()) = Some(capture);
        ENABLED.store(true, Ordering::Relaxed);
        let worker = thread::Builder::new()
            .name("VCLogg2StartupTrace".into())
            .spawn(move || {
                let _restore = TraceRestore(was_enabled);
                collect(
                    None,
                    foreground,
                    &data,
                    receiver,
                    HashMap::new(),
                    Some(attached),
                    Some(deadline),
                );
                *data.end.lock().unwrap_or_else(|e| e.into_inner()) =
                    Some(Instant::now().min(deadline));
                data.seal();
                // Restore tracing before allowing another UI recording to begin.
                drop(_restore);
                {
                    let mut active = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
                    if active
                        .as_ref()
                        .is_some_and(|active| Arc::ptr_eq(active, &data))
                    {
                        *active = None;
                        ENABLED.store(false, Ordering::Relaxed);
                    }
                }
                let result = export_trace(&data, &output);
                match &result {
                    Ok(lost) => eprintln!(
                        "Startup trace saved to {} (lost events: {lost})",
                        output.display()
                    ),
                    Err(error) => eprintln!(
                        "Could not save startup trace to {}: {error:#}",
                        output.display()
                    ),
                }
                result.map(|_| ())
            });
        let worker = match worker {
            Ok(worker) => worker,
            Err(error) => {
                *ACTIVE.lock().unwrap_or_else(|e| e.into_inner()) = None;
                ENABLED.store(false, Ordering::Relaxed);
                profiler::set_trace_enabled(was_enabled);
                return Err(error.into());
            }
        };
        Ok(Some(StartupTrace {
            stop,
            attachment,
            worker: Some(worker),
        }))
    }
}

pub(crate) struct StartupTrace {
    stop: mpsc::Sender<()>,
    attachment: mpsc::Sender<ForegroundJournalCollector>,
    worker: Option<JoinHandle<anyhow::Result<()>>>,
}

impl StartupTrace {
    pub(crate) fn attachment(&self) -> mpsc::Sender<ForegroundJournalCollector> {
        self.attachment.clone()
    }
}

impl Drop for StartupTrace {
    fn drop(&mut self) {
        // Main has returned (including startup failure/forwarding), so flush before exit.
        _ = self.stop.send(());
        if let Some(worker) = self.worker.take() {
            _ = worker.join();
        }
    }
}
