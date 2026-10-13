//! A single writer appends batches; stopping only drains the remaining events.
use super::*;

const SAVE_INTERVAL: Duration = Duration::from_millis(250);

pub(super) fn save(capture: &Capture) -> anyhow::Result<u64> {
    capture
        .stream
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .save(capture, true)
}

pub(super) struct Writer {
    stop: mpsc::Sender<()>,
    worker: Option<JoinHandle<anyhow::Result<u64>>>,
}

impl Writer {
    pub(super) fn start(capture: Arc<Capture>) -> anyhow::Result<Self> {
        let (stop, receiver) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("VCLogg2TraceWriter".into())
            .spawn(move || {
                let mut stopping = false;
                loop {
                    let result = capture
                        .stream
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .save(&capture, stopping);
                    *capture
                        .write_error
                        .lock()
                        .unwrap_or_else(|e| e.into_inner()) =
                        result.as_ref().err().map(|error| format!("{error:#}"));
                    if stopping {
                        return result;
                    }
                    stopping = !matches!(
                        receiver.recv_timeout(SAVE_INTERVAL),
                        Err(mpsc::RecvTimeoutError::Timeout)
                    );
                }
            })?;
        Ok(Self {
            stop,
            worker: Some(worker),
        })
    }

    pub(super) fn finish(mut self) -> anyhow::Result<u64> {
        _ = self.stop.send(());
        self.worker
            .take()
            .expect("trace writer handle")
            .join()
            .map_err(|_| anyhow::anyhow!("Trace writer panicked"))?
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        // Even a failed collector spawn requests a final drain; never join on the UI.
        _ = self.stop.send(());
    }
}
