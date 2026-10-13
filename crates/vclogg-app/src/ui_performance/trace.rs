//! Application-owned, bounded Chrome Trace recording; no framework source patches.
pub(crate) mod startup;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
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
}

struct Capture {
    // Captured on the GPUI foreground, never inferred from worker names or export thread.
    ui_thread: ThreadId,
    start: Instant,
    accepting: AtomicBool,
    end: Mutex<Option<Instant>>,
    slices: Mutex<Vec<Slice>>,
    lost: AtomicU64,
    active: Mutex<HashMap<u64, Slice>>,
    next_id: AtomicU64,
}

impl Capture {
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
            slices.push(slice);
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
    worker: Option<JoinHandle<()>>,
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
        let mut recorder = Recorder { recording: None, ready: None, task: None, status: None };
        if is_recording() {
            recorder.status = Some(crate::tr!("正在录制冷启动，将按命令行设置自动导出。", "Cold-start recording will export automatically as configured on the command line.").into());
            recorder.task = Some(cx.spawn(async move |this, cx| {
                while is_recording() {
                    cx.background_executor().timer(Duration::from_millis(500)).await;
                }
                _ = this.update(cx, |this, cx| {
                    this.task = None;
                    this.status = Some(crate::tr!("冷启动采集已停止，导出结果见命令行指定路径及终端输出。", "Cold-start capture stopped. Check the configured output path and terminal for export results.").into());
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
    status: Option<SharedString>,
}

impl Recorder {
    fn start(&mut self, cx: &mut Context<Self>) {
        if is_recording() || self.recording.is_some() || self.task.is_some() || self.ready.is_some()
        {
            return;
        }
        let capture = Arc::new(Capture {
            ui_thread: thread::current().id(),
            start: Instant::now(),
            accepting: AtomicBool::new(true),
            end: Mutex::new(None),
            slices: Mutex::new(Vec::with_capacity(MAX_EVENTS)),
            lost: AtomicU64::new(0),
            active: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
        });
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

    fn stop_and_export(&mut self, cx: &mut Context<Self>) {
        if self.task.is_some() {
            return;
        }
        let Some(mut recording) = self.recording.take() else {
            return;
        };
        recording.stop();
        cx.refresh_windows();
        self.status = Some(crate::tr!("正在结束录制…", "Stopping recording…").into());
        self.task = Some(cx.spawn(async move |this, cx| {
            let (data, success) = cx.background_spawn(async move {
                let success = recording.worker.take().is_none_or(|worker| worker.join().is_ok());
                recording.capture.seal();
                (recording.capture.clone(), success)
            }).await;
            _ = this.update(cx, |this, cx| {
                this.task = None;
                this.ready = Some(data);
                if success { this.export(cx); }
                else {
                    this.status = Some(crate::tr!("采集线程异常结束，已保留部分数据，可重试导出。", "The collector stopped unexpectedly. Partial data is retained for export.").into());
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn export(&mut self, cx: &mut Context<Self>) {
        if self.task.is_some() {
            return;
        }
        let Some(data) = self.ready.clone() else {
            return;
        };
        let directory = dirs::document_dir().unwrap_or_else(|| PathBuf::from("."));
        let name = format!(
            "vclogg2-trace-{}.json",
            chrono::Local::now().format("%Y%m%d-%H%M%S")
        );
        let prompt = cx.prompt_for_new_path(&directory, Some(&name));
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = match prompt.await {
                Ok(Ok(Some(path))) => Some(
                    cx.background_spawn(async move {
                        export_trace(&data, &path).map(|lost| (path, lost))
                    })
                    .await,
                ),
                Ok(Ok(None)) => None,
                Ok(Err(error)) => Some(Err(error)),
                Err(error) => Some(Err(anyhow::anyhow!(error))),
            };
            _ = this.update(cx, |this, cx| {
                this.task = None;
                this.status = Some(
                    match result {
                        Some(Ok((path, lost))) => {
                            this.ready = None;
                            crate::tr_args!(
                                "已导出到 {}。丢失事件：{lost}。可使用 Perfetto 打开。",
                                "Exported to {}. Lost events: {lost}. Open the file in Perfetto.",
                                path.display()
                            )
                        }
                        Some(Err(error)) => crate::tr_args!(
                            "导出失败：{error}。数据已保留，可重试。",
                            "Export failed: {error}. Data is retained for retry."
                        ),
                        None => crate::tr!(
                            "已停止录制，数据已保留，可重新导出。",
                            "Recording stopped. Data is retained for export."
                        )
                        .to_owned(),
                    }
                    .into(),
                );
                cx.notify();
            });
        }));
        cx.notify();
    }
}

impl Render for Recorder {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let busy = self.task.is_some();
        v_flex().gap_2().p_3().border_1().border_color(cx.theme().border).rounded(cx.theme().radius)
            .child(div().font_weight(gpui_kit::FontWeight::SEMIBOLD).child(crate::tr!("性能录制", "Performance recording")))
            .child(div().text_sm().text_color(cx.theme().muted_foreground).child(crate::tr!(
                "录制 GPUI 事件和控件耗时，停止后导出供 Perfetto 查看。最多保留 20 万条事件，超出部分会标记为丢失。",
                "Record GPUI events and control timings, then export for Perfetto. Up to 200,000 events are retained; excess events are reported as lost."
            )))
            .child(h_flex().gap_2().flex_wrap()
                .child(Button::new("trace-start").small().outline().label(crate::tr!("开始录制", "Start recording"))
                    .disabled(is_recording() || busy || self.recording.is_some() || self.ready.is_some())
                    .on_click(cx.listener(|this, _, _, cx| this.start(cx))))
                .child(Button::new("trace-stop-export").small().outline().label(crate::tr!("停止并导出 trace…", "Stop and export trace…"))
                    .disabled(busy || self.recording.is_none())
                    .on_click(cx.listener(|this, _, _, cx| this.stop_and_export(cx))))
                .child(Button::new("trace-export-retry").small().outline().label(crate::tr!("重新导出…", "Retry export…"))
                    .disabled(busy || self.ready.is_none())
                    .on_click(cx.listener(|this, _, _, cx| this.export(cx))))
                .child(Button::new("trace-discard").small().ghost().label(crate::tr!("丢弃录制", "Discard recording"))
                    .disabled(busy || self.ready.is_none())
                    .on_click(cx.listener(|this, _, _, cx| { this.ready = None; this.status = None; cx.notify(); }))))
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
                    });
                }
            }
            break;
        }
    }
}

fn export_trace(capture: &Capture, path: &Path) -> anyhow::Result<u64> {
    use std::io::Write as _;
    let stopped_at = capture
        .end
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .unwrap_or_else(Instant::now);
    let mut slices = capture
        .slices
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    slices.extend(
        capture
            .active
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .cloned()
            .map(|mut slice| {
                slice.end = stopped_at;
                slice
            }),
    );
    let mut ordered = slices.iter().collect::<Vec<_>>();
    ordered.sort_by(|a, b| a.start.cmp(&b.start).then_with(|| b.end.cmp(&a.end)));
    // Chrome synchronous slices must nest. Async scopes can cross, so assign those
    // intervals separate lanes instead of emitting invalid overlapping X events.
    type Lanes = Vec<(usize, Vec<Instant>)>;
    let mut tracks: HashMap<(ThreadId, &'static str), Lanes> = HashMap::new();
    let mut next_tid = 1;
    let mut events = Vec::with_capacity(ordered.len() + 16);
    for slice in ordered {
        if slice.start >= stopped_at || slice.end <= capture.start {
            continue;
        }
        let ui_thread = slice.thread == capture.ui_thread;
        let thread_role = if ui_thread { "ui" } else { "background" };
        let thread_label = if ui_thread {
            "GPUI UI / Main"
        } else {
            "Background"
        };
        let start = slice.start.max(capture.start);
        let end = slice.end.min(stopped_at);
        let lanes = tracks.entry((slice.thread, slice.category)).or_default();
        for (_, stack) in lanes.iter_mut() {
            while stack.last().is_some_and(|last| *last <= start) {
                stack.pop();
            }
        }
        let lane_ix = lanes
            .iter()
            .position(|(_, stack)| stack.last().is_none_or(|parent_end| end <= *parent_end));
        let lane_ix = lane_ix.unwrap_or_else(|| {
            let tid = next_tid;
            next_tid += 1;
            let lane_ix = lanes.len();
            events.push(json!({"ph":"M", "name":"thread_name", "pid":1, "tid":tid,
                "args":{"name":format!("{} · {} · lane {} · {:?}", thread_label, slice.category, lane_ix + 1, slice.thread)}}));
            lanes.push((tid, Vec::new()));
            lane_ix
        });
        let (tid, stack) = &mut lanes[lane_ix];
        stack.push(end);
        if start == end {
            events.push(json!({"ph":"i", "s":"t", "name":slice.name, "cat":slice.category, "pid":1, "tid":*tid,
                "ts":start.duration_since(capture.start).as_secs_f64() * 1_000_000.,
                "args":{"thread_role":thread_role,"source_thread":format!("{:?}", slice.thread)}}));
            continue;
        }
        events.push(
            json!({"ph":"X", "name":slice.name, "cat":slice.category, "pid":1, "tid":*tid,
            "ts":start.duration_since(capture.start).as_secs_f64() * 1_000_000.,
            "dur":end.saturating_duration_since(start).as_secs_f64() * 1_000_000., "args":{"incomplete":slice.incomplete || slice.end > stopped_at,"thread_role":thread_role,"source_thread":format!("{:?}", slice.thread)}}),
        );
    }
    let lost = capture.lost.load(Ordering::Relaxed);
    events.push(json!({"ph":"M", "name":"process_name", "pid":1, "args":{"name":"VCLogg2"}}));
    events.push(json!({"ph":"i", "s":"g", "name":"Recording summary", "pid":1, "tid":0, "ts":0,
        "args":{"ui_thread":format!("{:?}", capture.ui_thread),"ui_track_prefix":"GPUI UI / Main","lost_events":lost,"limit":MAX_EVENTS,"includes_active_scopes":true,"active_scope_limit":MAX_ACTIVE_SCOPES,"duration_ms":stopped_at.duration_since(capture.start).as_secs_f64()*1000.,"build":if cfg!(debug_assertions) {"Debug"} else {"Release"},"version":env!("CARGO_PKG_VERSION"),"tracks":"Synthetic lanes grouped by source Rust thread and category; GPUI UI / Main identifies the foreground thread. Durations are wall time, not CPU samples or GPU timings; overlapping tracks are not additive"}}));
    // Serialize on the worker and replace the destination only after the whole file is written.
    let parent = path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut output = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer(
        output.as_file_mut(),
        &json!({"traceEvents":events,"displayTimeUnit":"ms"}),
    )?;
    output.flush()?;
    output.as_file().sync_all()?;
    output.persist(path).map_err(|error| error.error)?;
    Ok(lost)
}
