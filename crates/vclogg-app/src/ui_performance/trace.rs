//! Application-owned, bounded Chrome Trace recording; no framework source patches.
mod live;
pub(crate) mod startup;
mod stream;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle, ThreadId},
    time::{Duration, Instant},
};

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use gpui_kit::profiler::{
    self,
    journal::{
        ForegroundEvent, ForegroundJournalCollector, ForegroundJournalEntry, FrameStateChange,
        IntervalBoundary,
    },
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, Global, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, Task, Window, div,
};
use serde_json::json;

const MAX_EVENTS: usize = 200_000;
const MAX_ACTIVE_SCOPES: usize = 4096;
static ENABLED: AtomicBool = AtomicBool::new(false);
static ACTIVE: Mutex<Option<Arc<Capture>>> = Mutex::new(None);

pub(super) fn is_recording() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

#[derive(Clone)]
struct Slice {
    name: String,
    category: &'static str,
    thread: ThreadId,
    start: Instant,
    end: Instant,
    incomplete: bool,
    scope_id: Option<u64>,
}

struct Capture {
    // Captured on the GPUI foreground, never inferred from worker names or export thread.
    ui_thread: ThreadId,
    start: Instant,
    accepting: AtomicBool,
    end: Mutex<Option<Instant>>,
    slices: Mutex<Vec<Arc<Slice>>>,
    path: PathBuf,
    write_error: Mutex<Option<String>>,
    stream: Mutex<stream::Stream>,
    lost: AtomicU64,
    active: Mutex<HashMap<u64, Slice>>,
    next_id: AtomicU64,
}

impl Capture {
    fn new() -> anyhow::Result<Arc<Self>> {
        let directory = crate::app_paths::trace_cache_dir()
            .ok_or_else(|| anyhow::anyhow!("Could not determine trace cache directory"))?;
        Ok(Arc::new(Self {
            ui_thread: thread::current().id(),
            start: Instant::now(),
            accepting: AtomicBool::new(true),
            end: Mutex::new(None),
            slices: Mutex::new(Vec::with_capacity(MAX_EVENTS)),
            path: directory.join(format!(
                "vclogg2-trace-{}-{}.json",
                chrono::Local::now().format("%Y%m%d-%H%M%S"),
                uuid::Uuid::new_v4()
            )),
            write_error: Mutex::new(None),
            stream: Mutex::new(stream::Stream::default()),
            lost: AtomicU64::new(0),
            active: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
        }))
    }

    fn seal(&self) {
        // Serialize with scope completion so unfinished scopes remain available to export.
        let _active = self.active.lock().unwrap_or_else(|e| e.into_inner());
        self.accepting.store(false, Ordering::Relaxed);
    }

    fn marker(&self, name: String, thread: ThreadId, at: Instant) {
        self.push(Slice {
            name,
            category: "GPUI markers",
            thread,
            start: at,
            end: at,
            incomplete: false,
            scope_id: None,
        });
    }

    fn push(&self, slice: Slice) {
        if !self.accepting.load(Ordering::Relaxed) || slice.end < self.start {
            return;
        }
        // Recording must never stall the UI behind a collector or exporter.
        if let Ok(mut slices) = self.slices.try_lock()
            && slices.len() < MAX_EVENTS
        {
            if !self.accepting.load(Ordering::Relaxed) {
                return;
            }
            slices.push(Arc::new(slice));
            return;
        }
        self.lost.fetch_add(1, Ordering::Relaxed);
    }
}

pub(crate) struct Scope {
    capture: Option<(Arc<Capture>, u64)>,
}

#[must_use]
pub(crate) fn scope(name: &'static str) -> Scope {
    let capture = if ENABLED.load(Ordering::Relaxed) {
        ACTIVE.lock().ok().and_then(|active| active.clone())
    } else {
        None
    };
    let capture = capture.and_then(|capture| {
        if !capture.accepting.load(Ordering::Relaxed) {
            return None;
        }
        let id = capture.next_id.fetch_add(1, Ordering::Relaxed);
        let start = Instant::now();
        let recorded = if let Ok(mut active) = capture.active.try_lock() {
            if capture.accepting.load(Ordering::Relaxed) && active.len() < MAX_ACTIVE_SCOPES {
                active.insert(
                    id,
                    Slice {
                        name: name.to_owned(),
                        category: "Application",
                        thread: thread::current().id(),
                        start,
                        end: start,
                        incomplete: true,
                        scope_id: Some(id),
                    },
                );
                true
            } else {
                false
            }
        } else {
            false
        };
        if recorded {
            Some((capture, id))
        } else {
            capture.lost.fetch_add(1, Ordering::Relaxed);
            None
        }
    });
    Scope { capture }
}

impl Scope {
    pub(super) fn finish(&mut self) {
        if let Some((capture, id)) = self.capture.take() {
            let end = Instant::now();
            // Bounded map operations only; exporters clone it before serialization.
            let mut active = capture.active.lock().unwrap_or_else(|e| e.into_inner());
            if !capture.accepting.load(Ordering::Relaxed) {
                return;
            }
            let slice = active.remove(&id);
            if let Some(mut slice) = slice {
                slice.end = end;
                slice.incomplete = false;
                capture.push(slice);
            }
        }
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        self.finish();
    }
}

struct Recording {
    stop: Option<mpsc::Sender<()>>,
    worker: Option<JoinHandle<anyhow::Result<u64>>>,
    capture: Arc<Capture>,
}
impl Recording {
    fn stop(&mut self) {
        self.capture
            .end
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_or_insert_with(Instant::now);
        let mut active = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
        if active
            .as_ref()
            .is_some_and(|active| Arc::ptr_eq(active, &self.capture))
        {
            ENABLED.store(false, Ordering::Relaxed);
            *active = None;
        }
        if let Some(stop) = self.stop.take() {
            _ = stop.send(());
        }
    }
}
impl Drop for Recording {
    fn drop(&mut self) {
        self.stop();
    }
}

struct TraceGlobal(Entity<Recorder>);
impl Global for TraceGlobal {}

pub(crate) fn recorder(cx: &mut App) -> Entity<Recorder> {
    if let Some(global) = cx.try_global::<TraceGlobal>() {
        return global.0.clone();
    }
    let recorder = cx.new(|cx: &mut Context<Recorder>| {
        let mut recorder = Recorder {
            recording: None,
            ready: None,
            task: None,
            directory_task: None,
            watch: None,
            status: None,
        };
        if is_recording() {
            let cold_capture = ACTIVE.lock().unwrap_or_else(|e| e.into_inner()).clone();
            recorder.status = Some(
                crate::tr!(
                    "正在录制冷启动，文件将持续保存到缓存目录。",
                    "Cold-start capture is saving to the cache folder."
                )
                .into(),
            );
            recorder.task = Some(cx.spawn(async move |this, cx| {
                let mut previous_error = None;
                while is_recording() {
                    cx.background_executor()
                        .timer(Duration::from_millis(500))
                        .await;
                    let error = cold_capture.as_ref().and_then(|data| {
                        data.write_error.lock().ok().and_then(|error| error.clone())
                    });
                    if error != previous_error {
                        previous_error = error;
                        _ = this.update(cx, |_, cx| cx.notify());
                    }
                }
                _ = this.update(cx, |this, cx| {
                    this.task = None;
                    this.status = Some(
                        crate::tr!(
                            "冷启动采集已停止，可打开目录查看录制文件。",
                            "Cold-start capture stopped. Open the folder to find the recording."
                        )
                        .into(),
                    );
                    if let Some(data) = cold_capture {
                        let error = data
                            .write_error
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .clone();
                        if let Some(error) = error {
                            this.ready = Some(data);
                            this.status = Some(
                                crate::tr_args!(
                                    "保存失败：{error}。数据已保留，可重试保存。",
                                    "Save failed: {error}. Data is retained for retry."
                                )
                                .into(),
                            );
                        }
                    }
                    cx.refresh_windows();
                    cx.notify();
                });
            }));
        }
        recorder
    });
    cx.set_global(TraceGlobal(recorder.clone()));
    recorder
}

/// Retained by the application so closing Settings does not cancel a recording or export.
pub(crate) struct Recorder {
    recording: Option<Recording>,
    ready: Option<Arc<Capture>>,
    task: Option<Task<()>>,
    directory_task: Option<Task<()>>,
    watch: Option<Task<()>>,
    status: Option<SharedString>,
}

impl Recorder {
    fn start(&mut self, cx: &mut Context<Self>) {
        if is_recording() || self.recording.is_some() || self.task.is_some() || self.ready.is_some()
        {
            return;
        }
        let capture = match Capture::new() {
            Ok(capture) => capture,
            Err(error) => {
                self.status = Some(
                    crate::tr_args!("无法开始录制：{error}", "Couldn’t start recording: {error}")
                        .into(),
                );
                cx.notify();
                return;
            }
        };
        let writer = match live::Writer::start(capture.clone()) {
            Ok(writer) => writer,
            Err(error) => {
                self.status = Some(
                    crate::tr_args!("无法开始录制：{error}", "Couldn’t start recording: {error}")
                        .into(),
                );
                cx.notify();
                return;
            }
        };
        let collector = cx.foreground_journal().collector();
        let foreground = capture.ui_thread;
        let task_cursors = profiler::get_all_timings(gpui_kit::TasksIncluded::OnlyCompleted)
            .into_iter()
            .map(|thread| (thread.thread_id, thread.total_pushed))
            .collect();
        let was_enabled = profiler::trace_enabled();
        profiler::set_trace_enabled(true);
        let (stop, receiver) = mpsc::channel();
        let data = capture.clone();
        let worker = thread::Builder::new()
            .name("VCLogg2Trace".into())
            .spawn(move || {
                let _restore = TraceRestore(was_enabled);
                collect(
                    Some(collector),
                    foreground,
                    &data,
                    receiver,
                    task_cursors,
                    None,
                    None,
                );
                data.seal();
                writer.finish()
            });
        match worker {
            Ok(worker) => {
                *ACTIVE.lock().unwrap_or_else(|e| e.into_inner()) = Some(capture.clone());
                ENABLED.store(true, Ordering::Relaxed);
                self.recording = Some(Recording {
                    stop: Some(stop),
                    worker: Some(worker),
                    capture,
                });
                self.watch = Some(cx.spawn(async move |this, cx| {
                    let mut previous_error = None;
                    loop {
                        cx.background_executor()
                            .timer(Duration::from_millis(500))
                            .await;
                        let keep_watching = this
                            .update(cx, |this, cx| {
                                let Some(recording) = &this.recording else {
                                    return false;
                                };
                                let error = recording
                                    .capture
                                    .write_error
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner())
                                    .clone();
                                if error != previous_error {
                                    previous_error = error;
                                    cx.notify();
                                }
                                true
                            })
                            .unwrap_or(false);
                        if !keep_watching {
                            break;
                        }
                    }
                }));
                self.status = Some(
                    crate::tr!(
                        "正在录制，可关闭设置并复现卡顿。",
                        "Recording. You can close Settings and reproduce the stutter."
                    )
                    .into(),
                );
            }
            Err(error) => {
                profiler::set_trace_enabled(was_enabled);
                self.status = Some(
                    crate::tr_args!("无法开始录制：{error}", "Couldn’t start recording: {error}")
                        .into(),
                );
            }
        }
        cx.refresh_windows();
        cx.notify();
    }

    fn stop_and_save(&mut self, cx: &mut Context<Self>) {
        if self.task.is_some() {
            return;
        }
        let Some(mut recording) = self.recording.take() else {
            return;
        };
        recording.stop();
        self.watch = None;
        cx.refresh_windows();
        self.status = Some(crate::tr!("正在结束录制…", "Stopping recording…").into());
        self.task = Some(cx.spawn(async move |this, cx| {
            let (data, result) = cx
                .background_spawn(async move {
                    let result = recording
                        .worker
                        .take()
                        .ok_or_else(|| anyhow::anyhow!("Missing trace collector"))
                        .and_then(|worker| {
                            worker
                                .join()
                                .map_err(|_| anyhow::anyhow!("Trace collector panicked"))
                        })
                        .and_then(|result| result);
                    recording.capture.seal();
                    (recording.capture.clone(), result)
                })
                .await;
            _ = this.update(cx, |this, cx| {
                this.task = None;
                match result {
                    Ok(lost) => {
                        this.status = Some(
                            crate::tr_args!(
                                "已保存到 {}。丢失事件：{lost}。",
                                "Saved to {}. Lost events: {lost}.",
                                data.path.display()
                            )
                            .into(),
                        )
                    }
                    Err(error) => {
                        this.ready = Some(data);
                        this.status = Some(
                            crate::tr_args!(
                                "保存失败：{error}。数据已保留，可重试保存。",
                                "Save failed: {error}. Data is retained for retry."
                            )
                            .into(),
                        );
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn retry_save(&mut self, cx: &mut Context<Self>) {
        if self.task.is_some() {
            return;
        }
        let Some(data) = self.ready.clone() else {
            return;
        };
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(
                    async move { live::save(&data).map(|lost| (data.path.clone(), lost)) },
                )
                .await;
            _ = this.update(cx, |this, cx| {
                this.task = None;
                this.status = Some(
                    match result {
                        Ok((path, lost)) => {
                            this.ready = None;
                            crate::tr_args!(
                                "已保存到 {}。丢失事件：{lost}。",
                                "Saved to {}. Lost events: {lost}.",
                                path.display()
                            )
                        }
                        Err(error) => crate::tr_args!(
                            "保存失败：{error}。数据已保留，可重试。",
                            "Save failed: {error}. Data is retained for retry."
                        ),
                    }
                    .into(),
                );
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn open_directory(&mut self, cx: &mut Context<Self>) {
        if self.directory_task.is_some() {
            return;
        }
        self.directory_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let directory = crate::app_paths::trace_cache_dir().ok_or_else(|| {
                        anyhow::anyhow!("Could not determine trace cache directory")
                    })?;
                    std::fs::create_dir_all(&directory)?;
                    Ok::<_, anyhow::Error>(directory)
                })
                .await;
            _ = this.update(cx, |this, cx| {
                this.directory_task = None;
                match result {
                    Ok(directory) => cx.reveal_path(&directory),
                    Err(error) => {
                        this.status = Some(
                            crate::tr_args!(
                                "打开目录失败：{error}",
                                "Couldn’t open folder: {error}"
                            )
                            .into(),
                        )
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
}

impl Render for Recorder {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let busy = self.task.is_some();
        let capture = self
            .recording
            .as_ref()
            .map(|recording| recording.capture.clone())
            .or_else(|| ACTIVE.lock().ok().and_then(|active| active.clone()));
        let write_error = capture
            .as_ref()
            .and_then(|data| data.write_error.lock().ok().and_then(|error| error.clone()));
        v_flex().gap_2().p_3().border_1().border_color(cx.theme().border).rounded(cx.theme().radius)
            .child(div().font_weight(gpui_kit::FontWeight::SEMIBOLD).child(crate::tr!("性能录制", "Performance recording")))
            .child(div().text_sm().text_color(cx.theme().muted_foreground).child(crate::tr!(
                "录制期间持续写入缓存目录，停止时仅写完剩余事件。异常退出后已写入的事件仍可用 Perfetto 打开。最多记录 20 万条事件。",
                "Events are written to the cache folder while recording. Stopping drains remaining events. Perfetto can open written events after a crash. Up to 200,000 events are recorded."
            )))
            .child(h_flex().gap_2().flex_wrap()
                .child(Button::new("trace-start").small().outline().label(crate::tr!("开始录制", "Start recording"))
                    .disabled(is_recording() || busy || self.recording.is_some() || self.ready.is_some())
                    .on_click(cx.listener(|this, _, _, cx| this.start(cx))))
                .child(Button::new("trace-stop-export").small().outline().label(crate::tr!("停止录制", "Stop recording"))
                    .disabled(busy || self.recording.is_none())
                    .on_click(cx.listener(|this, _, _, cx| this.stop_and_save(cx))))
                .child(Button::new("trace-export-retry").small().outline().label(crate::tr!("重试保存", "Retry save"))
                    .disabled(busy || self.ready.is_none())
                    .on_click(cx.listener(|this, _, _, cx| this.retry_save(cx))))
                .child(Button::new("trace-discard").small().ghost().label(crate::tr!("放弃重试", "Dismiss retry"))
                    .disabled(busy || self.ready.is_none())
                    .on_click(cx.listener(|this, _, _, cx| { this.ready = None; this.status = None; cx.notify(); })))
                .child(Button::new("trace-open-directory").small().outline().label(crate::tr!("打开目录", "Open folder"))
                .disabled(self.directory_task.is_some()).on_click(cx.listener(|this, _, _, cx| this.open_directory(cx)))))
            .children(write_error
                .map(|error| div().text_sm().text_color(cx.theme().danger).child(crate::tr_args!("写入失败，将自动重试：{error}", "Write failed; retrying automatically: {error}"))))
            .children(capture.map(|data| div().text_sm().text_color(cx.theme().muted_foreground).child(data.path.display().to_string())))
            .children(self.status.clone().map(|status| div().text_sm().child(status)))
    }
}

struct TraceRestore(bool);
impl Drop for TraceRestore {
    fn drop(&mut self) {
        profiler::set_trace_enabled(self.0);
    }
}

fn collect(
    mut collector: Option<ForegroundJournalCollector>,
    foreground: ThreadId,
    capture: &Capture,
    stop: mpsc::Receiver<()>,
    mut task_cursors: HashMap<ThreadId, u64>,
    attachment: Option<mpsc::Receiver<ForegroundJournalCollector>>,
    deadline: Option<Instant>,
) {
    loop {
        let stopping = deadline.is_some_and(|deadline| Instant::now() >= deadline)
            || !matches!(
                stop.recv_timeout(Duration::from_millis(50)),
                Err(mpsc::RecvTimeoutError::Timeout)
            );
        if let Some(attachment) = &attachment
            && let Ok(attached) = attachment.try_recv()
        {
            collector = Some(attached);
        }
        let drained = collector
            .as_mut()
            .map(|collector| collector.collect_unseen())
            .unwrap_or_default();
        capture.lost.fetch_add(drained.lost, Ordering::Relaxed);
        for entry in drained.entries {
            let event = match entry {
                ForegroundJournalEntry::Event(event) => event,
                ForegroundJournalEntry::Boundary(IntervalBoundary::Presented(frame)) => {
                    if let Some(start) = frame.frame.dirty_at {
                        capture.push(Slice {
                            name: format!("Dirty to present {:?}", frame.frame.window_id),
                            category: "GPUI frame latency",
                            thread: foreground,
                            start,
                            end: frame.presentation.present_end,
                            incomplete: false,
                            scope_id: None,
                        });
                    }
                    continue;
                }
                ForegroundJournalEntry::Boundary(boundary) => {
                    let name = match boundary {
                        IntervalBoundary::FrameSkipped(frame) => {
                            format!("Frame skipped {:?}: {:?}", frame.window_id, frame.reason)
                        }
                        IntervalBoundary::Idle { .. } => "Foreground idle".to_owned(),
                        IntervalBoundary::PowerTransition { .. } => "Power transition".to_owned(),
                        IntervalBoundary::Presented(_) => unreachable!(),
                    };
                    capture.marker(name, foreground, boundary.end_time());
                    continue;
                }
                ForegroundJournalEntry::FrameState(state) => {
                    let (name, at) = match state {
                        FrameStateChange::Pending {
                            window_id,
                            dirty_at,
                        } => (format!("Frame pending {window_id:?}"), dirty_at),
                        FrameStateChange::Closed { window_id, at } => {
                            (format!("Window closed {window_id:?}"), at)
                        }
                    };
                    capture.marker(name, foreground, at);
                    continue;
                }
                ForegroundJournalEntry::PlatformSignal(signal) => {
                    capture.marker(
                        format!("Frame callback {:?}: {:?}", signal.window_id, signal.source),
                        foreground,
                        signal.handled_at,
                    );
                    if let Some(start) =
                        signal.signal_at.filter(|start| *start <= signal.handled_at)
                    {
                        capture.push(Slice {
                            name: format!("Callback delivery {:?}", signal.window_id),
                            category: "GPUI frame latency",
                            thread: foreground,
                            start,
                            end: signal.handled_at,
                            incomplete: false,
                            scope_id: None,
                        });
                    }
                    continue;
                }
                ForegroundJournalEntry::ReentrantFrameSkipped { window_id, at } => {
                    capture.marker(
                        format!("Reentrant frame skipped {window_id:?}"),
                        foreground,
                        at,
                    );
                    continue;
                }
                ForegroundJournalEntry::Discontinuity { lost } => {
                    capture.marker(
                        format!("Journal gap: {lost} entries (observed here)"),
                        foreground,
                        Instant::now(),
                    );
                    continue;
                }
            };
            let (name, category) = match event {
                ForegroundEvent::TaskPoll(_) => continue,
                ForegroundEvent::Action(action) => (action.name.to_owned(), "GPUI actions"),
                ForegroundEvent::Input(input) => (input.kind.to_owned(), "GPUI input"),
                ForegroundEvent::Draw(frame) => {
                    (format!("Draw {:?}", frame.window_id), "GPUI draw")
                }
                ForegroundEvent::Present(frame) => {
                    (format!("Present {:?}", frame.window_id), "GPUI present")
                }
                // Aggregated short polls have gaps; never represent their span as CPU work.
                ForegroundEvent::SmallPolls(polls) => {
                    capture.marker(
                        format!(
                            "Short polls: {} ({:.3} ms total)",
                            polls.summary.count,
                            polls.summary.total.as_secs_f64() * 1000.
                        ),
                        foreground,
                        polls.until,
                    );
                    continue;
                }
            };
            capture.push(Slice {
                name,
                category,
                thread: foreground,
                start: event.start_time(),
                end: event.end_time(),
                incomplete: false,
                scope_id: None,
            });
        }
        for thread in profiler::get_all_timings(gpui_kit::TasksIncluded::OnlyCompleted) {
            let cursor = task_cursors.entry(thread.thread_id).or_default();
            let retained_start = thread
                .total_pushed
                .saturating_sub(thread.timings.len() as u64);
            capture
                .lost
                .fetch_add(retained_start.saturating_sub(*cursor), Ordering::Relaxed);
            let skip = cursor.saturating_sub(retained_start) as usize;
            for task in thread.timings.iter().skip(skip) {
                capture.push(Slice {
                    name: format!("Task {}", task.location),
                    category: "GPUI tasks",
                    thread: thread.thread_id,
                    start: task.start,
                    end: task.end.0,
                    incomplete: false,
                    scope_id: None,
                });
            }
            *cursor = thread.total_pushed;
        }
        if stopping {
            let snapshot_at = Instant::now();
            for thread in profiler::get_all_timings(gpui_kit::TasksIncluded::CompletedAndRunning) {
                if let Some(task) = thread
                    .timings
                    .last()
                    .filter(|task| task.end.0 >= snapshot_at)
                {
                    capture.push(Slice {
                        name: format!("Task at capture boundary {}", task.location),
                        category: "GPUI tasks",
                        thread: thread.thread_id,
                        start: task.start,
                        end: task.end.0,
                        incomplete: true,
                        scope_id: None,
                    });
                }
            }
            break;
        }
    }
}
