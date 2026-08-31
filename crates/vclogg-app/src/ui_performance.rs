#[cfg(all(feature = "ui-performance-profiler", not(debug_assertions)))]
compile_error!("ui-performance-profiler 仅允许用于 Debug 性能诊断构建");

#[cfg(debug_assertions)]
mod debug {
    use std::{
        backtrace::Backtrace,
        cell::{Cell, RefCell},
        collections::HashMap,
        sync::{Mutex, OnceLock},
        thread::{self, ThreadId},
        time::{Duration, Instant},
    };

    #[cfg(feature = "ui-performance-profiler")]
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    #[cfg(feature = "ui-performance-profiler")]
    use gpui::profiler::hang::{HangDetector, SerializedHangIncident};

    use gpui::{
        AnyElement, App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId,
        IntoElement, LayoutId, Pixels, Window,
    };

    const LOG_TARGET: &str = "vclogg2::ui_performance";
    const DEFAULT_WARN_AFTER_MS: u64 = 16;
    const DEFAULT_REPEAT_AFTER_MS: u64 = 2_000;
    const MIN_STACK_INTERVAL: Duration = Duration::from_millis(100);
    const MAX_SETTING_MS: u64 = 60_000;
    #[cfg(feature = "ui-performance-profiler")]
    const FRAME_MONITOR_INTERVAL: Duration = Duration::from_millis(250);
    #[cfg(feature = "ui-performance-profiler")]
    const MAX_FRAME_CONTRIBUTORS: usize = 12;

    #[derive(Clone, Copy)]
    struct Settings {
        warn_after: Duration,
        repeat_after: Duration,
    }

    #[derive(Default)]
    struct ScopeLogState {
        last_logged_at: Option<Instant>,
        suppressed_count: u64,
    }

    #[derive(Default)]
    struct LogStates {
        last_stack_at: Option<Instant>,
        scopes: HashMap<&'static str, ScopeLogState>,
    }

    #[derive(Default)]
    struct AggregateSample {
        count: u64,
        total: Duration,
        maximum: Duration,
    }

    struct ActiveAggregate {
        name: &'static str,
        started_at: Instant,
        diagnostic_overhead_at_start: Duration,
        samples: HashMap<&'static str, AggregateSample>,
    }

    struct AggregateGuard {
        name: &'static str,
    }

    thread_local! {
        static ACTIVE_AGGREGATES: RefCell<Vec<ActiveAggregate>> = const { RefCell::new(Vec::new()) };
        // 子作用域打印堆栈时会暂停 UI 线程。累计这部分诊断开销，供仍在计时的
        // 父作用域扣除，避免一次真实热点被放大成多层数百毫秒的误报。
        static DIAGNOSTIC_OVERHEAD: Cell<Duration> = const { Cell::new(Duration::ZERO) };
    }

    static SETTINGS: OnceLock<Settings> = OnceLock::new();
    static UI_THREAD_ID: OnceLock<ThreadId> = OnceLock::new();
    static LOG_STATES: OnceLock<Mutex<LogStates>> = OnceLock::new();

    pub(crate) struct UiPerformanceScope {
        name: &'static str,
        started_at: Instant,
        diagnostic_overhead_at_start: Duration,
        warn_after: Duration,
    }

    pub(crate) struct UiPerformanceElement {
        child: AnyElement,
        request_layout_name: &'static str,
        prepaint_name: &'static str,
        paint_name: &'static str,
    }

    pub(crate) fn element(
        request_layout_name: &'static str,
        prepaint_name: &'static str,
        paint_name: &'static str,
        child: impl IntoElement,
    ) -> UiPerformanceElement {
        UiPerformanceElement {
            child: child.into_any_element(),
            request_layout_name,
            prepaint_name,
            paint_name,
        }
    }

    impl IntoElement for UiPerformanceElement {
        type Element = Self;

        fn into_element(self) -> Self::Element {
            self
        }
    }

    impl Element for UiPerformanceElement {
        type RequestLayoutState = ();
        type PrepaintState = ();

        fn id(&self) -> Option<ElementId> {
            None
        }

        fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
            None
        }

        fn request_layout(
            &mut self,
            _: Option<&GlobalElementId>,
            _: Option<&InspectorElementId>,
            window: &mut Window,
            cx: &mut App,
        ) -> (LayoutId, Self::RequestLayoutState) {
            let _performance_scope = scope(self.request_layout_name);
            let _aggregate = aggregate(self.request_layout_name);
            (self.child.request_layout(window, cx), ())
        }

        fn prepaint(
            &mut self,
            _: Option<&GlobalElementId>,
            _: Option<&InspectorElementId>,
            _: Bounds<Pixels>,
            _: &mut Self::RequestLayoutState,
            window: &mut Window,
            cx: &mut App,
        ) -> Self::PrepaintState {
            let _performance_scope = scope(self.prepaint_name);
            let _aggregate = aggregate(self.prepaint_name);
            self.child.prepaint(window, cx);
        }

        fn paint(
            &mut self,
            _: Option<&GlobalElementId>,
            _: Option<&InspectorElementId>,
            _: Bounds<Pixels>,
            _: &mut Self::RequestLayoutState,
            _: &mut Self::PrepaintState,
            window: &mut Window,
            cx: &mut App,
        ) {
            let _performance_scope = scope(self.paint_name);
            let _aggregate = aggregate(self.paint_name);
            self.child.paint(window, cx);
        }
    }

    pub(crate) fn init_ui_thread() {
        let current_thread = thread::current();
        let _ = UI_THREAD_ID.set(current_thread.id());
        let settings = settings();
        log::debug!(
            target: LOG_TARGET,
            "Debug UI 性能检测已启用：长任务阈值={}ms，重复堆栈限频={}ms，父级净耗时排除诊断开销=true，UI线程={:?}",
            settings.warn_after.as_millis(),
            settings.repeat_after.as_millis(),
            current_thread.id(),
        );
    }

    #[cfg(feature = "ui-performance-profiler")]
    pub(crate) fn start_framework_monitor(cx: &mut gpui::App) {
        let settings = settings();
        let startup = Instant::now();
        let mut detector = HangDetector::new(
            cx.foreground_journal(),
            settings.warn_after,
            settings.warn_after,
        );
        let stopping = Arc::new(AtomicBool::new(false));
        let stopping_on_quit = Arc::clone(&stopping);
        std::mem::forget(cx.on_app_quit(move |_| {
            stopping_on_quit.store(true, Ordering::Release);
            async {}
        }));

        let monitor = thread::Builder::new()
            .name("VCLogg2UiPerformance".to_owned())
            .spawn(move || {
                while !stopping.load(Ordering::Acquire) {
                    thread::sleep(FRAME_MONITOR_INTERVAL);
                    let incidents = detector.poll();
                    let first_present_at = detector.first_present_at();
                    for incident in incidents {
                        let serialized = SerializedHangIncident::convert(
                            startup,
                            &incident,
                            MAX_FRAME_CONTRIBUTORS,
                            first_present_at,
                        );
                        match serde_json::to_string(&serialized) {
                            Ok(json) => log::warn!(
                                target: LOG_TARGET,
                                "检测到 GPUI 渲染线程长耗时区间：threshold_ms={} incident={json}",
                                settings.warn_after.as_millis(),
                            ),
                            Err(error) => log::warn!(
                                target: LOG_TARGET,
                                "GPUI 渲染线程长耗时区间序列化失败：{error}"
                            ),
                        }
                    }
                }
            });
        match monitor {
            Ok(_) => log::debug!(
                target: LOG_TARGET,
                "GPUI 前台任务与完整帧检测已启用：事件阈值={}ms，帧预算={}ms，采样间隔={}ms",
                settings.warn_after.as_millis(),
                settings.warn_after.as_millis(),
                FRAME_MONITOR_INTERVAL.as_millis(),
            ),
            Err(error) => log::error!(
                target: LOG_TARGET,
                "无法启动 GPUI 前台任务与完整帧检测线程：{error}"
            ),
        }
    }

    #[cfg(not(feature = "ui-performance-profiler"))]
    #[inline(always)]
    pub(crate) fn start_framework_monitor(_: &mut gpui::App) {}

    #[must_use]
    #[inline]
    pub(crate) fn scope(name: &'static str) -> UiPerformanceScope {
        UiPerformanceScope {
            name,
            started_at: Instant::now(),
            diagnostic_overhead_at_start: diagnostic_overhead(),
            warn_after: settings().warn_after,
        }
    }

    impl Drop for UiPerformanceScope {
        fn drop(&mut self) {
            let elapsed =
                elapsed_without_diagnostics(self.started_at, self.diagnostic_overhead_at_start);
            record_aggregate_sample(self.name, elapsed);
            if elapsed < self.warn_after || !log::log_enabled!(target: LOG_TARGET, log::Level::Warn)
            {
                return;
            }

            let now = Instant::now();
            let Some(suppressed_count) = begin_log(self.name, now) else {
                return;
            };
            let current_thread = thread::current();
            let expected_ui_thread = UI_THREAD_ID.get();
            let is_ui_thread = expected_ui_thread == Some(&current_thread.id());
            let thread_name = current_thread.name().unwrap_or("<unnamed>");
            let diagnostic_started_at = Instant::now();
            let backtrace = Backtrace::force_capture();
            log::warn!(
                target: LOG_TARGET,
                "检测到 UI 渲染线程长耗时任务：scope={} elapsed_ms={:.3} threshold_ms={} ui_thread={} thread_id={:?} thread_name={} suppressed_since_last={}\nstack:\n{}",
                self.name,
                elapsed.as_secs_f64() * 1_000.,
                self.warn_after.as_millis(),
                is_ui_thread,
                current_thread.id(),
                thread_name,
                suppressed_count,
                backtrace,
            );
            record_diagnostic_overhead(diagnostic_started_at.elapsed());
        }
    }

    fn aggregate(name: &'static str) -> AggregateGuard {
        ACTIVE_AGGREGATES.with(|aggregates| {
            aggregates.borrow_mut().push(ActiveAggregate {
                name,
                started_at: Instant::now(),
                diagnostic_overhead_at_start: diagnostic_overhead(),
                samples: HashMap::new(),
            });
        });
        AggregateGuard { name }
    }

    fn record_aggregate_sample(name: &'static str, elapsed: Duration) {
        ACTIVE_AGGREGATES.with(|aggregates| {
            for aggregate in aggregates.borrow_mut().iter_mut() {
                if aggregate.name == name {
                    continue;
                }
                let sample = aggregate.samples.entry(name).or_default();
                sample.count = sample.count.saturating_add(1);
                sample.total = sample.total.saturating_add(elapsed);
                sample.maximum = sample.maximum.max(elapsed);
            }
        });
    }

    impl Drop for AggregateGuard {
        fn drop(&mut self) {
            let aggregate = ACTIVE_AGGREGATES.with(|aggregates| aggregates.borrow_mut().pop());
            let Some(aggregate) = aggregate else {
                return;
            };
            debug_assert_eq!(aggregate.name, self.name);
            let elapsed = elapsed_without_diagnostics(
                aggregate.started_at,
                aggregate.diagnostic_overhead_at_start,
            );
            if elapsed < settings().warn_after
                || aggregate.samples.is_empty()
                || !log::log_enabled!(target: LOG_TARGET, log::Level::Warn)
            {
                return;
            }

            let diagnostic_started_at = Instant::now();
            let mut samples = aggregate.samples.into_iter().collect::<Vec<_>>();
            samples.sort_by_key(|(_, sample)| std::cmp::Reverse(sample.total));
            let contributors = samples
                .into_iter()
                .take(16)
                .map(|(name, sample)| {
                    format!(
                        "{}(count={}, total_ms={:.3}, max_ms={:.3})",
                        name,
                        sample.count,
                        sample.total.as_secs_f64() * 1_000.,
                        sample.maximum.as_secs_f64() * 1_000.,
                    )
                })
                .collect::<Vec<_>>()
                .join("; ");
            log::warn!(
                target: LOG_TARGET,
                "UI 渲染阶段耗时明细：scope={} elapsed_ms={:.3} contributors=[{}]",
                aggregate.name,
                elapsed.as_secs_f64() * 1_000.,
                contributors,
            );
            record_diagnostic_overhead(diagnostic_started_at.elapsed());
        }
    }

    fn diagnostic_overhead() -> Duration {
        DIAGNOSTIC_OVERHEAD.get()
    }

    fn record_diagnostic_overhead(elapsed: Duration) {
        DIAGNOSTIC_OVERHEAD.set(diagnostic_overhead().saturating_add(elapsed));
    }

    fn elapsed_without_diagnostics(
        started_at: Instant,
        diagnostic_overhead_at_start: Duration,
    ) -> Duration {
        let nested_diagnostic_overhead =
            diagnostic_overhead().saturating_sub(diagnostic_overhead_at_start);
        started_at
            .elapsed()
            .saturating_sub(nested_diagnostic_overhead)
    }

    fn settings() -> Settings {
        *SETTINGS.get_or_init(|| Settings {
            warn_after: Duration::from_millis(read_milliseconds(
                "VCLOGG2_UI_PERF_WARN_MS",
                DEFAULT_WARN_AFTER_MS,
            )),
            repeat_after: Duration::from_millis(read_milliseconds(
                "VCLOGG2_UI_PERF_REPEAT_MS",
                DEFAULT_REPEAT_AFTER_MS,
            )),
        })
    }

    fn read_milliseconds(name: &str, default: u64) -> u64 {
        std::env::var(name)
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| (1..=MAX_SETTING_MS).contains(value))
            .unwrap_or(default)
    }

    fn begin_log(name: &'static str, now: Instant) -> Option<u64> {
        let states = LOG_STATES.get_or_init(|| Mutex::new(LogStates::default()));
        let Ok(mut states) = states.lock() else {
            return Some(0);
        };
        let globally_limited = states
            .last_stack_at
            .is_some_and(|last_stack_at| now.duration_since(last_stack_at) < MIN_STACK_INTERVAL);
        let state = states.scopes.entry(name).or_default();
        let scope_limited = state.last_logged_at.is_some_and(|last_logged_at| {
            now.duration_since(last_logged_at) < settings().repeat_after
        });
        if globally_limited || scope_limited {
            state.suppressed_count = state.suppressed_count.saturating_add(1);
            return None;
        }

        state.last_logged_at = Some(now);
        let suppressed_count = std::mem::take(&mut state.suppressed_count);
        states.last_stack_at = Some(now);
        Some(suppressed_count)
    }
}

#[cfg(debug_assertions)]
pub(crate) use debug::{element, init_ui_thread, scope, start_framework_monitor};

#[cfg(not(debug_assertions))]
pub(crate) struct UiPerformanceScope;

#[cfg(not(debug_assertions))]
#[inline(always)]
pub(crate) fn init_ui_thread() {}

#[cfg(not(debug_assertions))]
#[inline(always)]
pub(crate) fn start_framework_monitor(_: &mut gpui::App) {}

#[cfg(not(debug_assertions))]
#[inline(always)]
pub(crate) fn element<E: gpui::IntoElement>(
    _: &'static str,
    _: &'static str,
    _: &'static str,
    child: E,
) -> E {
    child
}

#[cfg(not(debug_assertions))]
#[must_use]
#[inline(always)]
pub(crate) fn scope(_: &'static str) -> UiPerformanceScope {
    UiPerformanceScope
}
