//! Cold-start capture begins before platform, fonts, storage, or window initialization.
use super::*;
use std::ffi::OsString;

pub(crate) struct Arguments {
    pub(crate) paths: Vec<OsString>,
    enabled: bool,
    duration: Duration,
}

impl Arguments {
    pub(crate) fn parse() -> anyhow::Result<Self> {
        let mut arguments = std::env::args_os().skip(1);
        let mut paths = Vec::new();
        let mut enabled = false;
        let mut seconds = None;
        while let Some(argument) = arguments.next() {
            if argument == "--" {
                paths.extend(arguments);
                break;
            } else if argument == "--trace-startup" {
                anyhow::ensure!(!enabled, "--trace-startup may only be specified once");
                enabled = true;
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
            seconds.is_none() || enabled,
            "--trace-duration requires --trace-startup"
        );
        Ok(Self {
            paths,
            enabled,
            duration: Duration::from_secs(seconds.unwrap_or(15)),
        })
    }

    pub(crate) fn begin(&self) -> anyhow::Result<Option<StartupTrace>> {
        if !self.enabled {
            return Ok(None);
        }
        let capture = Capture::new()?;
        let writer = live::Writer::start(capture.clone())?;
        let deadline = capture.start + self.duration;
        let foreground = capture.ui_thread;
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
                let result = writer.finish();
                if let Err(error) = &result {
                    *data.write_error.lock().unwrap_or_else(|e| e.into_inner()) =
                        Some(format!("{error:#}"));
                }
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
                match &result {
                    Ok(lost) => eprintln!(
                        "Startup trace saved to {} (lost events: {lost})",
                        data.path.display()
                    ),
                    Err(error) => eprintln!(
                        "Could not save startup trace to {}: {error:#}",
                        data.path.display()
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
