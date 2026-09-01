use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet, HashSet, VecDeque},
    future::Future,
    ops::Range,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, SystemTime},
};

use anyhow::Result;
use chrono::{DateTime, Local};
use gpui::{
    Animation, AnimationExt as _, AnyElement, AnyWindowHandle, App, AppContext as _,
    AsyncWindowContext, BorrowAppContext as _, Bounds, ClickEvent, ClipboardItem, Context,
    DisplayId, DragMoveEvent, ElementId, Entity, ExternalPaths, FileDropEvent, FocusHandle,
    Focusable, FontWeight, Global, HighlightStyle, HitboxBehavior, InteractiveElement as _,
    IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    ParentElement as _, PathPromptOptions, Pixels, Point, Render, ScrollHandle, ScrollStrategy,
    ScrollWheelEvent, SharedString, Size, StatefulInteractiveElement as _, Styled as _, StyledText,
    Subscription, Task, TextStyle, UniformListScrollHandle, WeakEntity, Window, WindowId, canvas,
    deferred, div, point, prelude::FluentBuilder as _, px, relative, rems, size, svg, uniform_list,
};
use gpui_base::{
    GlobalState, POPUP_PRIORITY, Scrollbar, ScrollbarHandle, ScrollbarMode, TextSelection,
    TextSelectionScopeId,
    actions::{
        Cancel, Confirm, SelectDown, SelectFirst, SelectLast, SelectPageDown, SelectPageUp,
        SelectUp,
    },
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, ElementExt as _, FocusableExt as _, Icon, IconName,
    IndexPath, Root, Selectable as _, Side, Sizable as _, StyledExt as _, TitleBar, WindowExt as _,
    animation::ease_out_cubic,
    button::{Button, ButtonCustomVariant, ButtonRounded, ButtonVariant, ButtonVariants as _},
    checkbox::Checkbox,
    dialog::{DialogAction, DialogButtonProps, DialogClose, DialogFooter},
    h_flex,
    input::{Input, InputEvent, InputState},
    menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenu, PopupMenuItem},
    popover::Popover,
    resizable::{ResizableState, resizable_panel, v_resizable},
    scroll::ScrollableElement as _,
    select::{Select, SelectEvent, SelectState},
    status_bar::StatusBar,
    tab::{Tab, TabBar},
    table::{DataTable, TableDelegate, TableEvent, TableState},
    theme::ThemeMode,
    v_flex,
};
use rayon::prelude::{IntoParallelIterator as _, ParallelIterator as _};
use vclogg_core::{
    CompressedRows, LinePreviewReader, LineReader, LogDocument, PendingIndexCacheWrite,
    SearchCancellation, SearchMatcher, SearchQuery, SearchResult, SearchRun,
    search_with_compiled_matcher,
};

use crate::{
    actions::{
        CancelSearch, ClearSearch, CloseActiveTab, CopyCurrentLine, CopyCurrentLineWithNumber,
        CopyFilePath, CycleColorLabel, ExtendSelectionDown, ExtendSelectionFirst,
        ExtendSelectionLast, ExtendSelectionPageDown, ExtendSelectionPageUp, ExtendSelectionUp,
        FocusSearch, GoToLine, JumpToEnd, JumpToStart, LOG_TABLE_CONTEXT,
        MergeSearchResultsInNewTab, NewWindow, OpenFiles, OpenQuickFind, OpenSearchResultsInNewTab,
        OpenSettings, ReloadActive, SaveSearchResultsToFile, SelectAllRows, StartSearch,
        ToggleCaseSensitive, ToggleFullscreen, ToggleMarkedRow, ToggleRegex, ToggleWordWrap,
        WORKSPACE_CONTEXT,
    },
    cloud_filters::CloudClient,
    color_labels::{
        ColorLabel, KeywordColorRule, ResolvedColorRules, color_with_alpha, default_color_labels,
        resolve_color_rules,
    },
    color_labels_dialog::ColorLabelsDialog,
    directory_search_dialog::{
        DirectorySearchDialog, DirectorySearchOptions, enumerate_directory_search_paths,
    },
    global_search_files_dialog::{GlobalSearchFileOption, GlobalSearchFilesDialog},
    global_search_table::{
        GlobalGroupTogglePlan, GlobalSearchGroup, GlobalSearchGroupHeader, GlobalSearchRow,
        GlobalSearchTableDelegate, WrappedGlobalRow,
    },
    history_dialog::{HistoryDialog, HistoryDialogEvent},
    log_table::{
        LogTableCursor, LogTableDelegate, LogTableStateExt, TextHighlight, line_marker,
        line_marker_column_width, log_cell_horizontal_padding, log_fixed_column_divider_overlay,
        log_line_height, log_line_number_cell, log_row_selection_color, log_row_selection_overlay,
        log_row_separator_overlay, scroll_uniform_log_row_to_viewport_y, severity_accent_overlay,
        severity_style, text_highlight_style,
    },
    path_identity::{
        PathMatchKey, decode_persisted_path, deduplicate_paths, encode_persisted_path,
        path_buf_map_get, path_buf_map_insert, path_buf_map_remove, path_match_key,
        path_match_map_get, path_match_set_contains, paths_match,
    },
    predefined_filters::{PredefinedFilter, query_includes_filter, toggle_filter_in_query},
    predefined_filters_dialog::{PredefinedFiltersDialog, PredefinedFiltersDialogEvent},
    rename_tab_dialog::RenameTabDialog,
    result_export::{self, ExportGroup, ResultExport},
    search_autocomplete::{
        SearchSuggestion, SearchSuggestionSource, apply_search_suggestion,
        search_autocomplete_needle, search_autocomplete_suggestions,
    },
    search_context::{
        PersistedDirectorySearchOptions, PersistedGlobalSearchContext, PersistedPathSelection,
        PersistedSearchQuery, PersistedSearchRowKey, PersistedSearchScope, PersistedSearchViewport,
        WorkspaceSearchState,
    },
    selectable_log_text::{LogText, LogTextSelection, SelectableLogText, TextSelectionCache},
    settings_dialog::{
        SettingsCategory, SettingsDialog, SettingsDialogEvent, SettingsNetworkSnapshot,
    },
    sparse_virtual_list::{
        SparseListMeasurements, SparseVirtualListScrollHandle, prefix_height_for,
        row_for_absolute_y, sparse_v_virtual_list,
    },
    state_store::{
        AppSettings, CloudSettings, FileSessionState, LastWorkspaceFile, RecentFile,
        ShortcutSettings, StateStore, ThemePreference, normalize_search_history,
    },
    tab_resume::{PersistedLogRegion, TabResumeState, ViewportBookmark},
    ui_theme,
    virtual_log_lines::{LogRowKey, StagedVisibleLineLoadResult},
    workspace_state::{
        CloudController, GlobalSearchDocumentResult, GlobalSearchResults, GlobalSearchState,
        PersistenceController, QuickFindBoundary, QuickFindDirection, QuickFindMatch,
        QuickFindSource, QuickFindSourceVersion, QuickFindState, QuickFindTarget, ResultMode,
        RetainedGlobalSearchContext, RowViewportAnchor, SearchController, SearchScope,
        SearchTarget, ViewportAnchor,
    },
};

const WRAPPED_HEIGHT_CACHE_LIMIT: usize = 4096;
const PREVIEW_BYTE_LIMIT: usize = 1024 * 1024;
const PREVIEW_LINE_LIMIT: usize = 200;
const MAX_DOCUMENT_PREPARE_WORKERS: usize = 4;
const SEARCH_SUGGESTION_ROW_HEIGHT_REMS: f32 = 3.25;
const GITHUB_RELEASES_URL: &str = "https://github.com/zhaiyanqi/vclogg2/releases";
const SEARCH_SUGGESTION_MAX_VISIBLE_ROWS: usize = 5;
const SEARCH_CONTROL_HEIGHT: Pixels = px(34.);
const SEARCH_BAR_VERTICAL_INSET: Pixels = px(8.);
const EMPTY_WORKSPACE_CARD_HEADER_HEIGHT_REMS: f32 = 3.25;
const EMPTY_WORKSPACE_FILE_ROW_HEIGHT_REMS: f32 = 3.2;
const TRANSIENT_SURFACE_ENTER_DURATION: Duration = Duration::from_millis(160);
const FILE_WATCH_INTERVAL: Duration = Duration::from_millis(750);
static INDEX_CACHE_CLEANUP_SCHEDULED: AtomicBool = AtomicBool::new(false);
static PREDEFINED_FILTERS_SAVE_REVISION: AtomicU64 = AtomicU64::new(0);
static PREDEFINED_FILTERS_SAVE_LOCK: Mutex<()> = Mutex::new(());

fn save_predefined_filters_if_current(
    store: &StateStore,
    filters: &[PredefinedFilter],
    revision: u64,
) -> Result<bool> {
    let _guard = PREDEFINED_FILTERS_SAVE_LOCK.lock().map_err(|_| {
        anyhow::anyhow!(crate::tr!(
            "预定义过滤器保存锁已损坏",
            "The predefined-filter save lock is poisoned",
        ))
    })?;
    if revision != PREDEFINED_FILTERS_SAVE_REVISION.load(Ordering::Acquire) {
        return Ok(false);
    }
    store.save_predefined_filters(filters)?;
    Ok(true)
}

fn large_dialog_size(window: &Window) -> Size<Pixels> {
    size(
        (window.viewport_size().width - window.rem_size() * 2.).min(window.rem_size() * 64.),
        (window.viewport_size().height - window.rem_size() * 4.)
            .min(window.rem_size() * 40.)
            .max(window.rem_size() * 12.),
    )
}

fn centered_dialog_margin_top(viewport_height: Pixels, dialog_height: Pixels) -> Pixels {
    ((viewport_height - dialog_height) / 2.).max(px(0.))
}

fn management_dialog_geometry(window: &Window) -> (Size<Pixels>, Pixels) {
    let dialog_size = large_dialog_size(window);
    let margin_top = centered_dialog_margin_top(window.viewport_size().height, dialog_size.height);
    (dialog_size, margin_top)
}

fn persistent_log_scrollbar(scrollbar: Scrollbar, background: gpui::Hsla) -> Scrollbar {
    scrollbar
        .mode(ScrollbarMode::Always)
        .styles(|styles| styles.track(|track| track.bg(background)))
}

struct PreparedDocument {
    document: Arc<LogDocument>,
    session: Option<FileSessionState>,
    color_labels_snapshot: Option<Vec<ColorLabel>>,
    resolved_color_rules: Arc<ResolvedColorRules>,
    search_result: SearchResult,
    search_matcher: Option<SearchMatcher>,
    warning: Option<String>,
    load_state: DocumentLoadState,
    pending_index_cache: Option<PendingIndexCacheWrite>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DocumentLoadState {
    Opening,
    Preview,
    IndexFailed,
    Ready,
}

fn should_upgrade_loading_document(
    current: DocumentLoadState,
    next: DocumentLoadState,
    restores_session: bool,
) -> bool {
    if restores_session
        && current == DocumentLoadState::Opening
        && next == DocumentLoadState::Preview
    {
        return false;
    }

    matches!(
        (current, next),
        (DocumentLoadState::Opening, DocumentLoadState::Preview)
            | (DocumentLoadState::Opening, DocumentLoadState::Ready)
            | (DocumentLoadState::Preview, DocumentLoadState::Ready)
            | (DocumentLoadState::IndexFailed, DocumentLoadState::Preview)
            | (DocumentLoadState::IndexFailed, DocumentLoadState::Ready)
    )
}

#[derive(Default)]
struct OpenDocumentOverrides {
    sessions: BTreeMap<PathBuf, FileSessionState>,
    move_completions: BTreeMap<PathBuf, TabMoveCompletion>,
    target_indices: BTreeMap<PathBuf, usize>,
}

pub(crate) struct InitialDocument {
    path: PathBuf,
    session: Option<FileSessionState>,
    transient: bool,
    replace_new_tab: bool,
    move_completion: Option<TabMoveCompletion>,
    target_ix: Option<usize>,
}

impl InitialDocument {
    pub(crate) fn new(path: PathBuf, session: FileSessionState, transient: bool) -> Self {
        Self {
            path,
            session: Some(session),
            transient,
            replace_new_tab: false,
            move_completion: None,
            target_ix: None,
        }
    }

    pub(crate) fn from_path(path: PathBuf) -> Self {
        Self {
            path,
            session: None,
            transient: false,
            replace_new_tab: true,
            move_completion: None,
            target_ix: None,
        }
    }

    fn moving(
        path: PathBuf,
        session: FileSessionState,
        transient: bool,
        source: WeakEntity<Workspace>,
        source_window: AnyWindowHandle,
        document_id: u64,
    ) -> Self {
        Self {
            path,
            session: Some(session.clone()),
            transient,
            replace_new_tab: false,
            move_completion: Some(TabMoveCompletion {
                source,
                source_window,
                document_id,
                captured_state: session,
            }),
            target_ix: None,
        }
    }

    fn at_index(mut self, target_ix: usize) -> Self {
        self.target_ix = Some(target_ix);
        self
    }
}

struct TabMoveCompletion {
    source: WeakEntity<Workspace>,
    source_window: AnyWindowHandle,
    document_id: u64,
    captured_state: FileSessionState,
}

#[derive(Clone)]
struct RegisteredWorkspaceWindow {
    window: AnyWindowHandle,
    workspace: Entity<Workspace>,
    focus_order: u64,
}

#[derive(Default)]
struct WorkspaceWindowRegistry {
    windows: Vec<RegisteredWorkspaceWindow>,
    next_focus_order: u64,
    closed_flush_tasks: Vec<Task<()>>,
    predefined_filters: Option<Vec<PredefinedFilter>>,
    cross_window_tab_drag: Option<CrossWindowTabDrag>,
    last_settings_category: SettingsCategory,
    last_settings_category_loaded: bool,
}

#[derive(Clone)]
struct CrossWindowDropTarget {
    window: AnyWindowHandle,
    workspace: Entity<Workspace>,
    target_ix: usize,
}

struct TabTransferTarget {
    window: AnyWindowHandle,
    workspace: Entity<Workspace>,
    target_ix: Option<usize>,
}

struct CrossWindowTabDrag {
    source_window: AnyWindowHandle,
    source: WeakEntity<Workspace>,
    document_id: u64,
    mode: TabTransferMode,
    target: Option<CrossWindowDropTarget>,
    over_workspace_window: bool,
}

impl Global for WorkspaceWindowRegistry {}

impl WorkspaceWindowRegistry {
    fn register(&mut self, window: AnyWindowHandle, workspace: Entity<Workspace>) {
        self.next_focus_order += 1;
        self.windows.retain(|entry| entry.window != window);
        self.windows.push(RegisteredWorkspaceWindow {
            window,
            workspace,
            focus_order: self.next_focus_order,
        });
    }

    fn mark_focused(&mut self, window: AnyWindowHandle) {
        self.next_focus_order += 1;
        if let Some(entry) = self.windows.iter_mut().find(|entry| entry.window == window) {
            entry.focus_order = self.next_focus_order;
        }
    }

    fn unregister(&mut self, window_id: WindowId) -> Option<Entity<Workspace>> {
        let ix = self
            .windows
            .iter()
            .position(|entry| entry.window.window_id() == window_id)?;
        Some(self.windows.remove(ix).workspace)
    }

    fn previous_window(&self, source: AnyWindowHandle) -> Option<RegisteredWorkspaceWindow> {
        self.windows
            .iter()
            .filter(|entry| entry.window != source)
            .max_by_key(|entry| entry.focus_order)
            .cloned()
    }

    fn windows_by_recent_focus(&self) -> Vec<RegisteredWorkspaceWindow> {
        let mut windows = self.windows.clone();
        windows.sort_by_key(|entry| std::cmp::Reverse(entry.focus_order));
        windows
    }
}

struct QuitWorkspaceSnapshot {
    store: Option<Arc<StateStore>>,
    predefined_filters: Option<Vec<PredefinedFilter>>,
    predefined_filters_revision: u64,
    sessions: Vec<(PathBuf, FileSessionState)>,
    open_paths: Vec<PathBuf>,
    active_path: Option<PathBuf>,
    search_state: Option<WorkspaceSearchState>,
    state_tasks: Vec<Task<()>>,
    workspace_order_task: Option<Task<()>>,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum TabTransferMode {
    Copy,
    Move,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum WorkspaceTabId {
    Document(u64),
    New(u64),
}

impl WorkspaceTabId {
    fn document_id(self) -> Option<u64> {
        match self {
            Self::Document(id) => Some(id),
            Self::New(_) => None,
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum TabTransferReception {
    Accepted,
    AlreadyOpen,
    Busy,
    Closed,
}

#[derive(Clone, Copy)]
struct TabMenuState {
    tab_ix: usize,
    tab_count: usize,
    can_restore_title: bool,
    has_other_window: bool,
}

#[derive(IntoElement)]
struct TitleBarMenuButton {
    button: Button,
}

impl gpui::RenderOnce for TitleBarMenuButton {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        self.button
    }
}

impl gpui::Styled for TitleBarMenuButton {
    fn style(&mut self) -> &mut gpui::StyleRefinement {
        self.button.style()
    }
}

impl gpui::InteractiveElement for TitleBarMenuButton {
    fn interactivity(&mut self) -> &mut gpui::Interactivity {
        self.button.interactivity()
    }
}

impl gpui::StatefulInteractiveElement for TitleBarMenuButton {}

impl gpui_component::Selectable for TitleBarMenuButton {
    fn selected(mut self, selected: bool) -> Self {
        self.button = self.button.selected(selected);
        self
    }

    fn is_selected(&self) -> bool {
        self.button.is_selected()
    }
}

impl gpui_component::Disableable for TitleBarMenuButton {
    fn disabled(mut self, disabled: bool) -> Self {
        self.button = self.button.disabled(disabled);
        self
    }
}

impl gpui_component::menu::DropdownMenu for TitleBarMenuButton {}

struct LogContextMenuContext {
    selected_text: String,
    include_results: bool,
    include_global_merge: bool,
    export_disabled: bool,
}

impl TabMoveCompletion {
    fn finish(self, installed: bool, cx: &mut App) {
        _ = self.source_window.update(cx, move |_, window, cx| {
            let Some(source) = self.source.upgrade() else {
                return;
            };
            source.update(cx, |source, cx| {
                source.pending_tab_moves.remove(&self.document_id);
                if !installed {
                    window.push_notification(
                        crate::tr!(
                            "标签未能移动，源标签已保留",
                            "The tab couldn’t be moved, so the source tab was kept",
                        ),
                        cx,
                    );
                    return;
                }
                let unchanged = source
                    .documents
                    .iter()
                    .find(|tab| tab.id == self.document_id)
                    .is_some_and(|tab| {
                        Workspace::session_contents_equal(
                            &source.file_session_state(tab, cx),
                            &self.captured_state,
                        )
                    });
                if unchanged {
                    source.close_tab_by_id(self.document_id, window, cx);
                } else {
                    window.push_notification(
                        crate::tr!(
                            "目标窗口已打开副本；源标签在传输期间发生变化，因此保留",
                            "The destination opened a copy. The source tab changed during transfer and was kept.",
                        ),
                        cx,
                    );
                }
            });
        });
    }
}

#[derive(Default)]
struct TabDropLayout {
    tabs: Vec<Bounds<Pixels>>,
    end: Bounds<Pixels>,
}

impl TabDropLayout {
    fn drop_index(&self, position: Point<Pixels>) -> Option<usize> {
        for (ix, bounds) in self.tabs.iter().enumerate() {
            if bounds.contains(&position) {
                let midpoint = bounds.origin.x + bounds.size.width * 0.5;
                return Some(if position.x < midpoint { ix } else { ix + 1 });
            }
        }
        self.end.contains(&position).then_some(self.tabs.len())
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum SelectionTable {
    #[default]
    Log,
    Results,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum LogRegion {
    #[default]
    Body,
    CurrentResults,
    GlobalResults,
}

fn restored_results_visible(persisted: bool, result_mode: ResultMode, has_marks: bool) -> bool {
    persisted || (result_mode.includes_marks() && has_marks)
}

fn restored_selection_table(
    active_region: PersistedLogRegion,
    results_visible: bool,
) -> SelectionTable {
    match active_region {
        PersistedLogRegion::CurrentResults if results_visible => SelectionTable::Results,
        PersistedLogRegion::Body | PersistedLogRegion::CurrentResults => SelectionTable::Log,
    }
}

fn restored_log_region(active_region: PersistedLogRegion, results_visible: bool) -> LogRegion {
    match restored_selection_table(active_region, results_visible) {
        SelectionTable::Log => LogRegion::Body,
        SelectionTable::Results => LogRegion::CurrentResults,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct RowViewportPosition {
    row_ix: usize,
    viewport_y: Pixels,
}

struct DocumentTab {
    id: u64,
    opened_at: i64,
    title: SharedString,
    custom_title: Option<String>,
    document: Arc<LogDocument>,
    session_base: FileSessionState,
    log_table: Entity<TableState<LogTableDelegate>>,
    result_table: Entity<TableState<LogTableDelegate>>,
    log_surface: Entity<LogRegionSurface>,
    result_surface: Entity<LogRegionSurface>,
    log_viewport: LogViewportState<usize>,
    result_viewport: LogViewportState<usize>,
    search_query: SearchQuery,
    search_result: SearchResult,
    search_matcher: Option<SearchMatcher>,
    result_mode: ResultMode,
    result_mode_select: Entity<SelectState<Vec<ResultMode>>>,
    search_revision: u64,
    log_jump_revision: u64,
    log_jump_task: Option<Task<()>>,
    results_visible: bool,
    restoring_result_selection: bool,
    marked_rows: CompressedRows,
    pending_restore_marked_rows: CompressedRows,
    keyword_color_rules: Vec<KeywordColorRule>,
    resolved_color_rules: Arc<ResolvedColorRules>,
    log_text_selection_scope: TextSelectionScopeId,
    result_text_selection_scope: TextSelectionScopeId,
    log_focus_handle: FocusHandle,
    result_focus_handle: FocusHandle,
    auto_follow: bool,
    show_line_numbers: bool,
    show_row_separators: bool,
    selection_table: SelectionTable,
    uses_default_view_options: bool,
    load_state: DocumentLoadState,
    pending_restore_row: Option<usize>,
    pending_resume: Option<TabResumeState>,
}

struct PreparedTabFrame {
    document_id: u64,
    document: Arc<LogDocument>,
    log_revision: u64,
    result_revision: u64,
    log_lines: Option<StagedVisibleLineLoadResult<usize>>,
    result_lines: Option<StagedVisibleLineLoadResult<usize>>,
}

struct PreparedGlobalGroupToggle {
    plan: GlobalGroupTogglePlan,
    staged: Option<StagedVisibleLineLoadResult<(u64, usize)>>,
    anchor: Option<RowViewportAnchor<LogRowKey>>,
    measured_heights: BTreeMap<LogRowKey, Pixels>,
    row_height: Pixels,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum WrappedRegion {
    Log,
    Results,
    GlobalResults,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum LogScrollFrameTarget {
    /// The fixed-height logical coordinate exposed by the scrollbar. Wrapped rows map this
    /// coordinate to their measured physical height only when the prepared frame is committed.
    Scrollbar(Point<Pixels>),
    /// The physical viewport coordinate used by wheel and trackpad scrolling.
    Viewport(Point<Pixels>),
}

impl LogScrollFrameTarget {
    fn offset(self) -> Point<Pixels> {
        match self {
            Self::Scrollbar(offset) | Self::Viewport(offset) => offset,
        }
    }
}

#[derive(Default)]
struct PendingLogScrollFrames {
    targets: BTreeMap<(u64, WrappedRegion), LogScrollFrameTarget>,
}

impl PendingLogScrollFrames {
    fn request(&mut self, key: (u64, WrappedRegion), target: LogScrollFrameTarget) {
        // Input can arrive faster than GPUI paints. Like klogg's update()/paintEvent path, keep
        // only the last position and prepare exactly that viewport on the next frame.
        self.targets.insert(key, target);
    }

    fn latest(&self, key: (u64, WrappedRegion)) -> Option<LogScrollFrameTarget> {
        self.targets.get(&key).copied()
    }

    fn take(&mut self, key: (u64, WrappedRegion)) -> Option<LogScrollFrameTarget> {
        self.targets.remove(&key)
    }

    fn clear(&mut self, key: (u64, WrappedRegion)) {
        self.targets.remove(&key);
    }
}

struct WrappedListState<K> {
    item_count: Rc<Cell<usize>>,
    base_height: Rc<Cell<Pixels>>,
    measured_heights: Rc<RefCell<BTreeMap<usize, Pixels>>>,
    pending_heights: RefCell<BTreeMap<usize, Pixels>>,
    measured_rows: RefCell<VecDeque<usize>>,
    height_corrections: Rc<RefCell<Vec<(usize, Pixels)>>>,
    scroll_handle: SparseVirtualListScrollHandle,
    text_selections: RefCell<TextSelectionCache<K>>,
    measurement_anchor: Rc<Cell<Option<RowViewportPosition>>>,
    pending_scrollbar_offset: Rc<Cell<Option<Point<Pixels>>>>,
    layout_key: RefCell<Option<WrappedLayoutKey>>,
    row_bounds: Rc<RefCell<BTreeMap<usize, Bounds<Pixels>>>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LogViewportMode {
    Fixed,
    Wrapped,
}

#[derive(Clone, Copy)]
struct WrappedFramePrimeOptions {
    minimum_viewport_height: Pixels,
    reset_for_mode_switch: bool,
}

#[derive(Clone, Copy)]
struct LogWheelScrollRequest {
    delta_y: Pixels,
    row_count: usize,
    row_height: Pixels,
    line_count: usize,
    line_scroll: bool,
    scale: f32,
}

#[derive(Clone)]
struct FixedListState {
    scroll_handle: UniformListScrollHandle,
    pending_scrollbar_offset: Rc<Cell<Option<Point<Pixels>>>>,
    row_bounds: Rc<RefCell<BTreeMap<usize, Bounds<Pixels>>>>,
}

struct LogViewportState<K> {
    mode: Cell<LogViewportMode>,
    fixed: FixedListState,
    wrapped: WrappedListState<K>,
}

impl<K: Clone + Ord> LogViewportState<K> {
    fn new(
        word_wrap: bool,
        scroll_handle: UniformListScrollHandle,
        row_bounds: Rc<RefCell<BTreeMap<usize, Bounds<Pixels>>>>,
    ) -> Self {
        Self {
            mode: Cell::new(if word_wrap {
                LogViewportMode::Wrapped
            } else {
                LogViewportMode::Fixed
            }),
            fixed: FixedListState {
                scroll_handle,
                pending_scrollbar_offset: Rc::default(),
                row_bounds,
            },
            wrapped: WrappedListState::default(),
        }
    }

    fn is_wrapped(&self) -> bool {
        self.mode.get() == LogViewportMode::Wrapped
    }

    fn set_word_wrap(&self, enabled: bool) {
        self.fixed.pending_scrollbar_offset.set(None);
        self.wrapped.pending_scrollbar_offset.set(None);
        self.mode.set(if enabled {
            LogViewportMode::Wrapped
        } else {
            LogViewportMode::Fixed
        });
    }

    fn capture_viewport_position(
        &self,
        row_count: usize,
        preferred_row: Option<usize>,
        row_height: Pixels,
    ) -> Option<RowViewportPosition> {
        if self.is_wrapped() {
            return self.wrapped.capture_row_viewport_position(preferred_row);
        }
        if row_count == 0 {
            return None;
        }
        let base = self.fixed.scroll_handle.0.borrow().base_handle.clone();
        let top = (-base.offset().y).max(px(0.));
        let first = (top / row_height.max(px(1.))).floor().max(0.) as usize;
        let viewport_height = base.bounds().size.height;
        let row_ix = viewport_anchor_row(row_count, first, preferred_row, |row_ix| {
            let viewport_y = row_height * row_ix as f32 + base.offset().y;
            row_intersects_viewport(viewport_y, row_height, viewport_height)
        });
        Some(RowViewportPosition {
            row_ix,
            viewport_y: row_height * row_ix as f32 + base.offset().y,
        })
    }

    fn is_at_end(&self) -> bool {
        let (top, max) = if self.is_wrapped() {
            (
                (-self.wrapped.scroll_handle.offset().y).max(px(0.)),
                self.wrapped.scroll_handle.max_offset().y.max(px(0.)),
            )
        } else {
            let base = self.fixed.scroll_handle.0.borrow().base_handle.clone();
            (
                (-base.offset().y).max(px(0.)),
                base.max_offset().y.max(px(0.)),
            )
        };
        max > px(0.) && top >= max - px(0.5)
    }

    fn restore_viewport(
        &self,
        row_ix: usize,
        viewport_y: Pixels,
        at_end: bool,
        row_height: Pixels,
    ) {
        if at_end {
            self.scroll_to_end();
        } else if self.is_wrapped() {
            self.wrapped.scroll_row_to_viewport_y(row_ix, viewport_y);
        } else {
            scroll_uniform_log_row_to_viewport_y(
                &self.fixed.scroll_handle,
                row_ix,
                viewport_y,
                row_height,
            );
        }
    }

    fn scroll_to_end(&self) {
        if self.is_wrapped() {
            self.wrapped.scroll_to_end();
        } else {
            self.fixed.scroll_handle.scroll_to_bottom();
        }
    }

    fn center_row(&self, row_ix: usize) {
        if self.is_wrapped() {
            self.wrapped.center_row(row_ix);
        } else {
            self.fixed
                .scroll_handle
                .scroll_to_item_strict(row_ix, ScrollStrategy::Center);
        }
    }

    fn first_visible(&self, row_count: usize, row_height: Pixels) -> usize {
        if self.is_wrapped() {
            self.wrapped.first_visible_row()
        } else if row_count == 0 {
            0
        } else {
            let top = (-self.fixed.scroll_handle.0.borrow().base_handle.offset().y).max(px(0.));
            ((top / row_height.max(px(1.))).floor().max(0.) as usize)
                .min(row_count.saturating_sub(1))
        }
    }

    fn prospective_wrapped_measurement_range(
        &self,
        row_count: usize,
        viewport_height: Pixels,
        row_height: Pixels,
    ) -> Range<usize> {
        wrapped_viewport_measurement_range(
            self.first_visible(row_count, row_height),
            viewport_height,
            row_height,
            row_count,
        )
    }

    fn row_at_position(&self, position: Point<Pixels>) -> Option<usize> {
        let bounds = if self.is_wrapped() {
            &self.wrapped.row_bounds
        } else {
            &self.fixed.row_bounds
        };
        bounds
            .borrow()
            .iter()
            .find_map(|(row_ix, bounds)| bounds.contains(&position).then_some(*row_ix))
    }

    fn visible_row_edge(&self, after: bool) -> Option<usize> {
        let bounds = if self.is_wrapped() {
            self.wrapped.row_bounds.borrow()
        } else {
            self.fixed.row_bounds.borrow()
        };
        if after {
            bounds.keys().next_back().copied()
        } else {
            bounds.keys().next().copied()
        }
    }

    fn place_at_top(&self, row_ix: usize, row_height: Pixels) {
        if self.is_wrapped() {
            self.wrapped.place_row_at_top(row_ix);
        } else {
            scroll_uniform_log_row_to_viewport_y(
                &self.fixed.scroll_handle,
                row_ix,
                px(0.),
                row_height,
            );
        }
    }

    fn page_size(&self, fixed_visible_rows: usize, base_height: Pixels) -> usize {
        if self.is_wrapped() {
            (self.wrapped.scroll_handle.bounds().size.height / base_height)
                .floor()
                .max(1.) as usize
        } else {
            fixed_visible_rows.max(1)
        }
    }

    fn reveal_row(&self, row_ix: usize, strategy: ScrollStrategy) {
        if self.is_wrapped() {
            self.wrapped.scroll_handle.scroll_to_item(row_ix, strategy);
        } else {
            self.fixed
                .scroll_handle
                .scroll_to_item_strict(row_ix, strategy);
        }
    }

    fn wheel_scroll_target(
        &self,
        current: Point<Pixels>,
        request: LogWheelScrollRequest,
    ) -> Option<Point<Pixels>> {
        let LogWheelScrollRequest {
            delta_y,
            row_count,
            row_height,
            line_count,
            line_scroll,
            scale,
        } = request;
        if row_count == 0 || delta_y == px(0.) {
            return None;
        }
        if self.is_wrapped() {
            let max_y = self.wrapped.scroll_handle.max_offset().y.max(px(0.));
            let target_y = if line_scroll {
                let current_top = (-current.y).clamp(px(0.), max_y);
                let corrections = self.wrapped.height_corrections.borrow();
                let current_row =
                    row_for_absolute_y(row_count, row_height, &corrections, current_top);
                let target_row = if delta_y < px(0.) {
                    current_row.saturating_add(line_count)
                } else {
                    current_row.saturating_sub(line_count)
                }
                .min(row_count.saturating_sub(1));
                -prefix_height_for(row_height, &corrections, target_row).min(max_y)
            } else {
                (current.y + delta_y * scale).clamp(-max_y, px(0.))
            };
            return (target_y != current.y).then_some(point(current.x, target_y));
        }

        if row_height <= px(0.) {
            return None;
        }
        let base = self.fixed.scroll_handle.0.borrow().base_handle.clone();
        let max_y = base.max_offset().y.max(px(0.));
        let target_y = if line_scroll {
            let current_top = (-current.y).clamp(px(0.), max_y);
            let current_row = row_for_absolute_y(row_count, row_height, &[], current_top);
            let target_row = if delta_y < px(0.) {
                current_row.saturating_add(line_count)
            } else {
                current_row.saturating_sub(line_count)
            }
            .min(row_count.saturating_sub(1));
            -(row_height * target_row as f32).min(max_y)
        } else {
            (current.y + delta_y * scale).clamp(-max_y, px(0.))
        };
        (target_y != current.y).then_some(point(current.x, target_y))
    }

    fn horizontal_offset(&self) -> Pixels {
        if self.is_wrapped() {
            px(0.)
        } else {
            let base = self.fixed.scroll_handle.0.borrow().base_handle.clone();
            (-base.offset().x).max(px(0.))
        }
    }

    fn set_horizontal_offset(&self, offset: Pixels) {
        if self.is_wrapped() {
            return;
        }
        let base = self.fixed.scroll_handle.0.borrow().base_handle.clone();
        base.set_offset(point(-offset, base.offset().y));
    }

    fn wrapped_base_height(&self) -> Pixels {
        self.wrapped.base_height.get()
    }

    fn wrapped_viewport_height(&self) -> Pixels {
        self.wrapped.scroll_handle.bounds().size.height
    }

    fn committed_viewport_height(&self) -> Pixels {
        if self.is_wrapped() {
            self.wrapped.scroll_handle.bounds().size.height
        } else {
            self.fixed
                .scroll_handle
                .0
                .borrow()
                .base_handle
                .bounds()
                .size
                .height
        }
    }

    fn wrapped_scroll_handle(&self) -> SparseVirtualListScrollHandle {
        self.wrapped.scroll_handle.clone()
    }

    fn wrapped_row_bounds(&self) -> Rc<RefCell<BTreeMap<usize, Bounds<Pixels>>>> {
        self.wrapped.row_bounds.clone()
    }

    fn wrapped_selection(
        &self,
        key: K,
        text: &LogText,
        window: &Window,
        cx: &mut App,
    ) -> LogTextSelection {
        self.wrapped
            .text_selections
            .borrow_mut()
            .handle(key, text, window, cx)
    }

    fn wrapped_sizes(&self, count: usize, base_height: Pixels) -> SparseListMeasurements {
        self.wrapped.sizes(count, base_height)
    }

    fn wrapped_logical_scroll_handle(
        &self,
        item_count: usize,
        slot_height: Pixels,
    ) -> LogicalVirtualScrollHandle {
        self.wrapped.logical_scroll_handle(item_count, slot_height)
    }

    fn atomic_fixed_scroll_handle(&self) -> AtomicUniformScrollHandle {
        AtomicUniformScrollHandle {
            handle: self.fixed.scroll_handle.clone(),
            pending_offset: self.fixed.pending_scrollbar_offset.clone(),
        }
    }

    fn take_pending_scrollbar_offset(&self) -> Option<Point<Pixels>> {
        if self.is_wrapped() {
            self.wrapped.pending_scrollbar_offset.take()
        } else {
            self.fixed.pending_scrollbar_offset.take()
        }
    }

    fn committed_scroll_offset(&self) -> Point<Pixels> {
        if self.is_wrapped() {
            self.wrapped.scroll_handle.offset()
        } else {
            self.fixed.scroll_handle.0.borrow().base_handle.offset()
        }
    }

    fn viewport_offset_for_target(
        &self,
        target: LogScrollFrameTarget,
        item_count: usize,
        slot_height: Pixels,
    ) -> Point<Pixels> {
        match target {
            LogScrollFrameTarget::Viewport(offset) => offset,
            LogScrollFrameTarget::Scrollbar(offset) if self.is_wrapped() => self
                .wrapped
                .viewport_offset_for_logical_scrollbar_offset(offset, item_count, slot_height),
            LogScrollFrameTarget::Scrollbar(offset) => offset,
        }
    }

    fn scroll_frame_preload_range(
        &self,
        target: LogScrollFrameTarget,
        row_count: usize,
        viewport_height: Pixels,
        row_height: Pixels,
    ) -> Range<usize> {
        if !self.is_wrapped() || matches!(target, LogScrollFrameTarget::Scrollbar(_)) {
            return scrollbar_preload_range(
                target.offset(),
                row_count,
                viewport_height,
                row_height,
            );
        }
        if row_count == 0 {
            return 0..0;
        }

        let top = (-target.offset().y).clamp(
            px(0.),
            self.wrapped.scroll_handle.max_offset().y.max(px(0.)),
        );
        let first = row_for_absolute_y(
            row_count,
            row_height,
            &self.wrapped.height_corrections.borrow(),
            top,
        )
        .min(row_count.saturating_sub(1));
        let visible_count = (viewport_height / row_height.max(px(1.))).ceil().max(1.) as usize;
        first.saturating_sub(2)
            ..first
                .saturating_add(visible_count)
                .saturating_add(2)
                .min(row_count)
    }

    fn commit_scroll_frame_target(
        &self,
        target: LogScrollFrameTarget,
        item_count: usize,
        slot_height: Pixels,
    ) {
        match target {
            LogScrollFrameTarget::Scrollbar(offset) if self.is_wrapped() => self
                .wrapped
                .apply_logical_scrollbar_offset(offset, item_count, slot_height),
            LogScrollFrameTarget::Viewport(offset) if self.is_wrapped() => {
                self.wrapped.clear_measurement_anchor();
                let max_y = self.wrapped.scroll_handle.max_offset().y.max(px(0.));
                self.wrapped.scroll_handle.set_offset(point(
                    self.wrapped.scroll_handle.offset().x,
                    offset.y.clamp(-max_y, px(0.)),
                ));
            }
            LogScrollFrameTarget::Scrollbar(offset) | LogScrollFrameTarget::Viewport(offset) => {
                let base = self.fixed.scroll_handle.0.borrow().base_handle.clone();
                let max_y = base.max_offset().y.max(px(0.));
                base.set_offset(point(base.offset().x, offset.y.clamp(-max_y, px(0.))));
            }
        }
    }

    fn queue_wrapped_measured_height(
        &self,
        row_ix: usize,
        height: Pixels,
        base_height: Pixels,
    ) -> bool {
        self.wrapped
            .queue_measured_height(row_ix, height, base_height)
    }

    fn effective_row_height(&self, row_ix: usize, base_height: Pixels) -> Pixels {
        if self.is_wrapped() {
            self.wrapped.row_height(row_ix).unwrap_or(base_height)
        } else {
            base_height
        }
    }

    fn has_known_wrapped_row_height(&self, row_ix: usize) -> bool {
        self.wrapped.has_known_row_height(row_ix)
    }

    fn prime_wrapped_measured_heights(
        &self,
        count: usize,
        base_height: Pixels,
        heights: impl IntoIterator<Item = (usize, Pixels)>,
    ) {
        self.wrapped
            .prime_measured_heights(count, base_height, heights);
    }

    fn wrapped_measured_heights_by_key<T: Ord>(
        &self,
        key_for_row: impl Fn(usize) -> Option<T>,
    ) -> BTreeMap<T, Pixels> {
        self.wrapped.measured_heights_by_key(key_for_row)
    }

    fn reset_wrapped_with_remapped_heights<T: Ord>(
        &mut self,
        count: usize,
        base_height: Pixels,
        measured_heights: BTreeMap<T, Pixels>,
        row_for_key: impl Fn(&T) -> Option<usize>,
    ) {
        self.wrapped
            .reset_with_remapped_heights(count, base_height, measured_heights, row_for_key);
    }

    fn invalidate_wrapped(&mut self) {
        self.wrapped.invalidate();
    }

    fn reset_wrapped_scroll_for_mode_switch(&mut self) {
        self.wrapped.reset_scroll_for_mode_switch();
    }

    fn wrapped_layout_width(&self) -> Option<Pixels> {
        self.wrapped.layout_width()
    }

    fn capture_wrapped_viewport_position(
        &self,
        preferred_row: Option<usize>,
    ) -> Option<RowViewportPosition> {
        self.wrapped.capture_row_viewport_position(preferred_row)
    }

    fn ensure_wrapped_measurement_anchor(&self, preferred_row: Option<usize>) {
        if self.wrapped.measurement_anchor.get().is_none()
            && let Some(anchor) = self.capture_wrapped_viewport_position(preferred_row)
        {
            self.wrapped.measurement_anchor.set(Some(anchor));
        }
    }

    fn invalidate_wrapped_layout_preserving_position(
        &self,
        key: WrappedLayoutKey,
        preferred_row: Option<usize>,
    ) -> bool {
        if !self.wrapped.needs_layout_invalidation(&key) {
            return false;
        }
        self.ensure_wrapped_measurement_anchor(preferred_row);
        self.wrapped.invalidate_for_layout(key)
    }

    fn retain_wrapped_visible_rows(&self, visible_range: &Range<usize>) {
        self.wrapped.retain_visible_rows(visible_range);
    }

    fn wrapped_first_visible_row(&self) -> usize {
        self.wrapped.first_visible_row()
    }
}

#[derive(Clone, Debug)]
struct WrappedLayoutKey {
    content_revision: u64,
    width: Pixels,
    rem_size: Pixels,
    font_family: SharedString,
    font_size: u16,
    base_height: Pixels,
    horizontal_padding: Pixels,
}

impl WrappedLayoutKey {
    fn is_equivalent_to(&self, other: &Self) -> bool {
        self.content_revision == other.content_revision
            && (self.width - other.width).abs() < px(0.5)
            && self.rem_size == other.rem_size
            && self.font_family == other.font_family
            && self.font_size == other.font_size
            && self.base_height == other.base_height
            && self.horizontal_padding == other.horizontal_padding
    }
}

#[derive(Clone, Copy)]
struct RowDragSelection {
    document_id: u64,
    region: WrappedRegion,
    pointer: Point<Pixels>,
    start_row: usize,
    target_row: usize,
    mode: RowDragMode,
}

impl RowDragSelection {
    fn changed_row_selection(self) -> bool {
        self.mode == RowDragMode::Lines && self.target_row != self.start_row
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RowDragMode {
    Text,
    Lines,
}

struct LogRegionSurface {
    workspace: WeakEntity<Workspace>,
    document_id: u64,
    region: WrappedRegion,
    _table_subscription: Subscription,
}

struct WorkspaceStatusSurface {
    workspace: WeakEntity<Workspace>,
    _workspace_subscription: Subscription,
}

impl WorkspaceStatusSurface {
    fn new(workspace: WeakEntity<Workspace>, cx: &mut Context<Self>) -> Self {
        let workspace_entity = workspace
            .upgrade()
            .expect("workspace is alive while creating its status surface");
        let workspace_subscription = cx.observe(&workspace_entity, |_, _, cx| cx.notify());
        Self {
            workspace,
            _workspace_subscription: workspace_subscription,
        }
    }
}

impl LogRegionSurface {
    fn new<D>(
        workspace: WeakEntity<Workspace>,
        document_id: u64,
        region: WrappedRegion,
        table: &Entity<TableState<D>>,
        cx: &mut Context<Self>,
    ) -> Self
    where
        D: TableDelegate + 'static,
    {
        let table_subscription = cx.observe(table, |_, _, cx| cx.notify());
        Self {
            workspace,
            document_id,
            region,
            _table_subscription: table_subscription,
        }
    }
}

#[derive(Clone)]
struct LogicalVirtualScrollHandle {
    handle: SparseVirtualListScrollHandle,
    measured_heights: Rc<RefCell<BTreeMap<usize, Pixels>>>,
    height_corrections: Rc<RefCell<Vec<(usize, Pixels)>>>,
    pending_offset: Rc<Cell<Option<Point<Pixels>>>>,
    item_count: usize,
    slot_height: Pixels,
}

impl ScrollbarHandle for LogicalVirtualScrollHandle {
    fn viewport_bounds(&self) -> Bounds<Pixels> {
        self.handle.bounds()
    }

    fn offset(&self) -> Point<Pixels> {
        let viewport_height = self.handle.bounds().size.height;
        let logical_height = self.slot_height * self.item_count as f32;
        let logical_max = (logical_height - viewport_height).max(px(0.));
        let actual_max = self.handle.max_offset().y.max(px(0.));
        let actual_top = (-self.handle.offset().y).clamp(px(0.), actual_max);
        let logical_top = if actual_top >= actual_max - px(0.5) {
            logical_max
        } else {
            let corrections = self.height_corrections.borrow();
            let row =
                row_for_absolute_y(self.item_count, self.slot_height, &corrections, actual_top);
            let actual_row_top = prefix_height_for(self.slot_height, &corrections, row);
            let actual_row_height = self
                .measured_heights
                .borrow()
                .get(&row)
                .copied()
                .unwrap_or(self.slot_height)
                .max(self.slot_height);
            let fraction = ((actual_top - actual_row_top) / actual_row_height).clamp(0., 1.);
            (self.slot_height * row as f32 + self.slot_height * fraction).clamp(px(0.), logical_max)
        };
        point(self.handle.offset().x, -logical_top)
    }

    fn set_offset(&self, offset: Point<Pixels>) {
        self.pending_offset.set(Some(offset));
    }

    fn content_size(&self) -> Size<Pixels> {
        size(
            self.handle.bounds().size.width,
            self.slot_height * self.item_count as f32,
        )
    }
}

#[derive(Clone)]
struct AtomicUniformScrollHandle {
    handle: UniformListScrollHandle,
    pending_offset: Rc<Cell<Option<Point<Pixels>>>>,
}

impl ScrollbarHandle for AtomicUniformScrollHandle {
    fn viewport_bounds(&self) -> Bounds<Pixels> {
        self.handle.0.borrow().base_handle.bounds()
    }

    fn offset(&self) -> Point<Pixels> {
        self.handle.0.borrow().base_handle.offset()
    }

    fn set_offset(&self, offset: Point<Pixels>) {
        self.pending_offset.set(Some(offset));
    }

    fn content_size(&self) -> Size<Pixels> {
        let state = self.handle.0.borrow();
        let base = &state.base_handle;
        (base.max_offset() + base.bounds().size.into()).into()
    }
}

fn centered_scroll_top(
    row_top: Pixels,
    row_height: Pixels,
    viewport_height: Pixels,
    max_top: Pixels,
) -> Pixels {
    (row_top + row_height / 2. - viewport_height / 2.).clamp(px(0.), max_top.max(px(0.)))
}

fn centered_log_jump_preload_range(
    target_row: usize,
    row_count: usize,
    visible_row_count: usize,
) -> Range<usize> {
    if row_count == 0 {
        return 0..0;
    }
    // Loading three viewports absorbs small fixed/wrapped measurement differences when the
    // destination is first laid out, without approaching the cache's normal byte budget.
    let preload_count = visible_row_count.max(1).saturating_mul(3).min(row_count);
    let target_row = target_row.min(row_count - 1);
    let start = target_row
        .saturating_sub(preload_count / 2)
        .min(row_count - preload_count);
    start..start + preload_count
}

fn viewport_anchor_row(
    count: usize,
    first_visible: usize,
    selected: Option<usize>,
    selected_is_visible: impl FnOnce(usize) -> bool,
) -> usize {
    selected
        .filter(|row_ix| selected_is_visible(*row_ix))
        .unwrap_or(first_visible)
        .min(count.saturating_sub(1))
}

fn row_intersects_viewport(
    viewport_y: Pixels,
    row_height: Pixels,
    viewport_height: Pixels,
) -> bool {
    viewport_y + row_height > px(0.) && viewport_y < viewport_height
}

fn point_in_text_selection_regions(
    position: Point<Pixels>,
    regions: impl IntoIterator<Item = Bounds<Pixels>>,
) -> bool {
    regions.into_iter().any(|bounds| bounds.contains(&position))
}

fn restore_current_result_selection(
    table: &mut TableState<LogTableDelegate>,
    row_ix: usize,
    cx: &mut Context<TableState<LogTableDelegate>>,
) {
    table.delegate().settle_table_selection(row_ix);
    table.set_active_log_row(row_ix, cx);
}

/// 把长度对齐到设备像素网格，取整方式与 GPUI 布局和文字排版内部使用的一致。
///
/// 固定行高表格的行高会先落到设备像素上再回算成逻辑像素，文字行高同样如此。换行列表
/// 的槽位高度如果直接用未对齐的字号加行距，行距就会和固定行高模式差出不到 1px，并在
/// 逐行累加时忽大忽小；先对齐再使用，两种模式下不换行的行画出来一样高。
fn snap_to_device_pixels(value: Pixels, scale_factor: f32) -> Pixels {
    if scale_factor <= 0. || !scale_factor.is_finite() {
        return value;
    }
    let scaled = value.as_f32() * scale_factor;
    px((scaled.abs() - 0.5).ceil().copysign(scaled) / scale_factor)
}

fn wrapped_viewport_measurement_range(
    first_visible: usize,
    viewport_height: Pixels,
    base_height: Pixels,
    count: usize,
) -> Range<usize> {
    let visible_count = (viewport_height / base_height.max(px(1.))).ceil().max(1.) as usize;
    first_visible.saturating_sub(2).min(count)
        ..first_visible
            .saturating_add(visible_count)
            .saturating_add(2)
            .min(count)
}

fn scrollbar_preload_range(
    offset: Point<Pixels>,
    row_count: usize,
    viewport_height: Pixels,
    row_height: Pixels,
) -> Range<usize> {
    if row_count == 0 {
        return 0..0;
    }
    let row_height = row_height.max(px(1.));
    let max_top = (row_height * row_count as f32 - viewport_height).max(px(0.));
    let top = (-offset.y).clamp(px(0.), max_top);
    let first = (top / row_height).floor().max(0.) as usize;
    let first = first.min(row_count.saturating_sub(1));
    let visible_count = (viewport_height / row_height).ceil().max(1.) as usize;
    first.saturating_sub(2)
        ..first
            .saturating_add(visible_count)
            .saturating_add(2)
            .min(row_count)
}

fn take_pending_log_scroll_target<K: Clone + Ord>(
    pending: &mut PendingLogScrollFrames,
    key: (u64, WrappedRegion),
    viewport: &LogViewportState<K>,
) -> Option<LogScrollFrameTarget> {
    if let Some(offset) = viewport.take_pending_scrollbar_offset() {
        pending.clear(key);
        Some(LogScrollFrameTarget::Scrollbar(offset))
    } else {
        pending.take(key)
    }
}

#[cfg(test)]
#[path = "workspace/scroll_position_tests.rs"]
mod scroll_position_tests;

impl<K> Default for WrappedListState<K> {
    fn default() -> Self {
        Self {
            item_count: Rc::new(Cell::new(0)),
            base_height: Rc::new(Cell::new(px(0.))),
            measured_heights: Rc::new(RefCell::new(BTreeMap::new())),
            pending_heights: RefCell::new(BTreeMap::new()),
            measured_rows: RefCell::new(VecDeque::new()),
            height_corrections: Rc::new(RefCell::new(Vec::new())),
            scroll_handle: SparseVirtualListScrollHandle::new(),
            text_selections: RefCell::default(),
            measurement_anchor: Rc::new(Cell::new(None)),
            pending_scrollbar_offset: Rc::default(),
            layout_key: RefCell::new(None),
            row_bounds: Rc::default(),
        }
    }
}

impl<K: Clone + Ord> WrappedListState<K> {
    fn logical_scroll_handle(
        &self,
        item_count: usize,
        slot_height: Pixels,
    ) -> LogicalVirtualScrollHandle {
        LogicalVirtualScrollHandle {
            handle: self.scroll_handle.clone(),
            measured_heights: self.measured_heights.clone(),
            height_corrections: self.height_corrections.clone(),
            pending_offset: self.pending_scrollbar_offset.clone(),
            item_count,
            slot_height,
        }
    }
    fn sizes(&self, count: usize, base_height: Pixels) -> SparseListMeasurements {
        if self.item_count.get() != count || self.base_height.get() != base_height {
            self.item_count.set(count);
            self.base_height.set(base_height);
            self.measured_heights.borrow_mut().clear();
            self.pending_heights.borrow_mut().clear();
            self.measured_rows.borrow_mut().clear();
            self.height_corrections.borrow_mut().clear();
            self.row_bounds.borrow_mut().clear();
            if let Some(anchor) = self.measurement_anchor.get() {
                self.scroll_row_to_viewport_y(
                    anchor.row_ix.min(count.saturating_sub(1)),
                    anchor.viewport_y,
                );
            }
        }
        let pending = std::mem::take(&mut *self.pending_heights.borrow_mut());
        if !pending.is_empty() {
            let old_top = (-self.scroll_handle.offset().y).max(px(0.));
            let old_max = self.scroll_handle.max_offset().y.max(px(0.));
            let was_at_bottom = old_max > px(0.) && old_top >= old_max - px(0.5);
            let explicit_anchor = self.measurement_anchor.get();
            let anchor = explicit_anchor.unwrap_or_else(|| {
                let row = self.first_visible_row();
                RowViewportPosition {
                    row_ix: row,
                    viewport_y: self.prefix_height(row) - old_top,
                }
            });
            let mut next = self.measured_heights.borrow().clone();
            let mut measured_rows = self.measured_rows.borrow_mut();
            for (row_ix, height) in pending {
                if row_ix >= count {
                    continue;
                }
                next.insert(row_ix, height.max(base_height));
                if let Some(old_ix) = measured_rows.iter().position(|row| *row == row_ix) {
                    measured_rows.remove(old_ix);
                }
                measured_rows.push_back(row_ix);
            }
            while measured_rows.len() > WRAPPED_HEIGHT_CACHE_LIMIT {
                if let Some(evicted) = measured_rows.pop_front() {
                    next.remove(&evicted);
                }
            }
            let mut corrections = measured_rows
                .iter()
                .filter_map(|row_ix| {
                    next.get(row_ix)
                        .map(|height| (*row_ix, *height - base_height))
                })
                .collect::<Vec<_>>();
            corrections.sort_by_key(|(row_ix, _)| *row_ix);
            let mut cumulative = px(0.);
            for (_, correction) in &mut corrections {
                cumulative += *correction;
                *correction = cumulative;
            }
            *self.height_corrections.borrow_mut() = corrections;
            *self.measured_heights.borrow_mut() = next;
            if explicit_anchor.is_none() && was_at_bottom {
                self.scroll_handle.scroll_to_bottom();
            } else {
                self.set_row_viewport_y(anchor.row_ix, anchor.viewport_y);
            }
        }
        SparseListMeasurements {
            item_count: count,
            base_height,
            measured_heights: self.measured_heights.clone(),
            cumulative_corrections: self.height_corrections.clone(),
        }
    }

    fn queue_measured_height(&self, row_ix: usize, height: Pixels, base_height: Pixels) -> bool {
        let height = height.max(base_height);
        let current_height = self
            .pending_heights
            .borrow()
            .get(&row_ix)
            .copied()
            .or_else(|| self.measured_heights.borrow().get(&row_ix).copied())
            .or_else(|| (row_ix < self.item_count.get()).then_some(base_height));
        let Some(current_height) = current_height else {
            return false;
        };
        if (current_height - height).abs() < px(0.5) {
            return false;
        }
        self.pending_heights.borrow_mut().insert(row_ix, height);
        true
    }

    fn row_height(&self, row_ix: usize) -> Option<Pixels> {
        self.pending_heights
            .borrow()
            .get(&row_ix)
            .copied()
            .or_else(|| self.measured_heights.borrow().get(&row_ix).copied())
            .or_else(|| (row_ix < self.item_count.get()).then_some(self.base_height.get()))
    }

    fn has_known_row_height(&self, row_ix: usize) -> bool {
        // A measured single-line row does not need a sparse correction, so its retained bounds
        // are the evidence that this layout revision has already measured it.
        self.pending_heights.borrow().contains_key(&row_ix)
            || self.measured_heights.borrow().contains_key(&row_ix)
            || self.row_bounds.borrow().contains_key(&row_ix)
    }

    fn prime_measured_heights(
        &self,
        count: usize,
        base_height: Pixels,
        heights: impl IntoIterator<Item = (usize, Pixels)>,
    ) {
        self.sizes(count, base_height);
        for (row_ix, height) in heights {
            self.queue_measured_height(row_ix, height, base_height);
        }
        self.sizes(count, base_height);
    }

    fn measured_heights_by_key<T: Ord>(
        &self,
        key_for_row: impl Fn(usize) -> Option<T>,
    ) -> BTreeMap<T, Pixels> {
        let measured_heights = self.measured_heights.borrow();
        let pending_heights = self.pending_heights.borrow();
        let mut measured_rows = self
            .measured_rows
            .borrow()
            .iter()
            .copied()
            .collect::<Vec<_>>();
        measured_rows.extend(pending_heights.keys().copied());
        measured_rows.sort_unstable();
        measured_rows.dedup();
        measured_rows
            .into_iter()
            .filter_map(|row_ix| {
                let key = key_for_row(row_ix)?;
                let height = pending_heights
                    .get(&row_ix)
                    .copied()
                    .or_else(|| measured_heights.get(&row_ix).copied())?;
                Some((key, height))
            })
            .collect()
    }

    fn reset_with_remapped_heights<T: Ord>(
        &mut self,
        count: usize,
        base_height: Pixels,
        measured_heights: BTreeMap<T, Pixels>,
        row_for_key: impl Fn(&T) -> Option<usize>,
    ) {
        self.invalidate();
        let retained = measured_heights
            .iter()
            .filter_map(|(key, height)| Some((row_for_key(key)?, *height)));
        self.prime_measured_heights(count, base_height, retained);
    }

    fn invalidate(&mut self) {
        self.item_count.set(0);
        self.base_height.set(px(0.));
        self.measured_heights.borrow_mut().clear();
        self.pending_heights.borrow_mut().clear();
        self.measured_rows.borrow_mut().clear();
        self.height_corrections.borrow_mut().clear();
        self.text_selections.borrow_mut().clear();
        self.row_bounds.borrow_mut().clear();
        self.pending_scrollbar_offset.set(None);
    }

    fn invalidate_for_layout(&self, key: WrappedLayoutKey) -> bool {
        if !self.needs_layout_invalidation(&key) {
            return false;
        }
        self.layout_key.replace(Some(key));
        self.item_count.set(0);
        self.base_height.set(px(0.));
        self.measured_heights.borrow_mut().clear();
        self.pending_heights.borrow_mut().clear();
        self.measured_rows.borrow_mut().clear();
        self.height_corrections.borrow_mut().clear();
        self.text_selections.borrow_mut().clear();
        self.row_bounds.borrow_mut().clear();
        self.pending_scrollbar_offset.set(None);
        true
    }

    fn needs_layout_invalidation(&self, key: &WrappedLayoutKey) -> bool {
        key.width > px(0.)
            && !self
                .layout_key
                .borrow()
                .as_ref()
                .is_some_and(|current| current.is_equivalent_to(key))
    }

    fn layout_width(&self) -> Option<Pixels> {
        self.layout_key.borrow().as_ref().map(|key| key.width)
    }

    fn capture_row_viewport_position(
        &self,
        preferred_row: Option<usize>,
    ) -> Option<RowViewportPosition> {
        let count = self.item_count.get();
        if count == 0 {
            return None;
        }
        let top = (-self.scroll_handle.offset().y).max(px(0.));
        let viewport_height = self.scroll_handle.bounds().size.height;
        let first = self.first_visible_row();
        let row_ix = viewport_anchor_row(count, first, preferred_row, |row_ix| {
            let row_top = self.prefix_height(row_ix);
            let row_height = self
                .measured_heights
                .borrow()
                .get(&row_ix)
                .copied()
                .unwrap_or(self.base_height.get());
            row_intersects_viewport(row_top - top, row_height, viewport_height)
        });
        Some(RowViewportPosition {
            row_ix,
            viewport_y: self.prefix_height(row_ix) - top,
        })
    }

    fn restore_viewport(&self, row_ix: usize, viewport_y: Pixels, at_end: bool) {
        if at_end {
            self.scroll_to_end();
        } else {
            self.scroll_row_to_viewport_y(row_ix, viewport_y);
        }
    }

    fn scroll_to_end(&self) {
        self.clear_measurement_anchor();
        self.scroll_handle.scroll_to_bottom();
    }

    fn scroll_row_to_viewport_y(&self, row_ix: usize, viewport_y: Pixels) {
        self.measurement_anchor
            .set(Some(RowViewportPosition { row_ix, viewport_y }));
        self.set_row_viewport_y(row_ix, viewport_y);
    }

    fn set_row_viewport_y(&self, row_ix: usize, viewport_y: Pixels) {
        let top = (self.prefix_height(row_ix) - viewport_y).max(px(0.));
        self.scroll_handle
            .set_offset(point(self.scroll_handle.offset().x, -top));
    }

    fn reset_scroll_for_mode_switch(&mut self) {
        self.scroll_handle = SparseVirtualListScrollHandle::new();
        self.clear_measurement_anchor();
        self.pending_scrollbar_offset.set(None);
    }

    fn apply_logical_scrollbar_offset(
        &self,
        offset: Point<Pixels>,
        item_count: usize,
        slot_height: Pixels,
    ) {
        self.measurement_anchor.set(None);
        let offset =
            self.viewport_offset_for_logical_scrollbar_offset(offset, item_count, slot_height);
        self.scroll_handle.set_offset(offset);
    }

    fn viewport_offset_for_logical_scrollbar_offset(
        &self,
        offset: Point<Pixels>,
        item_count: usize,
        slot_height: Pixels,
    ) -> Point<Pixels> {
        let viewport_height = self.scroll_handle.bounds().size.height;
        let logical_height = slot_height * item_count as f32;
        let logical_max = (logical_height - viewport_height).max(px(0.));
        let requested_top = (-offset.y).clamp(px(0.), logical_max);
        let actual_top = if requested_top >= logical_max - px(0.5) {
            self.scroll_handle.max_offset().y.max(px(0.))
        } else {
            let row = (requested_top / slot_height).floor().max(0.) as usize;
            let row = row.min(item_count.saturating_sub(1));
            let logical_row_top = slot_height * row as f32;
            let fraction = ((requested_top - logical_row_top) / slot_height).clamp(0., 1.);
            let corrections = self.height_corrections.borrow();
            let actual_row_top = prefix_height_for(slot_height, &corrections, row);
            let actual_row_height = self
                .measured_heights
                .borrow()
                .get(&row)
                .copied()
                .unwrap_or(slot_height)
                .max(slot_height);
            actual_row_top + actual_row_height * fraction
        };
        point(self.scroll_handle.offset().x, -actual_top)
    }

    fn clear_measurement_anchor(&self) {
        self.measurement_anchor.set(None);
    }

    fn first_visible_row(&self) -> usize {
        let top = -self.scroll_handle.offset().y;
        let count = self.item_count.get();
        row_for_absolute_y(
            count,
            self.base_height.get(),
            &self.height_corrections.borrow(),
            top,
        )
    }

    fn place_row_at_top(&self, row_ix: usize) {
        self.clear_measurement_anchor();
        let y = self.prefix_height(row_ix);
        self.scroll_handle.set_offset(point(px(0.), -y));
    }

    fn center_row(&self, row_ix: usize) {
        self.clear_measurement_anchor();
        let count = self.item_count.get();
        if row_ix >= count {
            self.scroll_handle
                .scroll_to_item(row_ix, ScrollStrategy::Center);
            return;
        }
        let row_height = self
            .measured_heights
            .borrow()
            .get(&row_ix)
            .copied()
            .unwrap_or(self.base_height.get());
        let viewport_height = self.scroll_handle.bounds().size.height;
        if viewport_height <= px(0.) {
            self.scroll_handle
                .scroll_to_item(row_ix, ScrollStrategy::Center);
            return;
        }
        let row_top = self.prefix_height(row_ix);
        let content_height = self.prefix_height(count);
        let top = centered_scroll_top(
            row_top,
            row_height,
            viewport_height,
            content_height - viewport_height,
        );
        self.restore_viewport(row_ix, row_top - top, false);
    }

    fn retain_visible_rows(&self, visible_range: &Range<usize>) {
        self.row_bounds
            .borrow_mut()
            .retain(|row_ix, _| visible_range.contains(row_ix));
    }

    fn prefix_height(&self, row_ix: usize) -> Pixels {
        prefix_height_for(
            self.base_height.get(),
            &self.height_corrections.borrow(),
            row_ix,
        )
    }
}

impl DocumentTab {
    fn result_rows(&self, cx: &App) -> CompressedRows {
        self.result_table
            .read(cx)
            .delegate()
            .projected_rows()
            .cloned()
            .unwrap_or_default()
    }

    fn result_row_count(&self, cx: &App) -> usize {
        self.result_table.read(cx).delegate().row_count()
    }

    fn result_row_ix(&self, source_row: usize, cx: &App) -> Option<usize> {
        self.result_table
            .read(cx)
            .delegate()
            .row_ix_for_key(LogRowKey::Row {
                document_id: self.id,
                source_row,
            })
    }

    fn compute_result_rows(&self) -> CompressedRows {
        compute_result_rows(
            self.result_mode,
            Some(&self.search_result),
            &self.marked_rows,
        )
    }

    fn select_and_center_log_source_row(&mut self, source_row: usize, cx: &mut App) -> bool {
        let Some(row_ix) = self.document.local_row(source_row) else {
            return false;
        };
        self.log_table.update(cx, |table, cx| {
            // This selection mirrors a result-row command. Do not emit a second table selection
            // event that would make the log body steal focus/region ownership from the results.
            table.delegate().set_active_log_row(Some(row_ix));
            table.delegate().settle_table_selection(row_ix);
            cx.notify();
        });
        self.log_viewport.center_row(row_ix);
        true
    }

    fn install_result_rows(&mut self, result_rows: CompressedRows, cx: &mut App) {
        self.restoring_result_selection = true;
        let marked_rows = self.marked_rows.clone();
        let active_restored = self.result_table.update(cx, |table, cx| {
            if self.auto_follow {
                table.delegate().set_active_log_row(None);
            }
            table
                .delegate_mut()
                .set_matched_rows(self.search_result.line_indices.clone());
            table.delegate_mut().set_marked_rows(marked_rows);
            table.delegate_mut().set_row_projection(result_rows);
            let active_restored = table.sync_active_log_row(cx);
            table.refresh(cx);
            cx.notify();
            active_restored
        });
        if !active_restored {
            self.restoring_result_selection = false;
        }
    }

    fn refresh_result_rows(&mut self, row_height: Pixels, cx: &mut App) {
        let result_rows = self.compute_result_rows();
        let projection_changed =
            self.result_table.read(cx).delegate().projected_rows() != Some(&result_rows);
        if !projection_changed {
            self.install_result_rows(result_rows, cx);
            return;
        }
        let word_wrap = self.result_viewport.is_wrapped();
        let row_height = if word_wrap && self.result_viewport.wrapped_base_height() > px(0.) {
            self.result_viewport.wrapped_base_height()
        } else {
            row_height
        };
        let viewport_anchor =
            Workspace::capture_local_viewport_anchor(self, WrappedRegion::Results, row_height, cx);
        let measured_heights = if word_wrap {
            let table = self.result_table.read(cx);
            self.result_viewport
                .wrapped_measured_heights_by_key(|row_ix| table.delegate().row_key(row_ix))
        } else {
            BTreeMap::new()
        };
        self.install_result_rows(result_rows, cx);
        if word_wrap {
            let table = self.result_table.read(cx);
            self.result_viewport.reset_wrapped_with_remapped_heights(
                table.delegate().row_count(),
                row_height,
                measured_heights,
                |key| table.delegate().row_ix_for_key(*key),
            );
        } else {
            self.result_viewport.invalidate_wrapped();
        }
        Workspace::restore_local_viewport_anchor(
            self,
            WrappedRegion::Results,
            viewport_anchor,
            row_height,
            cx,
        );
    }

    fn refresh_view_options(&self, cx: &mut App) {
        let show_line_numbers = self.show_line_numbers;
        let show_row_separators = self.show_row_separators;
        self.log_table.update(cx, |table, cx| {
            table
                .delegate_mut()
                .set_matched_rows(self.search_result.line_indices.clone());
            table
                .delegate_mut()
                .set_view_options(show_line_numbers, show_row_separators);
            table.refresh(cx);
        });
        self.result_table.update(cx, |table, cx| {
            table
                .delegate_mut()
                .set_matched_rows(self.search_result.line_indices.clone());
            table
                .delegate_mut()
                .set_view_options(show_line_numbers, show_row_separators);
            table.refresh(cx);
        });
    }

    fn refresh_appearance(&self, settings: &AppSettings, cx: &mut App) {
        self.log_table.update(cx, |table, cx| {
            table.delegate_mut().set_appearance(settings);
            table.refresh(cx);
        });
        self.result_table.update(cx, |table, cx| {
            table.delegate_mut().set_appearance(settings);
            table.refresh(cx);
        });
    }

    fn refresh_word_boundary_characters(&self, characters: &str, cx: &mut App) {
        for table in [&self.log_table, &self.result_table] {
            table.update(cx, |table, cx| {
                table
                    .delegate_mut()
                    .set_word_boundary_characters(characters.to_string());
                table.refresh(cx);
            });
        }
    }

    fn refresh_log_level_highlighting(&self, enabled: bool, cx: &mut App) {
        self.log_table.update(cx, |table, cx| {
            table.delegate_mut().set_highlight_log_levels(enabled);
            table.refresh(cx);
        });
        self.result_table.update(cx, |table, cx| {
            table.delegate_mut().set_highlight_log_levels(enabled);
            table.refresh(cx);
        });
    }

    fn refresh_search_matcher(&self, highlight_matches: bool, cx: &mut App) {
        let search_matcher = highlight_matches
            .then(|| self.search_matcher.clone())
            .flatten();
        self.log_table.update(cx, |table, cx| {
            table
                .delegate_mut()
                .set_matched_rows(self.search_result.line_indices.clone());
            table
                .delegate_mut()
                .set_search_matcher(search_matcher.clone());
            table.refresh(cx);
        });
        self.result_table.update(cx, |table, cx| {
            table
                .delegate_mut()
                .set_matched_rows(self.search_result.line_indices.clone());
            table.delegate_mut().set_search_matcher(search_matcher);
            table.refresh(cx);
        });
    }

    fn selected_source_rows_compressed(&self, cx: &App) -> CompressedRows {
        let table = match self.selection_table {
            SelectionTable::Log => &self.log_table,
            SelectionTable::Results => &self.result_table,
        };
        let state = table.read(cx);
        let mut rows = state.delegate().selected_source_rows_compressed();
        if rows.is_empty()
            && let Some(source_row) = state
                .active_log_row()
                .and_then(|row_ix| state.delegate().source_row(row_ix))
        {
            rows.insert(source_row);
        }
        rows
    }

    fn selected_rows_count(&self, cx: &App) -> usize {
        let table = match self.selection_table {
            SelectionTable::Log => &self.log_table,
            SelectionTable::Results => &self.result_table,
        };
        let state = table.read(cx);
        let selected_count = state.delegate().selected_rows_count();
        selected_count.max(usize::from(state.active_log_row().is_some()))
    }
}

enum Activity {
    Ready,
    Opening,
    Searching,
    Error,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum SearchAutocompleteMode {
    #[default]
    Closed,
    Matches,
    History,
}

#[derive(Clone, Copy)]
enum ReloadStrategy {
    Full,
    ExtendAppend,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ResultExportOperation {
    OpenInNewTab,
    MergeByTimestamp,
    SaveAs,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LineCopyScope {
    Local,
    Global,
}

enum ColorKeywordSelection {
    Text(String),
    Rows(CompressedRows),
}

struct ColorKeywordTarget {
    document_id: u64,
    document: Arc<LogDocument>,
    selection: ColorKeywordSelection,
}

struct PreparedColorKeywords {
    document_id: u64,
    document: Arc<LogDocument>,
    keywords: BTreeSet<String>,
}

enum ColorRuleAction {
    Cycle,
    Apply {
        label_id: Option<String>,
        clear_all: bool,
    },
}

enum ColorRuleOutcome {
    EmptyKeywords,
    MissingLabels,
    MissingLabel,
    CycleRemoved { count: usize },
    CycleApplied { label: ColorLabel, count: usize },
    Applied,
    Removed,
    Cleared,
}

struct PreparedColorRuleUpdate {
    document_id: u64,
    document: Arc<LogDocument>,
    expected_rules: Vec<KeywordColorRule>,
    expected_labels: Vec<ColorLabel>,
    rules: Vec<KeywordColorRule>,
    resolved: Option<Arc<ResolvedColorRules>>,
    last_color_label_id: Option<String>,
    outcome: ColorRuleOutcome,
}

struct ColorRuleResolutionInput {
    document_id: u64,
    document: Arc<LogDocument>,
    rules: Vec<KeywordColorRule>,
}

struct PreparedColorRuleResolution {
    document_id: u64,
    document: Arc<LogDocument>,
    rules: Vec<KeywordColorRule>,
    resolved: Arc<ResolvedColorRules>,
}

struct CopiedLogLines {
    text: String,
    count: usize,
    first_source_row: Option<usize>,
}

enum DocumentLineTask<T> {
    Completed(T),
    Cancelled,
    SourceUnavailable,
}

#[cfg(test)]
impl<T> DocumentLineTask<T> {
    fn expect(self, message: &str) -> T {
        match self {
            Self::Completed(value) => value,
            Self::Cancelled => panic!("{message}: task was cancelled"),
            Self::SourceUnavailable => panic!("{message}: source was unavailable"),
        }
    }

    fn is_cancelled(&self) -> bool {
        matches!(self, Self::Cancelled)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TabCloseGroup {
    Current,
    Others,
    Left,
    Right,
    All,
}

#[derive(Clone)]
struct DraggedTab {
    tab_id: WorkspaceTabId,
    title: SharedString,
    position: Point<Pixels>,
    source: WeakEntity<Workspace>,
}

impl DraggedTab {
    fn new(tab_id: WorkspaceTabId, title: SharedString, source: WeakEntity<Workspace>) -> Self {
        Self {
            tab_id,
            title,
            position: Point::default(),
            source,
        }
    }

    fn position(mut self, position: Point<Pixels>) -> Self {
        self.position = position;
        self
    }
}

impl Render for DraggedTab {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _performance_scope = crate::ui_performance::scope("DraggedTab::render");
        let width = cx.theme().font_size * 15.;
        let height = cx.theme().font_size * 2.5;

        let element_id = match self.tab_id {
            WorkspaceTabId::Document(id) => ElementId::from(("document-tab-drag-preview", id)),
            WorkspaceTabId::New(id) => ElementId::from(("new-tab-drag-preview", id)),
        };

        div()
            .id(element_id)
            .pl(self.position.x - width * 0.5)
            .pt(self.position.y - height * 0.5)
            .child(
                h_flex()
                    .w(width)
                    .h(height)
                    .px_3()
                    .rounded_lg()
                    .border_1()
                    .border_color(cx.theme().primary)
                    .bg(cx.theme().popover.opacity(0.96))
                    .text_color(cx.theme().popover_foreground)
                    .shadow_lg()
                    .overflow_hidden()
                    .child(div().truncate().child(self.title.clone())),
            )
    }
}

#[derive(Clone, Copy)]
struct SearchPanelResizeGesture {
    start_y: Pixels,
    initial_height: Pixels,
}

struct DirectorySearchResult {
    title: SharedString,
    path: PathBuf,
    document: Arc<LogDocument>,
    search_result: SearchResult,
}

#[derive(Clone)]
struct PendingDirectoryResultJump {
    path: PathBuf,
    source_row: usize,
    expected_document: Arc<LogDocument>,
}

impl PendingDirectoryResultJump {
    fn matches(&self, document: &LogDocument) -> bool {
        result_snapshot_matches_document(&self.path, &self.expected_document, document)
    }
}

struct CompletedGlobalSearch {
    scope: SearchScope,
    query: SearchQuery,
    results: GlobalSearchResults,
    matcher: Option<SearchMatcher>,
    preserve_viewport: bool,
}

pub struct Workspace {
    primary_window: bool,
    focus_handle: FocusHandle,
    status_surface: Entity<WorkspaceStatusSurface>,
    query: Entity<InputState>,
    search_history: Vec<String>,
    predefined_filters: Vec<PredefinedFilter>,
    cloud: CloudController,
    search_history_ix: Option<usize>,
    search_history_draft: Option<String>,
    search_autocomplete_mode: SearchAutocompleteMode,
    search_suggestion_ix: Option<usize>,
    search_suggestion_scroll: UniformListScrollHandle,
    quick_find: QuickFindState,
    global_search: GlobalSearchState,
    global_table: Entity<TableState<GlobalSearchTableDelegate>>,
    global_surface: Entity<LogRegionSurface>,
    global_viewport: LogViewportState<(u64, usize)>,
    global_text_selection_scope: TextSelectionScopeId,
    global_results_focus_handle: FocusHandle,
    active_log_region: LogRegion,
    last_user_log_region: LogRegion,
    transient_paths: BTreeSet<PathMatchKey>,
    pending_tab_moves: BTreeSet<u64>,
    documents: Vec<DocumentTab>,
    tabs: Vec<WorkspaceTabId>,
    active_tab_id: WorkspaceTabId,
    active_ix: Option<usize>,
    document_tab_scroll: ScrollHandle,
    pending_document_tab_reveal: Cell<Option<u64>>,
    pending_directory_result_jump: Option<PendingDirectoryResultJump>,
    next_document_id: u64,
    next_new_tab_id: u64,
    case_sensitive: bool,
    regex: bool,
    activity: Activity,
    selected_source_row: Option<usize>,
    row_drag_bounds: BTreeMap<(u64, WrappedRegion), Bounds<Pixels>>,
    row_drag_selection: Option<RowDragSelection>,
    row_drag_frame_scheduled: bool,
    visible_line_tasks: BTreeMap<(u64, WrappedRegion), Task<()>>,
    pending_log_scroll_frames: PendingLogScrollFrames,
    global_group_toggle_task: Option<Task<()>>,
    global_group_toggle_revision: u64,
    tab_activation_task: Option<Task<()>>,
    tab_activation_revision: u64,
    open_task: Option<Task<()>>,
    pending_external_paths: Vec<PathBuf>,
    searches: SearchController,
    result_export_task: Option<Task<()>>,
    result_export_operation: Option<ResultExportOperation>,
    line_copy_task: Option<Task<()>>,
    line_copy_cancellation: Option<SearchCancellation>,
    line_copy_revision: u64,
    color_rule_task: Option<Task<()>>,
    color_rule_cancellation: Option<SearchCancellation>,
    color_rule_revision: u64,
    color_labels_resolution_task: Option<Task<()>>,
    color_labels_resolution_cancellation: Option<SearchCancellation>,
    color_labels_resolution_revision: u64,
    file_drop_visible: bool,
    file_drop_tab_transfer: Option<TabTransferMode>,
    cross_window_drop_ix: Option<usize>,
    tab_drop_layout: Rc<RefCell<TabDropLayout>>,
    search_panel_state: Entity<ResizableState>,
    search_panel_height: Option<Pixels>,
    search_panel_height_modified: bool,
    search_panel_resize_gesture: Option<SearchPanelResizeGesture>,
    search_panel_resize_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    file_watch_task: Option<Task<()>>,
    deactivated_input_focus: Option<FocusHandle>,
    _cloud_client_bootstrap_task: Task<()>,
    persistence: PersistenceController,
    last_workspace_files: Vec<LastWorkspaceFile>,
    pinned_files: Vec<RecentFile>,
    recent_files: Vec<RecentFile>,
    history_loading: bool,
    history_dialog_loading: bool,
    history_clearing: bool,
    pinned_updating: bool,
    app_settings: AppSettings,
    scale_factor: f32,
    color_labels: Vec<ColorLabel>,
    last_color_label_id: Option<String>,
    color_labels_saving: bool,
    predefined_filters_saving: bool,
    pending_predefined_filters_save: Option<(u64, Vec<PredefinedFilter>)>,
    settings_saving: bool,
    search_defaults_modified: bool,
    subscriptions: Vec<Subscription>,
    history_dialog_subscription: Option<Subscription>,
    predefined_filters_dialog_subscription: Option<Subscription>,
    settings_dialog_subscription: Option<Subscription>,
}

impl Workspace {}

mod quick_find;
mod render_shell;
mod result_export_flow;
mod search_orchestration;
mod tab_lifecycle;
mod viewport_orchestration;
mod window_registry;

impl Workspace {
    pub fn new(
        primary_window: bool,
        initial_documents: Vec<InitialDocument>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        window.set_window_title(crate::tr!("新标签页 — VCLogg2", "New tab — VCLogg2"));
        let query =
            cx.new(|cx| InputState::new(window, cx).placeholder(crate::tr!("搜索", "Search")));
        let quick_find_query = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(crate::tr!("在当前视图中查找", "Find in current view"))
        });
        let status_surface = {
            let workspace = cx.weak_entity();
            cx.new(move |cx| WorkspaceStatusSurface::new(workspace, cx))
        };
        let global_table = cx.new(|cx| {
            TableState::new(GlobalSearchTableDelegate::new(), window, cx)
                .loop_selection(false)
                .row_selectable(false)
                .sortable(false)
                .col_movable(false)
                .col_selectable(false)
        });
        let global_viewport = {
            let table = global_table.read(cx);
            LogViewportState::new(
                false,
                table.vertical_scroll_handle.clone(),
                table.delegate().row_bounds_handle(),
            )
        };
        let global_surface = {
            let workspace = cx.weak_entity();
            let table = global_table.clone();
            cx.new(move |cx| {
                LogRegionSurface::new(workspace, 0, WrappedRegion::GlobalResults, &table, cx)
            })
        };
        let global_text_selection_scope = TextSelectionScopeId::default();
        let global_results_focus_handle = cx.focus_handle().tab_stop(true);
        let search_panel_state = cx.new(|_| ResizableState::default());
        cx.on_focus_in(
            &global_results_focus_handle,
            window,
            |this: &mut Workspace, _, cx| {
                this.active_log_region = LogRegion::GlobalResults;
                cx.notify();
            },
        )
        .detach();
        let global_result_mode_select = cx.new(|cx| {
            SelectState::new(
                ResultMode::ALL.to_vec(),
                Some(IndexPath::new(ResultMode::MatchesAndMarks.select_index())),
                window,
                cx,
            )
        });
        let mut subscriptions =
            vec![
                cx.subscribe_in(&query, window, |this, _, event: &InputEvent, window, cx| {
                    match event {
                        InputEvent::Change => {
                            this.reset_search_history_navigation();
                            this.refresh_search_autocomplete(cx);
                        }
                        InputEvent::PressEnter { .. }
                            if !this.accept_active_search_suggestion(window, cx) =>
                        {
                            this.start_search(window, cx);
                        }
                        InputEvent::PressEnter { .. } => {}
                        _ => {}
                    }
                }),
            ];
        subscriptions.push(cx.subscribe_in(
            &quick_find_query,
            window,
            |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::Change => this.schedule_incremental_quick_find(window, cx),
                InputEvent::PressEnter { shift, .. } => this.start_quick_find(
                    if *shift {
                        QuickFindDirection::Backward
                    } else {
                        QuickFindDirection::Forward
                    },
                    false,
                    window,
                    cx,
                ),
                _ => {}
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &global_table,
            window,
            |this, table, event: &TableEvent, window, cx| {
                if matches!(event, TableEvent::ClearSelection) {
                    if table.read(cx).delegate().take_suppressed_table_clear() {
                        return;
                    }
                    table.read(cx).delegate().clear_row_selection();
                    table.read(cx).delegate().set_active_log_row(None);
                    this.schedule_workspace_search_state_save(window, cx);
                    cx.notify();
                    return;
                }
                let TableEvent::SelectRow(row_ix) = event else {
                    return;
                };
                let keep_quick_find_focus = this.quick_find_input_has_focus(window, cx);
                let word_wrap = this.global_viewport.is_wrapped();
                let Some(row) = table.read(cx).delegate().row(*row_ix) else {
                    return;
                };
                let wrapped_group_state = if word_wrap {
                    match row {
                        GlobalSearchRow::Group { document_id } => {
                            let anchor = this
                                .global_viewport
                                .capture_wrapped_viewport_position(Some(*row_ix))
                                .map(|position| RowViewportAnchor {
                                    key: LogRowKey::FileGroup { document_id },
                                    viewport_y: position.viewport_y,
                                    fallback_ix: position.row_ix,
                                });
                            let table = table.read(cx);
                            let measured_heights = this
                                .global_viewport
                                .wrapped_measured_heights_by_key(|row_ix| {
                                    table.delegate().row_key(row_ix)
                                });
                            Some((anchor, measured_heights))
                        }
                        GlobalSearchRow::Match { .. } => None,
                    }
                } else {
                    None
                };
                match row {
                    GlobalSearchRow::Group { .. } => {
                        table.read(cx).delegate().clear_row_selection();
                    }
                    GlobalSearchRow::Match { .. } => {
                        table.read(cx).delegate().settle_table_selection(*row_ix);
                        this.active_log_region = LogRegion::GlobalResults;
                    }
                }
                if this.global_search.restoring_selection {
                    this.global_search.restoring_selection = false;
                    return;
                }
                let mut save_search_state_immediately = true;
                match row {
                    GlobalSearchRow::Group { document_id } => {
                        if table.read(cx).delegate().group_has_results(document_id) {
                            let (anchor, measured_heights) =
                                wrapped_group_state.unwrap_or_else(|| (None, BTreeMap::new()));
                            this.prepare_global_group_toggle(
                                document_id,
                                anchor,
                                measured_heights,
                                this.log_row_height(),
                                window,
                                cx,
                            );
                            save_search_state_immediately = false;
                        } else {
                            this.activate_global_group(document_id, window, cx);
                        }
                    }
                    GlobalSearchRow::Match {
                        document_id,
                        source_row,
                    } => this.jump_to_global_result(document_id, source_row, window, cx),
                }
                if !keep_quick_find_focus {
                    this.global_results_focus_handle.focus(window, cx);
                }
                this.active_log_region = LogRegion::GlobalResults;
                if save_search_state_immediately {
                    this.schedule_workspace_search_state_save(window, cx);
                }
                cx.notify();
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &global_result_mode_select,
            window,
            |this, _, event: &SelectEvent<Vec<ResultMode>>, window, cx| {
                let SelectEvent::Confirm(Some(mode)) = event else {
                    return;
                };
                if this.global_search.result_mode == *mode {
                    return;
                }
                this.global_search.result_mode = *mode;
                if mode.includes_marks()
                    && this.documents.iter().any(|tab| {
                        this.global_search.selected_documents.contains(&tab.id)
                            && !tab.marked_rows.is_empty()
                    })
                {
                    this.global_search.results_visible = true;
                }
                this.refresh_global_result_rows(window, cx);
                this.schedule_workspace_search_state_save(window, cx);
                cx.notify();
            },
        ));
        let focus_handle = cx.focus_handle().tab_stop(true);
        let focus_on_start = focus_handle.clone();
        window.defer(cx, move |window, cx| focus_on_start.focus(window, cx));
        let cloud_client_bootstrap_task = cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let client = CloudClient::open_default().map_err(|error| error.to_string())?;
                    let connection = client.saved_connection().ok().flatten();
                    Ok::<_, String>((client, connection))
                })
                .await;
            _ = this.update_in(cx, |this, _, cx| {
                match result {
                    Ok((client, connection)) => {
                        this.cloud.client = Some(client);
                        this.cloud.connection = connection;
                        this.cloud.client_error = None;
                    }
                    Err(error) => this.cloud.client_error = Some(error),
                }
                cx.notify();
            });
        });
        let index_cache_cleanup_task = (!INDEX_CACHE_CLEANUP_SCHEDULED
            .swap(true, Ordering::AcqRel))
        .then(|| {
            cx.spawn(async move |_, cx| {
                cx.background_executor().timer(Duration::from_secs(2)).await;
                let Some(cache_root) = crate::app_paths::cache_dir() else {
                    return;
                };
                let directory = cache_root.join("VCLogg2").join("index");
                if let Err(error) = cx
                    .background_spawn(async move { vclogg_data::cleanup_index_cache(directory) })
                    .await
                {
                    log::warn!("索引缓存自动维护失败：{error:#}");
                }
            })
        });
        let state_bootstrap_task = cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let store = Arc::new(StateStore::open_default()?);
                    let recent_files = store.recent_files(8)?;
                    let pinned_files = store.pinned_files()?;
                    let last_workspace_files = store.last_workspace()?;
                    let app_settings = store.load_app_settings()?;
                    let last_settings_category = store.load_last_settings_category()?;
                    let search_panel_height = store.load_search_panel_height()?;
                    let workspace_search_state = store.load_workspace_search_state()?;
                    let color_labels = store.load_color_labels()?;
                    let global_search_preferences = store.global_search_preferences()?;
                    let search_history = store.load_search_history()?;
                    let predefined_filters = store.load_predefined_filters()?;
                    let cloud_settings = store.load_cloud_settings()?;
                    Ok::<_, anyhow::Error>((
                        store,
                        recent_files,
                        pinned_files,
                        last_workspace_files,
                        app_settings,
                        last_settings_category,
                        search_panel_height,
                        workspace_search_state,
                        color_labels,
                        global_search_preferences,
                        search_history,
                        predefined_filters,
                        cloud_settings,
                    ))
                })
                .await;

            _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok((
                        store,
                        recent_files,
                        pinned_files,
                        last_workspace_files,
                        app_settings,
                        last_settings_category,
                        search_panel_height,
                        workspace_search_state,
                        color_labels,
                        global_search_preferences,
                        search_history,
                        predefined_filters,
                        cloud_settings,
                    )) => {
                        let mut app_settings = app_settings;
                        let preserve_search_defaults = this.search_defaults_modified;
                        if preserve_search_defaults {
                            app_settings.default_case_sensitive = this.case_sensitive;
                            app_settings.default_use_regex = this.regex;
                        }
                        this.persistence.store = Some(store);
                        let pending_workspace_search_save =
                            this.persistence.pending_workspace_search_save.take();
                        cx.update_global::<WorkspaceWindowRegistry, _>(|registry, _| {
                            if !registry.last_settings_category_loaded {
                                registry.last_settings_category = last_settings_category
                                    .as_deref()
                                    .and_then(SettingsCategory::from_storage_value)
                                    .filter(|category| category.is_available())
                                    .unwrap_or_default();
                                registry.last_settings_category_loaded = true;
                            }
                        });
                        this.recent_files = recent_files;
                        this.pinned_files = pinned_files;
                        this.last_workspace_files = last_workspace_files;
                        crate::actions::apply_shortcuts(
                            &ShortcutSettings::default(),
                            &app_settings.shortcuts,
                            cx,
                        );
                        cx.set_reduce_motion(app_settings.reduce_motion);
                        crate::i18n::set_language(app_settings.language);
                        this.refresh_localized_input_copy(window, cx);
                        crate::app_log::set_level(app_settings.app_log_level);
                        Self::apply_theme_preference(app_settings.theme_preference, window, cx);
                        this.app_settings = app_settings.clone();
                        this.restore_search_panel_height(search_panel_height, window, cx);
                        this.apply_search_defaults(
                            app_settings.default_case_sensitive,
                            app_settings.default_use_regex,
                        );
                        this.search_defaults_modified = preserve_search_defaults;
                        if preserve_search_defaults {
                            this.queue_app_settings_save(app_settings.clone(), false, window, cx);
                        }
                        this.apply_color_labels(color_labels, cx);
                        if primary_window && pending_workspace_search_save.is_none() {
                            this.apply_persisted_workspace_search_state(
                                workspace_search_state,
                                window,
                                cx,
                            );
                        }
                        this.global_search
                            .replace_preferences(global_search_preferences);
                        this.search_history = search_history;
                        this.predefined_filters =
                            cx.update_global::<WorkspaceWindowRegistry, _>(|registry, _| {
                                if let Some(filters) = &registry.predefined_filters {
                                    filters.clone()
                                } else {
                                    registry.predefined_filters = Some(predefined_filters.clone());
                                    predefined_filters
                                }
                            });
                        this.cloud.settings = cloud_settings;
                        this.reset_search_history_navigation();
                        this.refresh_search_autocomplete(cx);
                        for tab in this
                            .documents
                            .iter_mut()
                            .filter(|tab| tab.uses_default_view_options)
                        {
                            tab.show_line_numbers = app_settings.default_show_line_numbers;
                            tab.show_row_separators = app_settings.default_show_row_separators;
                            tab.refresh_view_options(cx);
                        }
                        for tab in &this.documents {
                            tab.refresh_appearance(&app_settings, cx);
                            tab.refresh_word_boundary_characters(
                                &app_settings.word_boundary_characters,
                                cx,
                            );
                            tab.refresh_log_level_highlighting(
                                app_settings.highlight_log_levels,
                                cx,
                            );
                            tab.refresh_search_matcher(app_settings.highlight_matches, cx);
                        }
                        let global_matcher = this.global_result_matcher();
                        this.global_table.update(cx, |table, cx| {
                            table.delegate_mut().set_appearance(&app_settings);
                            table.delegate_mut().set_word_boundary_characters(
                                app_settings.word_boundary_characters.clone(),
                            );
                            table
                                .delegate_mut()
                                .set_highlight_log_levels(app_settings.highlight_log_levels);
                            table.delegate_mut().set_search_matcher(global_matcher);
                            table.refresh(cx);
                            cx.notify();
                        });
                        this.history_loading = false;
                        let pending_sessions = std::mem::take(&mut this.persistence.pending_sessions);
                        for (path, base, state) in pending_sessions {
                            this.save_file_session(path, base, state, window, cx);
                        }
                        if let Some(state) = pending_workspace_search_save {
                            this.queue_workspace_search_state_save(state, window, cx);
                        }
                        let open_paths = this
                            .documents
                            .iter()
                            .filter(|tab| {
                                !path_match_set_contains(
                                    &this.transient_paths,
                                    tab.document.path(),
                                )
                            })
                            .map(|tab| tab.document.path().to_path_buf())
                            .collect();
                        this.record_recent_paths(open_paths, window, cx);
                    }
                    Err(error) => {
                        this.history_loading = false;
                        window.push_notification(
                            crate::tr_args!(
                                "状态库不可用，本次仍可继续查看日志：{error}",
                                "State storage is unavailable. You can continue viewing logs: {error}",
                            ),
                            cx,
                        );
                    }
                }
                this.begin_open_initial_documents(initial_documents, window, cx);
                this.maybe_restore_persisted_search(window, cx);
                cx.notify();
            });
        });
        let mut persistence = PersistenceController::new(state_bootstrap_task);
        persistence.state_tasks.extend(index_cache_cleanup_task);

        Self {
            primary_window,
            focus_handle,
            status_surface,
            query,
            search_history: Vec::new(),
            predefined_filters: Vec::new(),
            cloud: CloudController::default(),
            search_history_ix: None,
            search_history_draft: None,
            search_autocomplete_mode: SearchAutocompleteMode::Closed,
            search_suggestion_ix: None,
            search_suggestion_scroll: UniformListScrollHandle::new(),
            quick_find: QuickFindState::new(quick_find_query),
            global_search: GlobalSearchState::new(global_result_mode_select),
            global_table,
            global_surface,
            global_viewport,
            global_text_selection_scope,
            global_results_focus_handle,
            active_log_region: LogRegion::Body,
            last_user_log_region: LogRegion::Body,
            transient_paths: BTreeSet::new(),
            pending_tab_moves: BTreeSet::new(),
            documents: Vec::new(),
            tabs: vec![WorkspaceTabId::New(1)],
            active_tab_id: WorkspaceTabId::New(1),
            active_ix: None,
            document_tab_scroll: ScrollHandle::new(),
            pending_document_tab_reveal: Cell::new(None),
            pending_directory_result_jump: None,
            next_document_id: 1,
            next_new_tab_id: 2,
            case_sensitive: false,
            regex: false,
            activity: Activity::Ready,
            selected_source_row: None,
            row_drag_bounds: BTreeMap::new(),
            row_drag_selection: None,
            row_drag_frame_scheduled: false,
            visible_line_tasks: BTreeMap::new(),
            pending_log_scroll_frames: PendingLogScrollFrames::default(),
            global_group_toggle_task: None,
            global_group_toggle_revision: 0,
            tab_activation_task: None,
            tab_activation_revision: 0,
            open_task: None,
            pending_external_paths: Vec::new(),
            searches: SearchController::default(),
            result_export_task: None,
            result_export_operation: None,
            line_copy_task: None,
            line_copy_cancellation: None,
            line_copy_revision: 0,
            color_rule_task: None,
            color_rule_cancellation: None,
            color_rule_revision: 0,
            color_labels_resolution_task: None,
            color_labels_resolution_cancellation: None,
            color_labels_resolution_revision: 0,
            file_drop_visible: false,
            file_drop_tab_transfer: None,
            cross_window_drop_ix: None,
            tab_drop_layout: Rc::new(RefCell::new(TabDropLayout::default())),
            search_panel_state,
            search_panel_height: None,
            search_panel_height_modified: false,
            search_panel_resize_gesture: None,
            search_panel_resize_bounds: Rc::new(Cell::new(None)),
            file_watch_task: None,
            deactivated_input_focus: None,
            _cloud_client_bootstrap_task: cloud_client_bootstrap_task,
            persistence,
            last_workspace_files: Vec::new(),
            pinned_files: Vec::new(),
            recent_files: Vec::new(),
            history_loading: true,
            history_dialog_loading: false,
            history_clearing: false,
            pinned_updating: false,
            app_settings: AppSettings::default(),
            scale_factor: 1.,
            color_labels: default_color_labels(),
            last_color_label_id: None,
            color_labels_saving: false,
            predefined_filters_saving: false,
            pending_predefined_filters_save: None,
            settings_saving: false,
            search_defaults_modified: false,
            subscriptions,
            history_dialog_subscription: None,
            predefined_filters_dialog_subscription: None,
            settings_dialog_subscription: None,
        }
    }

    fn active_document(&self) -> Option<&DocumentTab> {
        self.active_ix.and_then(|ix| self.documents.get(ix))
    }

    fn active_workspace_tab_ix(&self) -> Option<usize> {
        self.tabs
            .iter()
            .position(|tab_id| *tab_id == self.active_tab_id)
    }

    fn workspace_tab_title(&self, tab_id: WorkspaceTabId) -> SharedString {
        match tab_id {
            WorkspaceTabId::Document(document_id) => self
                .documents
                .iter()
                .find(|tab| tab.id == document_id)
                .map(|tab| tab.title.clone())
                .unwrap_or_else(|| crate::tr!("日志", "Log").into()),
            WorkspaceTabId::New(_) => crate::tr!("新标签页", "New tab").into(),
        }
    }

    fn sync_active_document_ix(&mut self) {
        self.active_ix = self
            .active_tab_id
            .document_id()
            .and_then(|document_id| self.documents.iter().position(|tab| tab.id == document_id));
    }

    fn reorder_documents_to_match_tabs(&mut self) {
        let document_order = self
            .tabs
            .iter()
            .filter_map(|tab_id| tab_id.document_id())
            .enumerate()
            .map(|(ix, document_id)| (document_id, ix))
            .collect::<BTreeMap<_, _>>();
        self.documents
            .sort_by_key(|tab| document_order.get(&tab.id).copied().unwrap_or(usize::MAX));
        self.sync_active_document_ix();
    }

    fn create_new_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let tab_id = WorkspaceTabId::New(self.next_new_tab_id);
        self.next_new_tab_id = self.next_new_tab_id.saturating_add(1);
        self.tabs.push(tab_id);
        self.activate_workspace_tab(tab_id, window, cx);
    }

    fn open_files(&mut self, _: &OpenFiles, window: &mut Window, cx: &mut Context<Self>) {
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some(crate::tr!("选择日志文件", "Select log files").into()),
        });

        self.open_task = Some(cx.spawn_in(window, async move |this, cx| {
            let paths = prompt.await.ok().and_then(Result::ok).flatten();
            _ = this.update_in(cx, |this, window, cx| {
                this.open_task = None;
                if let Some(paths) = paths {
                    this.begin_open_paths(paths, window, cx);
                }
                this.open_queued_external_paths_if_idle(window, cx);
            });
        }));
    }

    fn open_recent_file(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.begin_open_paths(vec![path], window, cx);
    }

    fn open_dropped_paths(
        &mut self,
        paths: &ExternalPaths,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.file_drop_visible = false;
        self.file_drop_tab_transfer = None;
        if self.open_task.is_some() {
            window.push_notification(
                crate::tr!(
                    "当前正在打开其他文件，请稍后再拖入",
                    "Another file is being opened. Drop files again shortly.",
                ),
                cx,
            );
            return;
        }
        let (files, ignored_count): (Vec<_>, Vec<_>) = paths
            .paths()
            .iter()
            .cloned()
            .partition(|path| !path.is_dir());
        if files.is_empty() {
            window.push_notification(
                crate::tr!("请拖入一个或多个日志文件", "Drop one or more log files"),
                cx,
            );
            return;
        }
        if !ignored_count.is_empty() {
            window.push_notification(
                crate::tr_args!(
                    "已忽略 {} 个文件夹",
                    "Ignored {} folders",
                    ignored_count.len(),
                ),
                cx,
            );
        }
        self.begin_open_paths(files, window, cx);
    }

    fn restore_last_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = self
            .last_workspace_files
            .iter()
            .map(|file| file.path.clone())
            .collect::<Vec<_>>();
        let active_path = self
            .last_workspace_files
            .iter()
            .find(|file| file.was_active)
            .map(|file| file.path.clone());
        self.begin_open_paths_with_active(paths, active_path, window, cx);
    }

    fn begin_open_paths(
        &mut self,
        paths: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.begin_open_paths_with_active(paths, None, window, cx);
    }

    fn enqueue_external_paths(
        &mut self,
        paths: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for path in paths {
            if !path.as_os_str().is_empty()
                && !self
                    .pending_external_paths
                    .iter()
                    .any(|queued| paths_match(queued, &path))
            {
                self.pending_external_paths.push(path);
            }
        }
        self.open_queued_external_paths_if_idle(window, cx);
    }

    fn open_queued_external_paths_if_idle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.open_task.is_some() || self.pending_external_paths.is_empty() {
            return;
        }
        let paths = std::mem::take(&mut self.pending_external_paths);
        self.begin_open_paths(paths, window, cx);
    }

    fn begin_open_initial_documents(
        &mut self,
        initial_documents: Vec<InitialDocument>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if initial_documents.is_empty() {
            return;
        }
        let mut overrides = OpenDocumentOverrides::default();
        let mut paths = Vec::with_capacity(initial_documents.len());
        let replace_new_tab = initial_documents
            .iter()
            .all(|initial| initial.replace_new_tab);
        for initial in initial_documents {
            if initial.transient {
                self.transient_paths.insert(path_match_key(&initial.path));
            }
            if let Some(completion) = initial.move_completion {
                overrides
                    .move_completions
                    .insert(initial.path.clone(), completion);
            }
            if let Some(target_ix) = initial.target_ix {
                path_buf_map_insert(
                    &mut overrides.target_indices,
                    initial.path.clone(),
                    target_ix,
                );
            }
            if let Some(session) = initial.session {
                path_buf_map_insert(&mut overrides.sessions, initial.path.clone(), session);
            }
            paths.push(initial.path);
        }
        let active_path = paths.last().cloned();
        self.begin_open_paths_with_sessions(
            paths,
            active_path,
            overrides,
            replace_new_tab,
            window,
            cx,
        );
    }

    fn begin_open_paths_with_active(
        &mut self,
        paths: Vec<PathBuf>,
        active_path: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.begin_open_paths_with_sessions(
            paths,
            active_path,
            OpenDocumentOverrides::default(),
            true,
            window,
            cx,
        );
    }

    fn begin_open_paths_with_sessions(
        &mut self,
        paths: Vec<PathBuf>,
        active_path: Option<PathBuf>,
        mut overrides: OpenDocumentOverrides,
        replace_new_tab: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let paths = deduplicate_paths(
            paths
                .into_iter()
                .filter(|path| !path.as_os_str().is_empty()),
        );
        if paths.is_empty() || self.open_task.is_some() {
            return;
        }
        for path in &paths {
            if path_buf_map_get(&overrides.sessions, path).is_none()
                && let Some(session) =
                    path_buf_map_get(&self.persistence.pending_session_overrides, path)
            {
                path_buf_map_insert(&mut overrides.sessions, path.clone(), session.clone());
            }
        }
        self.activity = Activity::Opening;
        let replacement_new_tab_id =
            replace_new_tab
                .then_some(self.active_tab_id)
                .and_then(|tab_id| match tab_id {
                    WorkspaceTabId::New(id) => Some(id),
                    WorkspaceTabId::Document(_) => None,
                });
        let shells = paths
            .iter()
            .map(|path| {
                (
                    path.clone(),
                    Ok(prepare_document_shell(
                        path,
                        path_buf_map_get(&overrides.sessions, path).cloned(),
                    )),
                )
            })
            .collect::<Vec<_>>();
        self.install_documents(
            shells,
            active_path.as_deref(),
            &overrides.target_indices,
            replacement_new_tab_id,
            false,
            window,
            cx,
        );
        let paths = paths
            .into_iter()
            .filter(|path| {
                self.documents
                    .iter()
                    .find(|tab| paths_match(tab.document.path(), path))
                    .is_some_and(|tab| tab.load_state != DocumentLoadState::Ready)
            })
            .collect::<Vec<_>>();
        if paths.is_empty() {
            self.activity = Activity::Ready;
            self.maybe_restore_persisted_search(window, cx);
            for (path, completion) in overrides.move_completions {
                let installed = self.documents.iter().any(|tab| {
                    paths_match(tab.document.path(), &path)
                        && tab.load_state == DocumentLoadState::Ready
                });
                completion.finish(installed, cx);
            }
            self.open_queued_external_paths_if_idle(window, cx);
            cx.notify();
            return;
        }
        let opening_ids = paths
            .iter()
            .filter_map(|path| {
                self.documents
                    .iter()
                    .find(|tab| {
                        paths_match(tab.document.path(), path)
                            && matches!(
                                tab.load_state,
                                DocumentLoadState::Opening
                                    | DocumentLoadState::Preview
                                    | DocumentLoadState::IndexFailed
                            )
                    })
                    .map(|tab| (path.clone(), tab.id))
            })
            .collect::<BTreeMap<_, _>>();
        let state_store = self.persistence.store.clone();
        let OpenDocumentOverrides {
            sessions,
            move_completions,
            target_indices,
        } = overrides;
        let search_result_limit = self.app_settings.search_result_limit();
        let color_labels = self.color_labels.clone();

        self.open_task = Some(cx.spawn_in(window, async move |this, cx| {
            let restore_paths = paths.clone();
            let restore_store = state_store.clone();
            let (sessions, fallback_store, effective_search_result_limit) = cx
                .background_spawn(async move {
                    let effective_search_result_limit = restore_store
                        .as_deref()
                        .and_then(|store| store.load_app_settings().ok())
                        .map(|settings| settings.search_result_limit())
                        .unwrap_or(search_result_limit);
                    let (sessions, fallback_store) = match restore_store {
                        Some(store) => match store.load_sessions(&restore_paths) {
                            Ok(mut restored) => {
                                for (path, session) in sessions {
                                    path_buf_map_insert(&mut restored, path, session);
                                }
                                (restored, None)
                            }
                            Err(_) => (sessions, Some(store)),
                        },
                        None => (sessions, None),
                    };
                    (sessions, fallback_store, effective_search_result_limit)
                })
                .await;

            let preview_paths = paths.clone();
            let preview_sessions = sessions.clone();
            let preview_store = fallback_store.clone();
            let preview_color_labels = color_labels.clone();
            let previews = cx
                .background_spawn(async move {
                    prepare_paths_bounded(preview_paths, |path| {
                        prepare_document_preview(
                            path,
                            preview_store.as_deref(),
                            path_buf_map_get(&preview_sessions, path).cloned(),
                            effective_search_result_limit,
                            &preview_color_labels,
                        )
                    })
                })
                .await;

            _ = this.update_in(cx, |this, window, cx| {
                let previews = previews
                    .into_iter()
                    .filter(|(path, _)| {
                        path_buf_map_get(&opening_ids, path).is_some_and(|expected_id| {
                            this.documents.iter().any(|tab| {
                                tab.id == *expected_id
                                    && paths_match(tab.document.path(), path)
                                    && matches!(
                                        tab.load_state,
                                        DocumentLoadState::Opening | DocumentLoadState::IndexFailed
                                    )
                            })
                        })
                    })
                    .collect();
                this.install_documents(
                    previews,
                    active_path.as_deref(),
                    &target_indices,
                    None,
                    false,
                    window,
                    cx,
                );
            });

            let full_paths = paths.clone();
            let full_store = fallback_store;
            let opened = cx
                .background_spawn(async move {
                    prepare_paths_bounded(full_paths, |path| {
                        prepare_document(
                            path,
                            full_store.as_deref(),
                            path_buf_map_get(&sessions, path).cloned(),
                            effective_search_result_limit,
                            &color_labels,
                        )
                    })
                })
                .await;

            _ = this.update_in(cx, |this, window, cx| {
                this.install_completed_documents(
                    opened,
                    active_path.as_deref(),
                    &target_indices,
                    &opening_ids,
                    window,
                    cx,
                );
                this.open_task = None;
                this.maybe_restore_persisted_search(window, cx);
                for (path, completion) in move_completions {
                    let installed = this.documents.iter().any(|tab| {
                        paths_match(tab.document.path(), &path)
                            && tab.load_state == DocumentLoadState::Ready
                    });
                    completion.finish(installed, cx);
                }
                this.open_queued_external_paths_if_idle(window, cx);
            });
        }));
    }

    fn record_recent_paths(
        &mut self,
        paths: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(store) = self.persistence.store.clone() else {
            return;
        };
        if paths.is_empty() {
            return;
        }

        self.persistence
            .state_tasks
            .push(cx.spawn_in(window, async move |this, cx| {
                let result = cx
                    .background_spawn(async move {
                        store.record_opened(&paths)?;
                        Ok::<_, anyhow::Error>((store.recent_files(8)?, store.pinned_files()?))
                    })
                    .await;
                _ = this.update_in(cx, |this, window, cx| match result {
                    Ok((recent_files, pinned_files)) => {
                        this.recent_files = recent_files;
                        this.pinned_files = pinned_files;
                        cx.notify();
                    }
                    Err(error) => window.push_notification(
                        crate::tr_args!(
                            "最近文件未能保存：{error}",
                            "Couldn’t save recent files: {error}"
                        ),
                        cx,
                    ),
                });
            }));
    }

    fn active_file_is_pinned(&self) -> bool {
        self.active_document().is_some_and(|tab| {
            self.pinned_files
                .iter()
                .any(|file| paths_match(&file.path, tab.document.path()))
        })
    }

    fn toggle_active_file_pinned(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self
            .active_document()
            .map(|tab| tab.document.path().to_path_buf())
        else {
            return;
        };
        let Some(store) = self.persistence.store.clone() else {
            return;
        };
        if self.pinned_updating {
            return;
        }
        let pinned = !self.active_file_is_pinned();
        self.pinned_updating = true;
        cx.notify();
        self.persistence
            .state_tasks
            .push(cx.spawn_in(window, async move |this, cx| {
                let result = cx
                    .background_spawn(async move {
                        store.set_pinned(&path, pinned)?;
                        Ok::<_, anyhow::Error>((store.recent_files(8)?, store.pinned_files()?))
                    })
                    .await;
                _ = this.update_in(cx, |this, window, cx| {
                    this.pinned_updating = false;
                    match result {
                        Ok((recent_files, pinned_files)) => {
                            this.recent_files = recent_files;
                            this.pinned_files = pinned_files;
                        }
                        Err(error) => window.push_notification(
                            crate::tr_args!(
                                "收藏状态未能保存：{error}",
                                "Couldn’t save favorite status: {error}"
                            ),
                            cx,
                        ),
                    }
                    cx.notify();
                });
            }));
    }

    fn clear_pinned_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(store) = self.persistence.store.clone() else {
            return;
        };
        if self.pinned_updating || self.pinned_files.is_empty() {
            return;
        }
        self.pinned_updating = true;
        cx.notify();
        self.persistence
            .state_tasks
            .push(cx.spawn_in(window, async move |this, cx| {
                let result = cx
                    .background_spawn(async move {
                        store.clear_pinned()?;
                        store.recent_files(8)
                    })
                    .await;
                _ = this.update_in(cx, |this, window, cx| {
                    this.pinned_updating = false;
                    match result {
                        Ok(recent_files) => {
                            this.recent_files = recent_files;
                            this.pinned_files.clear();
                        }
                        Err(error) => window.push_notification(
                            crate::tr_args!(
                                "收藏未能清空：{error}",
                                "Couldn’t clear favorites: {error}"
                            ),
                            cx,
                        ),
                    }
                    cx.notify();
                });
            }));
    }

    fn open_history_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(store) = self.persistence.store.clone() else {
            window.push_notification(
                crate::tr!("状态库尚未就绪", "State storage is not ready"),
                cx,
            );
            return;
        };
        if self.history_dialog_loading {
            return;
        }
        let current_workspace_id = cx.entity_id();
        let workspaces = cx
            .global::<WorkspaceWindowRegistry>()
            .windows
            .iter()
            .map(|entry| entry.workspace.clone())
            .collect::<Vec<_>>();
        let mut open_paths = self
            .documents
            .iter()
            .map(|tab| tab.document.path().to_path_buf())
            .collect::<Vec<_>>();
        for workspace in workspaces {
            if workspace.entity_id() == current_workspace_id {
                continue;
            }
            open_paths.extend(
                workspace
                    .read(cx)
                    .documents
                    .iter()
                    .map(|tab| tab.document.path().to_path_buf()),
            );
        }
        self.history_dialog_loading = true;
        cx.notify();
        self.persistence
            .state_tasks
            .push(cx.spawn_in(window, async move |this, cx| {
                let store_for_query = store.clone();
                let result = cx
                    .background_spawn(async move {
                        Ok::<_, anyhow::Error>((
                            store_for_query.session_history()?,
                            store_for_query.database_info()?,
                            result_export::temporary_result_files()?,
                        ))
                    })
                    .await;
                _ = this.update_in(cx, |this, window, cx| {
                    this.history_dialog_loading = false;
                    match result {
                        Ok((sessions, database_info, temporary_results)) => {
                            let history = cx.new(|cx| {
                                HistoryDialog::new(
                                    sessions,
                                    database_info,
                                    temporary_results,
                                    open_paths,
                                    store.clone(),
                                    window,
                                    cx,
                                )
                            });
                            this.history_dialog_subscription = Some(cx.subscribe_in(
                                &history,
                                window,
                                |this, _, event: &HistoryDialogEvent, window, cx| match event {
                                    HistoryDialogEvent::Open(path) => {
                                        // GPUI queues emitted events, so the owner closes the
                                        // dialog only after this subscription receives the event.
                                        window.close_dialog(cx);
                                        this.open_recent_file(path.clone(), window, cx);
                                    }
                                    HistoryDialogEvent::ClearHistory => {
                                        window.close_dialog(cx);
                                        this.confirm_clear_history(window, cx);
                                    }
                                    HistoryDialogEvent::HistoryChanged {
                                        recent_files,
                                        pinned_files,
                                        last_workspace_files,
                                    } => {
                                        this.recent_files = recent_files.clone();
                                        this.pinned_files = pinned_files.clone();
                                        this.last_workspace_files = last_workspace_files.clone();
                                        cx.notify();
                                    }
                                },
                            ));
                            let (history_dialog_size, history_dialog_margin_top) =
                                management_dialog_geometry(window);
                            window.open_dialog(cx, move |dialog, _, _| {
                                let history = history.clone();
                                dialog
                                    .w(history_dialog_size.width)
                                    .h(history_dialog_size.height)
                                    .margin_top(history_dialog_margin_top)
                                    .title(crate::tr!("文件历史", "File history"))
                                    .content(move |content, _, _| {
                                        content.min_h_0().overflow_hidden().child(history.clone())
                                    })
                                    .button_props(
                                        DialogButtonProps::default()
                                            .ok_text(crate::tr!("关闭", "Close")),
                                    )
                            });
                        }
                        Err(error) => window.push_notification(
                            crate::tr_args!(
                                "历史记录未能读取：{error}",
                                "Couldn’t read history: {error}"
                            ),
                            cx,
                        ),
                    }
                    cx.notify();
                });
            }));
    }

    fn remember_settings_category(
        &mut self,
        category: SettingsCategory,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !category.is_available() {
            return;
        }
        let changed = cx.update_global::<WorkspaceWindowRegistry, _>(|registry, _| {
            let changed = !registry.last_settings_category_loaded
                || registry.last_settings_category != category;
            registry.last_settings_category = category;
            registry.last_settings_category_loaded = true;
            changed
        });
        if !changed {
            return;
        }
        let Some(store) = self.persistence.store.clone() else {
            return;
        };
        let category = category.storage_value().to_string();
        let previous_save = self.persistence.settings_category_save_task.take();
        self.persistence.settings_category_save_task =
            Some(cx.spawn_in(window, async move |this, cx| {
                if let Some(previous_save) = previous_save {
                    previous_save.await;
                }
                let result = cx
                    .background_spawn(async move { store.save_last_settings_category(&category) })
                    .await;
                if let Err(error) = result {
                    _ = this.update_in(cx, |_, window, cx| {
                        window.push_notification(
                            crate::tr_args!(
                                "设置页位置未能保存：{error}",
                                "Couldn’t save the settings page position: {error}"
                            ),
                            cx,
                        );
                    });
                }
            }));
    }

    fn restore_search_panel_height(
        &mut self,
        stored_height: Option<f32>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.search_panel_height_modified {
            if let Some(height) = self.search_panel_height {
                self.remember_search_panel_height(height, window, cx);
            }
            return;
        }
        let Some(height) = stored_height.map(px) else {
            return;
        };
        self.search_panel_height = Some(height);
        if self.search_panel_state.read(cx).sizes().len() < 2 {
            return;
        }
        self.search_panel_state.update(cx, |state, cx| {
            state.resize_panel(1, height, window, cx);
        });
    }

    fn remember_search_panel_height(
        &mut self,
        height: Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.search_panel_height = Some(height);
        self.search_panel_height_modified = true;
        let Some(store) = self.persistence.store.clone() else {
            return;
        };
        let previous_save = self.persistence.search_panel_height_save_task.take();
        self.persistence.search_panel_height_save_task =
            Some(cx.spawn_in(window, async move |this, cx| {
                if let Some(previous_save) = previous_save {
                    previous_save.await;
                }
                let result = cx
                    .background_spawn(
                        async move { store.save_search_panel_height(height.as_f32()) },
                    )
                    .await;
                if let Err(error) = result {
                    _ = this.update_in(cx, |_, window, cx| {
                        window.push_notification(
                            crate::tr_args!(
                                "搜索面板高度未能保存：{error}",
                                "Couldn’t save the search panel height: {error}"
                            ),
                            cx,
                        );
                    });
                }
            }));
    }

    fn resize_search_panel_from_drag(
        &mut self,
        pointer_y: Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(gesture) = self.search_panel_resize_gesture else {
            return;
        };

        let requested_height = gesture.initial_height + gesture.start_y - pointer_y;
        self.search_panel_state.update(cx, |state, cx| {
            state.resize_panel(1, requested_height, window, cx);
        });
        let height = self.search_panel_state.read(cx).sizes().get(1).copied();
        if self.search_panel_height != height {
            self.search_panel_height = height;
            cx.notify();
        }
    }

    fn finish_search_panel_resize(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.search_panel_resize_gesture.take().is_none() {
            return false;
        }

        let height = self
            .search_panel_state
            .read(cx)
            .sizes()
            .get(1)
            .copied()
            .or(self.search_panel_height);
        if let Some(height) = height {
            self.remember_search_panel_height(height, window, cx);
        }
        true
    }

    fn render_search_panel_resize_event_layer(&self, cx: &mut Context<Self>) -> AnyElement {
        let resize_bounds = self.search_panel_resize_bounds.clone();
        let workspace = cx.weak_entity();

        canvas(
            |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
            move |_, event_hitbox, window, _cx| {
                window.on_mouse_event({
                    let resize_bounds = resize_bounds.clone();
                    let workspace = workspace.clone();
                    move |event: &MouseDownEvent, phase, window, cx| {
                        if phase.bubble()
                            || event.button != MouseButton::Left
                            || !event_hitbox.is_hovered(window)
                        {
                            return;
                        }
                        let Some(bounds) = resize_bounds.get() else {
                            return;
                        };
                        if !bounds.contains(&event.position) {
                            return;
                        }

                        let started = workspace
                            .update(cx, |workspace, cx| {
                                let initial_height = workspace
                                    .search_panel_state
                                    .read(cx)
                                    .sizes()
                                    .get(1)
                                    .copied()
                                    .or(workspace.search_panel_height)
                                    .unwrap_or(window.rem_size() * 16.);
                                workspace.search_panel_resize_gesture =
                                    Some(SearchPanelResizeGesture {
                                        start_y: event.position.y,
                                        initial_height,
                                    });
                            })
                            .is_ok();
                        if started {
                            window.capture_pointer(event_hitbox.id);
                            cx.stop_propagation();
                        }
                    }
                });

                window.on_mouse_event({
                    let workspace = workspace.clone();
                    move |event: &MouseMoveEvent, phase, window, cx| {
                        if phase.bubble() {
                            return;
                        }

                        let mut release_pointer = false;
                        let consumed = workspace
                            .update(cx, |workspace, cx| {
                                if workspace.search_panel_resize_gesture.is_none() {
                                    return false;
                                }
                                if event.dragging() {
                                    workspace.resize_search_panel_from_drag(
                                        event.position.y,
                                        window,
                                        cx,
                                    );
                                } else {
                                    release_pointer =
                                        workspace.finish_search_panel_resize(window, cx);
                                }
                                true
                            })
                            .unwrap_or(false);
                        if release_pointer {
                            window.release_pointer();
                        }
                        if consumed {
                            cx.stop_propagation();
                        }
                    }
                });

                window.on_mouse_event({
                    move |event: &MouseUpEvent, phase, window, cx| {
                        if phase.bubble() || event.button != MouseButton::Left {
                            return;
                        }
                        let consumed = workspace
                            .update(cx, |workspace, cx| {
                                workspace.finish_search_panel_resize(window, cx)
                            })
                            .unwrap_or(false);
                        if consumed {
                            cx.stop_propagation();
                        }
                    }
                });
            },
        )
        // This is the second child of a block container. Explicitly anchor it so
        // Taffy does not place the full-size event layer at its post-content static position.
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .into_any_element()
    }

    fn open_settings_dialog(
        &mut self,
        requested_category: Option<SettingsCategory>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.settings_saving {
            return;
        }
        let active_category = requested_category
            .filter(|category| category.is_available())
            .unwrap_or_else(|| {
                cx.global::<WorkspaceWindowRegistry>()
                    .last_settings_category
            });
        self.remember_settings_category(active_category, window, cx);
        let original_settings = self.app_settings.clone();
        let original_search_history = self.search_history.clone();
        let settings = cx.new(|cx| {
            SettingsDialog::new(
                self.app_settings.clone(),
                original_search_history.clone(),
                SettingsNetworkSnapshot {
                    settings: self.cloud.settings.clone(),
                    client: self.cloud.client.clone(),
                    connection: self.cloud.connection.clone(),
                    client_error: self.cloud.client_error.clone(),
                },
                active_category,
                window,
                cx,
            )
        });
        self.settings_dialog_subscription = Some(cx.subscribe_in(
            &settings,
            window,
            |this, settings, event: &SettingsDialogEvent, window, cx| match event {
                SettingsDialogEvent::DraftChanged => {
                    let draft = {
                        let settings = settings.read(cx);
                        let Ok(draft) = settings.settings(cx) else {
                            return;
                        };
                        draft
                    };
                    this.preview_app_settings(draft, window, cx);
                }
                SettingsDialogEvent::CategoryChanged(category) => {
                    this.remember_settings_category(*category, window, cx)
                }
                SettingsDialogEvent::CloudSettings(settings) => {
                    this.save_cloud_settings(settings.clone(), window, cx)
                }
                SettingsDialogEvent::CloudConnection(connection) => {
                    this.cloud.connection = connection.clone();
                    cx.notify();
                }
            },
        ));
        let workspace = cx.entity();
        let (settings_dialog_size, settings_dialog_margin_top) = management_dialog_geometry(window);
        window.open_dialog(cx, move |dialog, _, _| {
            let settings = settings.clone();
            let workspace_for_save = workspace.clone();
            let workspace_for_cancel = workspace.clone();
            let workspace_for_close = workspace.clone();
            let original_settings = original_settings.clone();
            let original_search_history = original_search_history.clone();
            dialog
                .w(settings_dialog_size.width)
                .h(settings_dialog_size.height)
                .margin_top(settings_dialog_margin_top)
                .title(crate::tr!("设置", "Settings"))
                .child(settings.clone())
                .footer(
                    h_flex()
                        .w_full()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("settings-dialog-cancel")
                                .label(crate::tr!("取消", "Cancel"))
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(Box::new(Cancel), cx)
                                }),
                        )
                        .child(
                            Button::new("settings-dialog-save")
                                .primary()
                                .label(crate::tr!("保存", "Save"))
                                .on_click(|_, window, cx| {
                                    window
                                        .dispatch_action(Box::new(Confirm { secondary: false }), cx)
                                }),
                        ),
                )
                .on_ok(move |_, window, cx| {
                    let (draft, search_history, network_settings) = {
                        let settings = settings.read(cx);
                        let draft = match settings.settings(cx) {
                            Ok(draft) => draft,
                            Err(error) => {
                                window.push_notification(error, cx);
                                return false;
                            }
                        };
                        (
                            draft,
                            settings.search_history(),
                            settings.network_settings(cx),
                        )
                    };
                    let retained = search_history
                        .iter()
                        .map(String::as_str)
                        .collect::<HashSet<_>>();
                    let removed = original_search_history
                        .iter()
                        .filter(|query| !retained.contains(query.as_str()))
                        .cloned()
                        .collect::<Vec<_>>();
                    workspace_for_save.update(cx, |this, cx| {
                        this.save_app_settings(draft, window, cx);
                        this.save_cloud_settings(network_settings, window, cx);
                        this.remove_search_history_entries(&removed, window, cx);
                    });
                    true
                })
                .on_cancel(move |_, window, cx| {
                    workspace_for_cancel.update(cx, |this, cx| {
                        this.preview_app_settings(original_settings.clone(), window, cx);
                    });
                    true
                })
                .on_close(move |_, _, cx| {
                    workspace_for_close.update(cx, |this, _| {
                        this.settings_dialog_subscription = None;
                    });
                })
        });
    }

    fn apply_color_labels(&mut self, labels: Vec<ColorLabel>, cx: &mut Context<Self>) {
        self.cancel_color_rule_action();
        self.cancel_color_labels_resolution();
        self.color_labels = labels;
        if self
            .last_color_label_id
            .as_ref()
            .is_some_and(|id| self.color_labels.iter().all(|label| &label.id != id))
        {
            self.last_color_label_id = None;
        }
        let revision = self.color_labels_resolution_revision;
        let labels = self.color_labels.clone();
        let inputs = self
            .documents
            .iter()
            .map(|tab| ColorRuleResolutionInput {
                document_id: tab.id,
                document: tab.document.clone(),
                rules: tab.keyword_color_rules.clone(),
            })
            .collect();
        let cancellation = SearchCancellation::default();
        self.color_labels_resolution_cancellation = Some(cancellation.clone());
        self.color_labels_resolution_task = Some(cx.spawn(async move |this, cx| {
            let prepared = cx
                .background_spawn(async move {
                    prepare_color_rule_resolutions(inputs, &labels, &cancellation)
                        .map(|prepared| (labels, prepared))
                })
                .await;
            _ = this.update(cx, |this, cx| {
                if this.color_labels_resolution_revision != revision {
                    return;
                }
                this.color_labels_resolution_task = None;
                this.color_labels_resolution_cancellation = None;
                let Some((labels, prepared)) = prepared else {
                    return;
                };
                if this.color_labels != labels {
                    return;
                }
                for prepared in prepared {
                    let Some(tab) = this.documents.iter_mut().find(|tab| {
                        tab.id == prepared.document_id
                            && Arc::ptr_eq(&tab.document, &prepared.document)
                            && tab.keyword_color_rules == prepared.rules
                    }) else {
                        continue;
                    };
                    tab.resolved_color_rules = prepared.resolved.clone();
                    for table in [tab.log_table.clone(), tab.result_table.clone()] {
                        table.update(cx, |table, cx| {
                            table
                                .delegate_mut()
                                .set_color_rules(prepared.resolved.clone());
                            table.refresh(cx);
                        });
                    }
                }
                this.refresh_global_color_rules(cx);
                cx.notify();
            });
        }));
    }

    fn cancel_color_labels_resolution(&mut self) {
        self.color_labels_resolution_revision =
            self.color_labels_resolution_revision.saturating_add(1);
        if let Some(cancellation) = self.color_labels_resolution_cancellation.take() {
            cancellation.cancel();
        }
        self.color_labels_resolution_task = None;
    }

    fn refresh_global_color_rules(&mut self, cx: &mut Context<Self>) {
        let color_rules_by_path = self
            .documents
            .iter()
            .map(|tab| {
                (
                    path_match_key(tab.document.path()),
                    (tab.document.clone(), tab.resolved_color_rules.clone()),
                )
            })
            .collect::<BTreeMap<_, _>>();
        self.global_table.update(cx, |table, cx| {
            table.delegate_mut().update_color_rules(|source| {
                path_match_map_get(&color_rules_by_path, &source.path)
                    .filter(|(document, _)| {
                        result_snapshot_matches_document(&source.path, &source.document, document)
                    })
                    .map(|(_, color_rules)| color_rules.clone())
                    .unwrap_or_default()
            });
            table.refresh(cx);
        });
    }

    fn open_color_labels_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.color_labels_saving {
            return;
        }
        let labels = cx.new(|cx| ColorLabelsDialog::new(self.color_labels.clone(), window, cx));
        let workspace = cx.entity();
        let (color_labels_dialog_size, color_labels_dialog_margin_top) =
            management_dialog_geometry(window);
        window.open_dialog(cx, move |dialog, _, _| {
            let labels = labels.clone();
            let workspace = workspace.clone();
            dialog
                .w(color_labels_dialog_size.width)
                .h(color_labels_dialog_size.height)
                .margin_top(color_labels_dialog_margin_top)
                .title(crate::tr!("颜色标签", "Color labels"))
                .child(labels.clone())
                .footer(
                    DialogFooter::new()
                        .child(
                            DialogClose::new().child(
                                Button::new("color-label-dialog-cancel")
                                    .label(crate::tr!("取消", "Cancel")),
                            ),
                        )
                        .child(
                            DialogAction::new().child(
                                Button::new("color-label-dialog-save")
                                    .primary()
                                    .label(crate::tr!("保存", "Save")),
                            ),
                        ),
                )
                .on_ok(move |_, window, cx| {
                    let draft = match labels.read(cx).labels(cx) {
                        Ok(draft) => draft,
                        Err(error) => {
                            window.push_notification(error, cx);
                            return false;
                        }
                    };
                    workspace.update(cx, |this, cx| {
                        this.save_color_labels(draft, window, cx);
                    });
                    true
                })
        });
    }

    fn save_color_labels(
        &mut self,
        labels: Vec<ColorLabel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(store) = self.persistence.store.clone() else {
            window.push_notification(
                crate::tr!(
                    "状态库尚未就绪，颜色标签未保存",
                    "State storage is not ready; color labels weren’t saved"
                ),
                cx,
            );
            return;
        };
        self.apply_color_labels(labels.clone(), cx);
        let source_window = window.window_handle();
        let other_workspaces = cx
            .global::<WorkspaceWindowRegistry>()
            .windows
            .iter()
            .filter(|entry| entry.window != source_window)
            .map(|entry| entry.workspace.clone())
            .collect::<Vec<_>>();
        for workspace in other_workspaces {
            let shared_labels = labels.clone();
            workspace.update(cx, |workspace, cx| {
                workspace.apply_color_labels(shared_labels, cx)
            });
        }
        self.color_labels_saving = true;
        cx.notify();
        self.persistence
            .state_tasks
            .push(cx.spawn_in(window, async move |this, cx| {
                let result = cx
                    .background_spawn(async move { store.save_color_labels(&labels) })
                    .await;
                _ = this.update_in(cx, |this, window, cx| {
                    this.color_labels_saving = false;
                    match result {
                        Ok(()) => window.push_notification(
                            crate::tr!("颜色标签已保存", "Color labels saved"),
                            cx,
                        ),
                        Err(error) => window.push_notification(
                            crate::tr_args!(
                                "颜色标签未能保存：{error}",
                                "Couldn’t save color labels: {error}"
                            ),
                            cx,
                        ),
                    }
                    cx.notify();
                });
            }));
    }

    fn open_predefined_filters_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.predefined_filters_saving {
            return;
        }
        let filters = cx.new(|cx| {
            PredefinedFiltersDialog::new(
                self.predefined_filters.clone(),
                self.cloud.settings.clone(),
                self.cloud.client.clone(),
                self.cloud.connection.clone(),
                self.cloud.client_error.clone(),
                window,
                cx,
            )
        });
        self.predefined_filters_dialog_subscription = Some(cx.subscribe_in(
            &filters,
            window,
            |this, _, event: &PredefinedFiltersDialogEvent, window, cx| match event {
                PredefinedFiltersDialogEvent::Filters(filters) => {
                    this.save_predefined_filters(filters.clone(), window, cx)
                }
                PredefinedFiltersDialogEvent::CloudSettings(settings) => {
                    this.save_cloud_settings(settings.clone(), window, cx)
                }
                PredefinedFiltersDialogEvent::CloudConnection(connection) => {
                    this.cloud.connection = connection.clone();
                    cx.notify();
                }
            },
        ));
        let workspace = cx.entity();
        let (predefined_filters_dialog_size, predefined_filters_dialog_margin_top) =
            management_dialog_geometry(window);
        window.open_dialog(cx, move |dialog, _, _| {
            let filters = filters.clone();
            let content_filters = filters.clone();
            let workspace = workspace.clone();
            dialog
                .w(predefined_filters_dialog_size.width)
                .h(predefined_filters_dialog_size.height)
                .margin_top(predefined_filters_dialog_margin_top)
                .title(crate::tr!("预定义过滤器", "Predefined filters"))
                .content(move |content, _, _| {
                    content
                        .p_0()
                        .min_h_0()
                        .overflow_hidden()
                        .child(content_filters.clone())
                })
                .on_ok(move |_, window, cx| {
                    if !filters.read(cx).accepts_confirm() {
                        return false;
                    }
                    let draft = match filters.read(cx).filters(cx) {
                        Ok(draft) => draft,
                        Err(error) => {
                            window.push_notification(error, cx);
                            return false;
                        }
                    };
                    workspace.update(cx, |this, cx| {
                        this.save_predefined_filters(draft, window, cx);
                    });
                    true
                })
        });
    }

    fn apply_predefined_filters(&mut self, filters: Vec<PredefinedFilter>, cx: &mut Context<Self>) {
        self.predefined_filters = filters;
        self.refresh_search_autocomplete(cx);
    }

    fn save_predefined_filters(
        &mut self,
        filters: Vec<PredefinedFilter>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let revision = PREDEFINED_FILTERS_SAVE_REVISION.fetch_add(1, Ordering::AcqRel) + 1;
        cx.update_global::<WorkspaceWindowRegistry, _>(|registry, _| {
            registry.predefined_filters = Some(filters.clone());
        });
        self.apply_predefined_filters(filters.clone(), cx);
        let source_window = window.window_handle();
        let other_workspaces = cx
            .global::<WorkspaceWindowRegistry>()
            .windows
            .iter()
            .filter(|entry| entry.window != source_window)
            .map(|entry| entry.workspace.clone())
            .collect::<Vec<_>>();
        for workspace in other_workspaces {
            let shared_filters = filters.clone();
            workspace.update(cx, |workspace, cx| {
                workspace.apply_predefined_filters(shared_filters, cx)
            });
        }
        if self.persistence.store.is_none() {
            window.push_notification(
                crate::tr!(
                    "过滤器已应用，但状态库尚未就绪，未持久保存",
                    "The filter was applied but not persisted because state storage is not ready"
                ),
                cx,
            );
            return;
        }
        if self.predefined_filters_saving {
            self.pending_predefined_filters_save = Some((revision, filters));
            cx.notify();
            return;
        }
        self.persist_predefined_filters(revision, filters, window, cx);
    }

    fn persist_predefined_filters(
        &mut self,
        revision: u64,
        filters: Vec<PredefinedFilter>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(store) = self.persistence.store.clone() else {
            window.push_notification(
                crate::tr!(
                    "状态库尚未就绪，预定义过滤器未保存",
                    "State storage is not ready; predefined filters weren’t saved"
                ),
                cx,
            );
            return;
        };
        self.predefined_filters_saving = true;
        cx.notify();
        self.persistence
            .state_tasks
            .push(cx.spawn_in(window, async move |this, cx| {
                let result = cx
                    .background_spawn(async move {
                        save_predefined_filters_if_current(&store, &filters, revision)
                    })
                    .await;
                _ = this.update_in(cx, |this, window, cx| {
                    this.predefined_filters_saving = false;
                    let pending = this.pending_predefined_filters_save.take();
                    match result {
                        Ok(true) if pending.is_none() => window.push_notification(
                            crate::tr!("预定义过滤器已保存", "Predefined filters saved"),
                            cx,
                        ),
                        Ok(_) => {}
                        Err(error) => window.push_notification(
                            crate::tr_args!(
                                "预定义过滤器未能保存：{error}",
                                "Couldn’t save predefined filters: {error}"
                            ),
                            cx,
                        ),
                    }
                    if let Some((revision, pending)) = pending {
                        this.persist_predefined_filters(revision, pending, window, cx);
                    }
                    cx.notify();
                });
            }));
    }

    fn save_cloud_settings(
        &mut self,
        settings: CloudSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(store) = self.persistence.store.clone() else {
            window.push_notification(
                crate::tr!(
                    "状态库尚未就绪，云端连接设置未保存",
                    "State storage is not ready; cloud connection settings weren’t saved"
                ),
                cx,
            );
            return;
        };
        self.cloud.settings = settings.clone();
        let source_window = window.window_handle();
        let other_workspaces = cx
            .global::<WorkspaceWindowRegistry>()
            .windows
            .iter()
            .filter(|entry| entry.window != source_window)
            .map(|entry| entry.workspace.clone())
            .collect::<Vec<_>>();
        for workspace in other_workspaces {
            let settings = settings.clone();
            workspace.update(cx, |workspace, cx| {
                workspace.cloud.settings = settings;
                cx.notify();
            });
        }
        self.persistence
            .state_tasks
            .push(cx.spawn_in(window, async move |this, cx| {
                let result = cx
                    .background_spawn(async move { store.save_cloud_settings(&settings) })
                    .await;
                if let Err(error) = result {
                    _ = this.update_in(cx, |_, window, cx| {
                        window.push_notification(
                            crate::tr_args!(
                                "云端连接设置未能保存：{error}",
                                "Couldn’t save cloud connection settings: {error}"
                            ),
                            cx,
                        );
                    });
                }
            }));
    }

    fn apply_search_defaults(&mut self, case_sensitive: bool, regex: bool) {
        if self.case_sensitive != case_sensitive || self.regex != regex {
            self.cancel_search();
        }
        self.case_sensitive = case_sensitive;
        self.regex = regex;
        self.app_settings.default_case_sensitive = case_sensitive;
        self.app_settings.default_use_regex = regex;
        self.global_search.query.case_sensitive = case_sensitive;
        self.global_search.query.regex = regex;
        self.global_search.directory_query.case_sensitive = case_sensitive;
        self.global_search.directory_query.regex = regex;
        for tab in &mut self.documents {
            tab.search_query.case_sensitive = case_sensitive;
            tab.search_query.regex = regex;
        }
        self.search_defaults_modified = true;
    }

    fn queue_app_settings_save(
        &mut self,
        settings: AppSettings,
        report_completion: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(store) = self.persistence.store.clone() else {
            return;
        };
        let previous_save = self.persistence.app_settings_save_task.take();
        if report_completion {
            self.settings_saving = true;
        }
        self.persistence.app_settings_save_task =
            Some(cx.spawn_in(window, async move |this, cx| {
                if let Some(previous_save) = previous_save {
                    previous_save.await;
                }
                let result = cx
                    .background_spawn(async move { store.save_app_settings(settings) })
                    .await;
                _ = this.update_in(cx, |this, window, cx| {
                    if report_completion {
                        this.settings_saving = false;
                    }
                    match result {
                        Ok(()) if report_completion => {
                            window.push_notification(crate::tr!("设置已保存", "Settings saved"), cx)
                        }
                        Ok(()) => {}
                        Err(error) if report_completion => window.push_notification(
                            crate::tr_args!(
                                "设置未能保存：{error}",
                                "Couldn’t save settings: {error}"
                            ),
                            cx,
                        ),
                        Err(error) => window.push_notification(
                            crate::tr_args!(
                                "搜索默认值未能保存：{error}",
                                "Couldn’t save search defaults: {error}"
                            ),
                            cx,
                        ),
                    }
                    cx.notify();
                });
            }));
    }

    fn set_search_defaults(
        &mut self,
        case_sensitive: bool,
        regex: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_search_defaults(case_sensitive, regex);
        let source_window = window.window_handle();
        let other_workspaces = cx
            .global::<WorkspaceWindowRegistry>()
            .windows
            .iter()
            .filter(|entry| entry.window != source_window)
            .map(|entry| entry.workspace.clone())
            .collect::<Vec<_>>();
        for workspace in other_workspaces {
            workspace.update(cx, |workspace, cx| {
                workspace.apply_search_defaults(case_sensitive, regex);
                cx.notify();
            });
        }
        self.queue_app_settings_save(self.app_settings.clone(), false, window, cx);
        cx.notify();
    }

    fn refresh_localized_input_copy(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.query.update(cx, |input, cx| {
            input.set_placeholder(crate::tr!("搜索", "Search"), window, cx);
        });
        self.quick_find.query.update(cx, |input, cx| {
            input.set_placeholder(
                crate::tr!("在当前视图中查找", "Find in current view"),
                window,
                cx,
            );
        });
    }

    fn preview_app_settings(
        &mut self,
        settings: AppSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_app_settings_inner(settings, false, window, cx);
    }

    fn apply_app_settings(
        &mut self,
        settings: AppSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_app_settings_inner(settings, true, window, cx);
    }

    fn apply_app_settings_inner(
        &mut self,
        settings: AppSettings,
        commit_defaults: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.app_settings.search_result_limit() != settings.search_result_limit() {
            self.cancel_search();
        }
        crate::actions::apply_shortcuts(&self.app_settings.shortcuts, &settings.shortcuts, cx);
        cx.set_reduce_motion(settings.reduce_motion);
        crate::i18n::set_language(settings.language);
        self.refresh_localized_input_copy(window, cx);
        crate::app_log::set_level(settings.app_log_level);
        Self::apply_theme_preference(settings.theme_preference, window, cx);
        self.app_settings = settings.clone();
        if commit_defaults {
            self.apply_search_defaults(settings.default_case_sensitive, settings.default_use_regex);
            let document_id = self.active_ix.map(|active_ix| {
                let tab = &mut self.documents[active_ix];
                tab.show_line_numbers = settings.default_show_line_numbers;
                tab.show_row_separators = settings.default_show_row_separators;
                tab.uses_default_view_options = true;
                tab.refresh_view_options(cx);
                tab.id
            });
            if let Some(document_id) = document_id {
                self.schedule_checkpoint(document_id, window, cx);
            }
        }
        let search_result_limit = settings.search_result_limit();
        for tab in &mut self.documents {
            tab.search_query.max_results = search_result_limit;
            tab.refresh_appearance(&settings, cx);
            tab.refresh_word_boundary_characters(&settings.word_boundary_characters, cx);
            tab.refresh_log_level_highlighting(settings.highlight_log_levels, cx);
            tab.refresh_search_matcher(settings.highlight_matches, cx);
        }
        self.global_search.query.max_results = search_result_limit;
        self.global_search.directory_query.max_results = search_result_limit;
        let global_matcher = self.global_result_matcher();
        self.global_table.update(cx, |table, cx| {
            table.delegate_mut().set_appearance(&settings);
            table
                .delegate_mut()
                .set_word_boundary_characters(settings.word_boundary_characters.clone());
            table
                .delegate_mut()
                .set_highlight_log_levels(settings.highlight_log_levels);
            table.delegate_mut().set_search_matcher(global_matcher);
            table.refresh(cx);
            cx.notify();
        });
        let source_window = window.window_handle();
        let other_workspaces = cx
            .global::<WorkspaceWindowRegistry>()
            .windows
            .iter()
            .filter(|entry| entry.window != source_window)
            .map(|entry| entry.workspace.clone())
            .collect::<Vec<_>>();
        for workspace in other_workspaces {
            let shared_settings = settings.clone();
            workspace.update(cx, |workspace, cx| {
                if workspace.app_settings.search_result_limit()
                    != shared_settings.search_result_limit()
                {
                    workspace.cancel_search();
                }
                workspace.app_settings = shared_settings.clone();
                workspace.refresh_localized_input_copy(window, cx);
                if commit_defaults {
                    workspace.apply_search_defaults(
                        shared_settings.default_case_sensitive,
                        shared_settings.default_use_regex,
                    );
                }
                let search_result_limit = shared_settings.search_result_limit();
                for tab in &mut workspace.documents {
                    tab.search_query.max_results = search_result_limit;
                    tab.refresh_appearance(&shared_settings, cx);
                    tab.refresh_word_boundary_characters(
                        &shared_settings.word_boundary_characters,
                        cx,
                    );
                    tab.refresh_log_level_highlighting(shared_settings.highlight_log_levels, cx);
                    tab.refresh_search_matcher(shared_settings.highlight_matches, cx);
                }
                workspace.global_search.query.max_results = search_result_limit;
                workspace.global_search.directory_query.max_results = search_result_limit;
                let global_matcher = workspace.global_result_matcher();
                workspace.global_table.update(cx, |table, cx| {
                    table.delegate_mut().set_appearance(&shared_settings);
                    table.delegate_mut().set_word_boundary_characters(
                        shared_settings.word_boundary_characters.clone(),
                    );
                    table
                        .delegate_mut()
                        .set_highlight_log_levels(shared_settings.highlight_log_levels);
                    table.delegate_mut().set_search_matcher(global_matcher);
                    table.refresh(cx);
                    cx.notify();
                });
                cx.notify();
            });
        }
        cx.notify();
    }

    /// 菜单里的开关只改一个字段就落盘。这里刻意不走 `apply_app_settings`，因为那条路会把
    /// 视图默认值重新下发给当前标签，顺带覆盖用户在该标签上单独调过的行号与分隔线。
    fn update_app_setting(
        &mut self,
        update: impl FnOnce(&mut AppSettings),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.persistence.store.is_none() {
            window.push_notification(
                crate::tr!(
                    "状态库尚未就绪，设置未保存",
                    "State storage is not ready; settings weren’t saved"
                ),
                cx,
            );
            return;
        }
        let mut settings = self.app_settings.clone();
        update(&mut settings);
        self.apply_app_settings_inner(settings.clone(), false, window, cx);
        self.queue_app_settings_save(settings, false, window, cx);
    }

    fn save_app_settings(
        &mut self,
        settings: AppSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.persistence.store.is_none() {
            window.push_notification(
                crate::tr!(
                    "状态库尚未就绪，设置未保存",
                    "State storage is not ready; settings weren’t saved"
                ),
                cx,
            );
            return;
        }
        self.apply_app_settings(settings.clone(), window, cx);
        self.queue_app_settings_save(settings, true, window, cx);
    }

    fn open_settings_action(
        &mut self,
        _: &OpenSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_settings_dialog(None, window, cx);
    }

    fn adjust_log_font_size_from_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !event.modifiers.secondary() {
            return;
        }
        let delta_y = event.delta.pixel_delta(window.line_height()).y;
        if delta_y == px(0.) {
            return;
        }
        cx.stop_propagation();
        if self.persistence.store.is_none() {
            return;
        }

        let current = self.app_settings.log_font_size;
        let next = if delta_y > px(0.) {
            current.saturating_add(1).min(32)
        } else {
            current.saturating_sub(1).max(8)
        };
        if next == current {
            return;
        }

        self.app_settings.log_font_size = next;
        for tab in &self.documents {
            tab.refresh_appearance(&self.app_settings, cx);
        }
        self.global_table.update(cx, |table, cx| {
            table.delegate_mut().set_appearance(&self.app_settings);
            table.refresh(cx);
        });

        let source_window = window.window_handle();
        let shared_settings = self.app_settings.clone();
        let other_workspaces = cx
            .global::<WorkspaceWindowRegistry>()
            .windows
            .iter()
            .filter(|entry| entry.window != source_window)
            .map(|entry| entry.workspace.clone())
            .collect::<Vec<_>>();
        for workspace in other_workspaces {
            let shared_settings = shared_settings.clone();
            workspace.update(cx, |workspace, cx| {
                workspace.app_settings = shared_settings.clone();
                for tab in &workspace.documents {
                    tab.refresh_appearance(&shared_settings, cx);
                }
                workspace.global_table.update(cx, |table, cx| {
                    table.delegate_mut().set_appearance(&shared_settings);
                    table.refresh(cx);
                });
                cx.notify();
            });
        }
        self.schedule_appearance_save(window, cx);
        cx.notify();
    }

    fn capture_log_wheel(
        workspace: Entity<Self>,
        document_id: u64,
        region: WrappedRegion,
    ) -> impl IntoElement {
        canvas(
            |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
            move |_, hitbox, window, _| {
                // The table's scroll mask consumes wheel input during capture, so register this
                // listener first while painting the owning log region.
                window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
                    if !phase.capture() || !hitbox.should_handle_scroll(window) {
                        return;
                    }
                    workspace.update(cx, |workspace, cx| {
                        if event.modifiers.secondary() {
                            workspace.adjust_log_font_size_from_wheel(event, window, cx);
                        } else {
                            workspace.handle_log_region_scroll_wheel(
                                document_id,
                                region,
                                event,
                                window,
                                cx,
                            );
                        }
                    });
                });
            },
        )
        .absolute()
        .size_full()
    }

    fn handle_log_region_scroll_wheel(
        &mut self,
        document_id: u64,
        region: WrappedRegion,
        event: &ScrollWheelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.modifiers.control || event.modifiers.platform || event.modifiers.shift {
            return;
        }

        let axis_delta = event.delta.pixel_delta(window.line_height());
        if axis_delta.y == px(0.) || axis_delta.x.abs() > axis_delta.y.abs() {
            return;
        }
        let Some(document_ix) = self.documents.iter().position(|tab| tab.id == document_id) else {
            return;
        };

        let word_wrap = match region {
            WrappedRegion::Log => self.documents[document_ix].log_viewport.is_wrapped(),
            WrappedRegion::Results => self.documents[document_ix].result_viewport.is_wrapped(),
            WrappedRegion::GlobalResults => self.global_viewport.is_wrapped(),
        };
        let line_scroll = self.app_settings.scroll_by_line
            && (!word_wrap || self.app_settings.scroll_by_line_when_word_wrap);
        let scroll_scale = self.app_settings.mouse_wheel_scroll_percent as f32 / 100.;
        let custom_pixel_scale = (scroll_scale - 1.).abs() >= f32::EPSILON;
        let delta_y = if word_wrap && (line_scroll || custom_pixel_scale) {
            event.delta.pixel_delta(px(20.)).y
        } else {
            axis_delta.y
        };
        if delta_y == px(0.) {
            return;
        }

        let auto_follow_changed = region == WrappedRegion::Log
            && std::mem::replace(&mut self.documents[document_ix].auto_follow, false);
        let row_height = self.log_row_height();
        let line_count = usize::from(self.app_settings.mouse_wheel_scroll_lines.max(1));
        let row_count = match region {
            WrappedRegion::Log => self.documents[document_ix].document.line_count(),
            WrappedRegion::Results => self.documents[document_ix].result_row_count(cx),
            WrappedRegion::GlobalResults => self.global_table.read(cx).delegate().rows_len(),
        };
        // A wheel event is newer than any scrollbar offset that was recorded but has not yet
        // reached Workspace rendering. Do not let that older drag sample overwrite the wheel
        // target on the next frame.
        match region {
            WrappedRegion::Log => {
                self.documents[document_ix]
                    .log_viewport
                    .take_pending_scrollbar_offset();
            }
            WrappedRegion::Results => {
                self.documents[document_ix]
                    .result_viewport
                    .take_pending_scrollbar_offset();
            }
            WrappedRegion::GlobalResults => {
                self.global_viewport.take_pending_scrollbar_offset();
            }
        }
        let key = if region == WrappedRegion::GlobalResults {
            (0, region)
        } else {
            (document_id, region)
        };
        let latest_target = self.pending_log_scroll_frames.latest(key);
        let wheel_request = LogWheelScrollRequest {
            delta_y,
            row_count,
            row_height,
            line_count,
            line_scroll,
            scale: scroll_scale,
        };
        let target_offset = match region {
            WrappedRegion::Log => {
                let viewport = &self.documents[document_ix].log_viewport;
                let current = latest_target.map_or_else(
                    || viewport.committed_scroll_offset(),
                    |target| viewport.viewport_offset_for_target(target, row_count, row_height),
                );
                viewport.wheel_scroll_target(current, wheel_request)
            }
            WrappedRegion::Results => {
                let viewport = &self.documents[document_ix].result_viewport;
                let current = latest_target.map_or_else(
                    || viewport.committed_scroll_offset(),
                    |target| viewport.viewport_offset_for_target(target, row_count, row_height),
                );
                viewport.wheel_scroll_target(current, wheel_request)
            }
            WrappedRegion::GlobalResults => {
                let viewport = &self.global_viewport;
                let current = latest_target.map_or_else(
                    || viewport.committed_scroll_offset(),
                    |target| viewport.viewport_offset_for_target(target, row_count, row_height),
                );
                viewport.wheel_scroll_target(current, wheel_request)
            }
        };

        cx.stop_propagation();
        if let Some(offset) = target_offset {
            self.pending_log_scroll_frames
                .request(key, LogScrollFrameTarget::Viewport(offset));
            let surface = match region {
                WrappedRegion::Log => self.documents[document_ix].log_surface.clone(),
                WrappedRegion::Results => self.documents[document_ix].result_surface.clone(),
                WrappedRegion::GlobalResults => self.global_surface.clone(),
            };
            Self::refresh_log_surfaces_atomically([surface], window, cx);
        }

        if auto_follow_changed || target_offset.is_some() {
            cx.notify();
        }
    }

    fn schedule_appearance_save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(store) = self.persistence.store.clone() else {
            return;
        };
        let settings = self.app_settings.clone();
        self.persistence.appearance_save_task = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(300))
                .await;
            let result = cx
                .background_spawn(async move { store.save_app_settings(settings) })
                .await;
            if let Err(error) = result {
                _ = this.update_in(cx, |_, window, cx| {
                    window.push_notification(
                        crate::tr_args!(
                            "日志字号未能保存：{error}",
                            "Couldn’t save the log font size: {error}"
                        ),
                        cx,
                    );
                });
            }
        }));
    }

    fn confirm_clear_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.history_loading || self.history_clearing || self.recent_files.is_empty() {
            return;
        }
        let workspace = cx.entity();
        window.open_alert_dialog(cx, move |alert, _, cx| {
            let workspace = workspace.clone();
            alert
                .icon(Icon::new(IconName::Info).text_color(cx.theme().danger))
                .title(crate::tr!("清除历史？", "Clear history?"))
                .description(crate::tr!("未打开、未收藏且没有行标记的文件会话将被删除。日志文件本身不会改变。", "File sessions that are not open, favorited, or marked will be deleted. Log files will not be changed."))
                .button_props(
                    DialogButtonProps::default()
                        .ok_variant(ButtonVariant::Danger)
                        .ok_text(crate::tr!("清除历史", "Clear history"))
                        .cancel_text(crate::tr!("取消", "Cancel"))
                        .show_cancel(true),
                )
                .on_ok(move |_, window, cx| {
                    workspace.update(cx, |this, cx| this.clear_history(window, cx));
                    true
                })
        });
    }

    fn clear_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(store) = self.persistence.store.clone() else {
            return;
        };
        if self.history_clearing {
            return;
        }
        let open_paths = self
            .documents
            .iter()
            .map(|tab| tab.document.path().to_path_buf())
            .collect::<Vec<_>>();
        self.history_clearing = true;
        cx.notify();
        self.persistence
            .state_tasks
            .push(cx.spawn_in(window, async move |this, cx| {
                let result = cx
                    .background_spawn(async move {
                        let removed = store.clear_history(&open_paths)?;
                        Ok::<_, anyhow::Error>((
                            removed,
                            store.recent_files(8)?,
                            store.pinned_files()?,
                            store.last_workspace()?,
                        ))
                    })
                    .await;
                _ = this.update_in(cx, |this, window, cx| {
                    this.history_clearing = false;
                    match result {
                        Ok((removed, recent_files, pinned_files, last_workspace_files)) => {
                            this.recent_files = recent_files;
                            this.pinned_files = pinned_files;
                            this.last_workspace_files = last_workspace_files;
                            window.push_notification(
                                if removed == 0 {
                                    crate::tr!(
                                        "没有可清除的历史记录",
                                        "There is no history to clear"
                                    )
                                    .to_string()
                                } else {
                                    crate::tr_args!(
                                        "已清除 {removed} 条历史记录",
                                        "Cleared {removed} history entries"
                                    )
                                },
                                cx,
                            );
                        }
                        Err(error) => window.push_notification(
                            crate::tr_args!(
                                "历史记录未能清除：{error}",
                                "Couldn’t clear history: {error}"
                            ),
                            cx,
                        ),
                    }
                    cx.notify();
                });
            }));
    }

    fn save_file_session(
        &mut self,
        path: PathBuf,
        base: FileSessionState,
        state: FileSessionState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if path_match_set_contains(&self.transient_paths, &path) {
            return;
        }
        path_buf_map_insert(
            &mut self.persistence.pending_session_overrides,
            path.clone(),
            state.clone(),
        );
        let Some(store) = self.persistence.store.clone() else {
            self.persistence.pending_sessions.push((path, base, state));
            return;
        };
        let saved_path = path.clone();
        let desired_state = state.clone();
        let previous_save = self.persistence.session_save_task.take();
        self.persistence.session_save_task = Some(cx.spawn_in(window, async move |this, cx| {
            if let Some(previous_save) = previous_save {
                previous_save.await;
            }
            let effective_base = this
                .update_in(cx, |this, _, _| {
                    path_buf_map_get(&this.persistence.last_saved_sessions, &saved_path)
                        .filter(|saved| saved.revision > base.revision)
                        .cloned()
                })
                .ok()
                .flatten()
                .unwrap_or(base);
            let result = cx
                .background_spawn(async move { store.save_session(&path, &effective_base, &state) })
                .await;
            _ = this.update_in(cx, |this, window, cx| match result {
                Ok(result) => {
                    if path_buf_map_get(&this.persistence.pending_session_overrides, &saved_path)
                        .is_some_and(|latest| Self::session_contents_equal(latest, &desired_state))
                    {
                        path_buf_map_remove(
                            &mut this.persistence.pending_session_overrides,
                            &saved_path,
                        );
                    }
                    path_buf_map_insert(
                        &mut this.persistence.last_saved_sessions,
                        saved_path.clone(),
                        result.state.clone(),
                    );
                    if let Some(tab) = this
                        .documents
                        .iter_mut()
                        .find(|tab| paths_match(tab.document.path(), &saved_path))
                        && result.state.revision > tab.session_base.revision
                    {
                        tab.session_base = result.state;
                    }
                    cx.notify();
                }
                Err(error) => {
                    window.push_notification(
                        crate::tr_args!(
                            "文件会话未能保存：{error}",
                            "Couldn’t save the file session: {error}"
                        ),
                        cx,
                    );
                }
            });
        }));
    }

    fn session_contents_equal(left: &FileSessionState, right: &FileSessionState) -> bool {
        left.custom_title == right.custom_title
            && left.selected_row == right.selected_row
            && left.query_text == right.query_text
            && left.case_sensitive == right.case_sensitive
            && left.regex == right.regex
            && left.result_mode == right.result_mode
            && left.marked_rows == right.marked_rows
            && left.show_line_numbers == right.show_line_numbers
            && left.show_row_separators == right.show_row_separators
            && left.word_wrap == right.word_wrap
            && left.keyword_color_rules == right.keyword_color_rules
            && left.resume == right.resume
    }

    fn file_session_state(&self, tab: &DocumentTab, cx: &App) -> FileSessionState {
        let mut marked_rows = tab.marked_rows.clone();
        marked_rows.insert_rows(&tab.pending_restore_marked_rows);
        let selected_row = tab
            .log_table
            .read(cx)
            .active_log_row()
            .and_then(|row_ix| tab.log_table.read(cx).delegate().source_row(row_ix))
            .or(tab.pending_restore_row);
        let (selected_result_ix, selected_result_source_row) = {
            let result_table = tab.result_table.read(cx);
            let selected_result_ix = result_table.active_log_row();
            let selected_result_source_row = selected_result_ix
                .and_then(|row_ix| result_table.delegate().source_row(row_ix))
                .or_else(|| {
                    tab.pending_resume
                        .as_ref()
                        .and_then(|resume| resume.current_search.selected_source_row)
                });
            (selected_result_ix, selected_result_source_row)
        };

        let row_height = self.log_row_height();
        let mut resume = tab.pending_resume.clone().unwrap_or_default();
        resume.viewer.viewport =
            Self::capture_persisted_local_viewport(tab, WrappedRegion::Log, row_height, cx)
                .or(resume.viewer.viewport);
        resume.viewer.auto_follow = tab.auto_follow;
        resume.current_search.results_visible = tab.results_visible;
        resume.current_search.selected_source_row = selected_result_source_row;
        resume.current_search.selected_result_ix = selected_result_ix;
        resume.current_search.viewport =
            Self::capture_persisted_local_viewport(tab, WrappedRegion::Results, row_height, cx)
                .or(resume.current_search.viewport);
        resume.active_region = match tab.selection_table {
            SelectionTable::Log => PersistedLogRegion::Body,
            SelectionTable::Results => PersistedLogRegion::CurrentResults,
        };
        FileSessionState {
            revision: tab.session_base.revision,
            custom_title: tab.custom_title.clone(),
            selected_row,
            query_text: tab.search_query.text.clone(),
            case_sensitive: tab.search_query.case_sensitive,
            regex: tab.search_query.regex,
            result_mode: tab.result_mode.database_value(),
            marked_rows,
            show_line_numbers: tab.show_line_numbers,
            show_row_separators: tab.show_row_separators,
            word_wrap: tab.log_viewport.is_wrapped(),
            keyword_color_rules: tab.keyword_color_rules.clone(),
            resume,
        }
    }

    fn take_quit_snapshot(&mut self, cx: &mut Context<Self>) -> QuitWorkspaceSnapshot {
        self.persistence.checkpoint_tasks.clear();
        self.capture_retained_global_context(self.global_search.scope, cx);
        let search_state = self.primary_window.then(|| self.workspace_search_state());
        let store = self.persistence.store.clone();
        let predefined_filters = cx
            .global::<WorkspaceWindowRegistry>()
            .predefined_filters
            .clone();
        let mut sessions = BTreeMap::new();
        for (path, _, state) in std::mem::take(&mut self.persistence.pending_sessions)
            .into_iter()
            .filter(|(path, _, _)| !path_match_set_contains(&self.transient_paths, path))
        {
            path_buf_map_insert(&mut sessions, path, state);
        }
        for (path, state) in self
            .persistence
            .pending_session_overrides
            .iter()
            .filter(|(path, _)| !path_match_set_contains(&self.transient_paths, path))
        {
            path_buf_map_insert(&mut sessions, path.clone(), state.clone());
        }
        for tab in self
            .documents
            .iter()
            .filter(|tab| !path_match_set_contains(&self.transient_paths, tab.document.path()))
        {
            path_buf_map_insert(
                &mut sessions,
                tab.document.path().to_path_buf(),
                self.file_session_state(tab, cx),
            );
        }
        let open_paths = self
            .documents
            .iter()
            .filter(|tab| !path_match_set_contains(&self.transient_paths, tab.document.path()))
            .map(|tab| tab.document.path().to_path_buf())
            .collect::<Vec<_>>();
        let active_path = self
            .active_document()
            .filter(|tab| !path_match_set_contains(&self.transient_paths, tab.document.path()))
            .map(|tab| tab.document.path().to_path_buf());

        let mut state_tasks = std::mem::take(&mut self.persistence.state_tasks);
        if let Some(task) = self.persistence.session_save_task.take() {
            state_tasks.push(task);
        }
        if let Some(task) = self.persistence.search_history_save_task.take() {
            state_tasks.push(task);
        }
        if let Some(task) = self.persistence.app_settings_save_task.take() {
            state_tasks.push(task);
        }
        if let Some(task) = self.persistence.appearance_save_task.take() {
            state_tasks.push(task);
        }
        if let Some(task) = self.persistence.settings_category_save_task.take() {
            state_tasks.push(task);
        }
        if let Some(task) = self.persistence.search_panel_height_save_task.take() {
            state_tasks.push(task);
        }
        if let Some(task) = self.persistence.search_context_save_task.take() {
            state_tasks.push(task);
        }
        QuitWorkspaceSnapshot {
            store,
            predefined_filters,
            predefined_filters_revision: PREDEFINED_FILTERS_SAVE_REVISION.load(Ordering::Acquire),
            sessions: sessions.into_iter().collect(),
            open_paths,
            active_path,
            search_state,
            state_tasks,
            workspace_order_task: self.persistence.workspace_order_task.take(),
        }
    }

    fn schedule_checkpoint(
        &mut self,
        document_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.documents.iter().any(|tab| tab.id == document_id) {
            return;
        }
        let generation = self.persistence.checkpoint_tasks.reserve(document_id);
        let task = cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(1_500))
                .await;
            _ = this.update_in(cx, |this, window, cx| {
                if this
                    .persistence
                    .checkpoint_tasks
                    .take_if_current(document_id, generation)
                    .is_none()
                {
                    return;
                }
                let Some(tab) = this.documents.iter().find(|tab| tab.id == document_id) else {
                    return;
                };
                let path = tab.document.path().to_path_buf();
                let base = tab.session_base.clone();
                let state = this.file_session_state(tab, cx);
                this.save_file_session(path, base, state, window, cx);
            });
        });
        self.persistence
            .checkpoint_tasks
            .install(document_id, generation, task);
    }

    fn schedule_log_region_state_save(
        &mut self,
        document_id: u64,
        region: WrappedRegion,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match region {
            WrappedRegion::Log | WrappedRegion::Results => {
                self.schedule_checkpoint(document_id, window, cx);
            }
            WrappedRegion::GlobalResults => {
                self.schedule_workspace_search_state_save(window, cx);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn install_documents(
        &mut self,
        opened: Vec<(PathBuf, Result<PreparedDocument>)>,
        active_path: Option<&std::path::Path>,
        target_indices: &BTreeMap<PathBuf, usize>,
        mut replacement_new_tab_id: Option<u64>,
        final_phase: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let adds_document = opened.iter().any(|(path, result)| {
            result.is_ok()
                && !self
                    .documents
                    .iter()
                    .any(|tab| paths_match(tab.document.path(), path))
        });
        if adds_document && self.searches.is_affected_by_added_documents() {
            self.cancel_search();
        }
        if adds_document {
            self.global_search.revision = self.global_search.revision.saturating_add(1);
        }

        let previous_active_id = self.active_document().map(|tab| tab.id);
        let mut errors = Vec::new();
        let mut warnings = Vec::new();
        let mut recorded_paths = Vec::new();
        let mut cache_writes = Vec::new();
        let mut installed_document_ids = BTreeSet::new();
        let mut global_sources_changed = false;

        for (path, result) in opened {
            let mut prepared = match result {
                Ok(prepared) => prepared,
                Err(error) => {
                    errors.push(error.to_string());
                    continue;
                }
            };
            if let Some(warning) = prepared.warning.take() {
                warnings.push(warning);
            }
            let pending_index_cache = prepared.pending_index_cache.take();
            cache_writes.extend(pending_index_cache);

            if let Some(existing_ix) = self
                .documents
                .iter()
                .position(|tab| paths_match(tab.document.path(), &path))
            {
                if let Some(new_tab_id) = replacement_new_tab_id.take() {
                    self.tabs
                        .retain(|tab_id| *tab_id != WorkspaceTabId::New(new_tab_id));
                }
                let current_state = self.documents[existing_ix].load_state;
                // 会话恢复必须等正文、搜索结果、选中行和两个视口都可用后再一次安装，
                // 否则预览首屏会先被绘制，下一帧才跳到持久化位置。
                let should_upgrade = should_upgrade_loading_document(
                    current_state,
                    prepared.load_state,
                    prepared.session.is_some(),
                );
                if should_upgrade {
                    let ready = prepared.load_state == DocumentLoadState::Ready;
                    self.upgrade_loading_document(existing_ix, prepared, window, cx);
                    global_sources_changed = true;
                    if ready && !path_match_set_contains(&self.transient_paths, &path) {
                        recorded_paths.push(path.clone());
                    }
                }
                if self.active_ix != Some(existing_ix) {
                    self.activate_tab(existing_ix, window, cx);
                }
                continue;
            }

            if prepared.load_state == DocumentLoadState::Ready
                && !path_match_set_contains(&self.transient_paths, &path)
            {
                recorded_paths.push(path.clone());
            }
            let document = prepared.document;

            let uses_default_view_options = prepared.session.is_none();
            let session = prepared.session.unwrap_or_else(|| FileSessionState {
                show_line_numbers: self.app_settings.default_show_line_numbers,
                show_row_separators: self.app_settings.default_show_row_separators,
                ..FileSessionState::default()
            });
            let pending_resume = Some(session.resume.clone());
            let session_base = session.clone();
            let custom_title = session
                .custom_title
                .as_deref()
                .map(str::trim)
                .filter(|title| !title.is_empty())
                .map(str::to_owned);
            let search_query = SearchQuery {
                text: session.query_text.clone(),
                case_sensitive: self.app_settings.default_case_sensitive,
                regex: self.app_settings.default_use_regex,
                max_results: self.app_settings.search_result_limit(),
            };
            let result_mode = ResultMode::from_database(session.result_mode);
            let restored_marked_rows = session.marked_rows.clone();
            let marked_rows = restored_marked_rows
                .iter()
                .filter(|row| document.contains_source_row(*row))
                .collect::<CompressedRows>();
            let result_rows =
                compute_result_rows(result_mode, Some(&prepared.search_result), &marked_rows);
            let marked_rows_snapshot = marked_rows.clone();
            let keyword_color_rules = session.keyword_color_rules.clone();
            let resolved_color_rules = installable_color_rules(
                prepared.color_labels_snapshot.as_deref(),
                prepared.resolved_color_rules,
                &keyword_color_rules,
                &self.color_labels,
            );
            let document_id = self.next_document_id;
            self.next_document_id += 1;
            if self.global_search.preference_for(&path).unwrap_or(true) {
                self.global_search.selected_documents.insert(document_id);
            }
            let log_table = cx.new(|cx| {
                let mut delegate = LogTableDelegate::all(document_id, document.clone());
                delegate.set_marked_rows(marked_rows_snapshot.clone());
                delegate.set_view_options(session.show_line_numbers, session.show_row_separators);
                delegate.set_appearance(&self.app_settings);
                delegate.set_word_boundary_characters(
                    self.app_settings.word_boundary_characters.clone(),
                );
                delegate.set_highlight_log_levels(self.app_settings.highlight_log_levels);
                delegate.set_matched_rows(prepared.search_result.line_indices.clone());
                delegate.set_search_matcher(
                    self.app_settings
                        .highlight_matches
                        .then(|| prepared.search_matcher.clone())
                        .flatten(),
                );
                delegate.set_color_rules(resolved_color_rules.clone());
                TableState::new(delegate, window, cx)
                    .loop_selection(false)
                    .row_selectable(false)
                    .sortable(false)
                    .col_movable(false)
                    .col_selectable(false)
            });
            let result_table = cx.new(|cx| {
                let mut delegate =
                    LogTableDelegate::projected(document_id, document.clone(), result_rows);
                delegate.set_marked_rows(marked_rows_snapshot);
                delegate.set_view_options(session.show_line_numbers, session.show_row_separators);
                delegate.set_appearance(&self.app_settings);
                delegate.set_word_boundary_characters(
                    self.app_settings.word_boundary_characters.clone(),
                );
                delegate.set_highlight_log_levels(self.app_settings.highlight_log_levels);
                delegate.set_matched_rows(prepared.search_result.line_indices.clone());
                delegate.set_search_matcher(
                    self.app_settings
                        .highlight_matches
                        .then(|| prepared.search_matcher.clone())
                        .flatten(),
                );
                delegate.set_color_rules(resolved_color_rules.clone());
                TableState::new(delegate, window, cx)
                    .loop_selection(false)
                    .row_selectable(false)
                    .sortable(false)
                    .col_movable(false)
                    .col_selectable(false)
            });
            let result_mode_select = cx.new(|cx| {
                SelectState::new(
                    ResultMode::ALL.to_vec(),
                    Some(IndexPath::new(result_mode.select_index())),
                    window,
                    cx,
                )
            });

            self.subscriptions.push(cx.subscribe_in(
                &log_table,
                window,
                move |this, table, event: &TableEvent, window, cx| {
                    let keep_quick_find_focus = this.quick_find_input_has_focus(window, cx);
                    let source_row = match event {
                        TableEvent::SelectRow(row_ix) => {
                            table.read(cx).delegate().settle_table_selection(*row_ix)
                        }
                        TableEvent::ClearSelection => {
                            if table.read(cx).delegate().take_suppressed_table_clear() {
                                return;
                            }
                            table.read(cx).delegate().clear_row_selection();
                            table.read(cx).delegate().set_active_log_row(None);
                            None
                        }
                        _ => return,
                    };
                    this.selected_source_row = source_row;
                    this.active_log_region = LogRegion::Body;
                    if let Some(tab) = this.documents.iter_mut().find(|tab| tab.id == document_id) {
                        tab.log_jump_revision = tab.log_jump_revision.saturating_add(1);
                        tab.log_jump_task.take();
                        if !keep_quick_find_focus {
                            tab.log_focus_handle.focus(window, cx);
                        }
                        tab.pending_restore_row = None;
                        tab.selection_table = SelectionTable::Log;
                        if source_row.is_some_and(|row| row + 1 < tab.document.source_line_count())
                        {
                            tab.auto_follow = false;
                        }
                    }
                    this.schedule_checkpoint(document_id, window, cx);
                    cx.notify();
                },
            ));
            self.subscriptions.push(cx.subscribe_in(
                &result_table,
                window,
                move |this, table, event: &TableEvent, window, cx| {
                    let keep_quick_find_focus = this.quick_find_input_has_focus(window, cx);
                    let result_ix = match event {
                        TableEvent::SelectRow(result_ix) => *result_ix,
                        TableEvent::ClearSelection => {
                            if table.read(cx).delegate().take_suppressed_table_clear() {
                                return;
                            }
                            table.read(cx).delegate().clear_row_selection();
                            table.read(cx).delegate().set_active_log_row(None);
                            if let Some(tab) =
                                this.documents.iter_mut().find(|tab| tab.id == document_id)
                            {
                                tab.log_jump_revision = tab.log_jump_revision.saturating_add(1);
                                tab.log_jump_task.take();
                            }
                            this.schedule_checkpoint(document_id, window, cx);
                            return;
                        }
                        _ => return,
                    };
                    let Some(tab_ix) = this.documents.iter().position(|tab| tab.id == document_id)
                    else {
                        return;
                    };
                    if this.documents[tab_ix].restoring_result_selection {
                        this.documents[tab_ix].restoring_result_selection = false;
                        return;
                    }
                    let Some(source_row) =
                        table.read(cx).delegate().settle_table_selection(result_ix)
                    else {
                        return;
                    };
                    this.documents[tab_ix].auto_follow = false;
                    if !this.select_and_center_log_source_row_atomically(
                        document_id,
                        source_row,
                        window,
                        cx,
                    ) {
                        return;
                    }
                    if !keep_quick_find_focus {
                        this.documents[tab_ix].result_focus_handle.focus(window, cx);
                    }
                    this.documents[tab_ix].selection_table = SelectionTable::Results;
                    this.active_log_region = LogRegion::CurrentResults;
                    this.selected_source_row = Some(source_row);
                    this.schedule_checkpoint(document_id, window, cx);
                    cx.notify();
                },
            ));
            self.subscriptions.push(cx.subscribe_in(
                &result_mode_select,
                window,
                move |this, _, event: &SelectEvent<Vec<ResultMode>>, window, cx| {
                    let SelectEvent::Confirm(Some(mode)) = event else {
                        return;
                    };
                    let row_height = this.log_row_height();
                    {
                        let Some(tab) = this.documents.iter_mut().find(|tab| tab.id == document_id)
                        else {
                            return;
                        };
                        if tab.result_mode == *mode {
                            return;
                        }
                        tab.result_mode = *mode;
                        tab.refresh_result_rows(row_height, cx);
                        if mode.includes_marks() && !tab.marked_rows.is_empty() {
                            tab.results_visible = true;
                        }
                    }
                    if this
                        .active_document()
                        .is_some_and(|tab| tab.id == document_id)
                    {
                        this.refresh_active_document_surfaces_atomically(window, cx);
                    }
                    this.schedule_checkpoint(document_id, window, cx);
                    cx.notify();
                },
            ));

            let results_visible = restored_results_visible(
                session.resume.current_search.results_visible,
                result_mode,
                !marked_rows.is_empty(),
            );
            let pending_restore_row = session.selected_row;
            if let Some(selected_row) = pending_restore_row.and_then(|row| document.local_row(row))
            {
                log_table.update(cx, |table, cx| table.set_active_log_row(selected_row, cx));
            }
            let title: SharedString = custom_title
                .clone()
                .unwrap_or_else(|| document.file_name())
                .into();
            let log_focus_handle = cx.focus_handle().tab_stop(true);
            let result_focus_handle = cx.focus_handle().tab_stop(true);
            cx.on_focus_in(
                &log_focus_handle,
                window,
                move |this: &mut Workspace, _, cx| {
                    this.active_log_region = LogRegion::Body;
                    if let Some(tab) = this.documents.iter_mut().find(|tab| tab.id == document_id) {
                        tab.selection_table = SelectionTable::Log;
                    }
                    cx.notify();
                },
            )
            .detach();
            cx.on_focus_in(
                &result_focus_handle,
                window,
                move |this: &mut Workspace, _, cx| {
                    this.active_log_region = LogRegion::CurrentResults;
                    if let Some(tab) = this.documents.iter_mut().find(|tab| tab.id == document_id) {
                        tab.selection_table = SelectionTable::Results;
                    }
                    cx.notify();
                },
            )
            .detach();
            let log_surface = {
                let workspace = cx.weak_entity();
                let table = log_table.clone();
                cx.new(move |cx| {
                    LogRegionSurface::new(workspace, document_id, WrappedRegion::Log, &table, cx)
                })
            };
            let result_surface = {
                let workspace = cx.weak_entity();
                let table = result_table.clone();
                cx.new(move |cx| {
                    LogRegionSurface::new(
                        workspace,
                        document_id,
                        WrappedRegion::Results,
                        &table,
                        cx,
                    )
                })
            };
            let log_viewport = {
                let table = log_table.read(cx);
                LogViewportState::new(
                    session.word_wrap,
                    table.vertical_scroll_handle.clone(),
                    table.delegate().row_bounds_handle(),
                )
            };
            let result_viewport = {
                let table = result_table.read(cx);
                LogViewportState::new(
                    session.word_wrap,
                    table.vertical_scroll_handle.clone(),
                    table.delegate().row_bounds_handle(),
                )
            };
            self.documents.push(DocumentTab {
                id: document_id,
                opened_at: Local::now().timestamp(),
                title,
                custom_title,
                document,
                session_base,
                log_table,
                result_table,
                log_surface,
                result_surface,
                log_viewport,
                result_viewport,
                search_query,
                search_result: prepared.search_result,
                search_matcher: prepared.search_matcher,
                result_mode,
                result_mode_select,
                search_revision: 0,
                log_jump_revision: 0,
                log_jump_task: None,
                results_visible,
                restoring_result_selection: false,
                marked_rows,
                pending_restore_marked_rows: if prepared.load_state != DocumentLoadState::Ready {
                    restored_marked_rows
                } else {
                    CompressedRows::default()
                },
                keyword_color_rules,
                resolved_color_rules,
                log_text_selection_scope: TextSelectionScopeId::default(),
                result_text_selection_scope: TextSelectionScopeId::default(),
                log_focus_handle,
                result_focus_handle,
                auto_follow: false,
                show_line_numbers: session.show_line_numbers,
                show_row_separators: session.show_row_separators,
                selection_table: restored_selection_table(
                    session.resume.active_region,
                    results_visible,
                ),
                uses_default_view_options,
                load_state: prepared.load_state,
                pending_restore_row: (prepared.load_state != DocumentLoadState::Ready)
                    .then_some(pending_restore_row)
                    .flatten(),
                pending_resume,
            });
            global_sources_changed = true;
            installed_document_ids.insert(document_id);
            let workspace_tab_id = WorkspaceTabId::Document(document_id);
            if let Some(new_tab_id) = replacement_new_tab_id.take() {
                let replacement_id = WorkspaceTabId::New(new_tab_id);
                if let Some(tab_ix) = self
                    .tabs
                    .iter()
                    .position(|tab_id| *tab_id == replacement_id)
                {
                    self.tabs[tab_ix] = workspace_tab_id;
                } else {
                    self.tabs.push(workspace_tab_id);
                }
            } else if let Some(target_ix) = path_buf_map_get(target_indices, &path).copied() {
                let target_ix = target_ix.min(self.tabs.len());
                self.tabs.insert(target_ix, workspace_tab_id);
            } else {
                self.tabs.push(workspace_tab_id);
            }
            self.active_tab_id = workspace_tab_id;
        }
        self.reorder_documents_to_match_tabs();
        if global_sources_changed {
            self.refresh_global_result_rows(window, cx);
        }
        if let Some(active_path) = active_path
            && let Some(document_id) = self
                .documents
                .iter()
                .find(|tab| paths_match(tab.document.path(), active_path))
                .map(|tab| tab.id)
        {
            self.active_tab_id = WorkspaceTabId::Document(document_id);
            self.sync_active_document_ix();
        }
        if !installed_document_ids.is_empty() {
            let active_document_id = self
                .active_ix
                .and_then(|ix| self.documents.get(ix).map(|tab| tab.id))
                .filter(|document_id| installed_document_ids.contains(document_id));
            self.pending_document_tab_reveal.set(active_document_id);
            let installed_document_ids = installed_document_ids.iter().copied().collect::<Vec<_>>();
            for document_id in installed_document_ids {
                if let Some(document_ix) =
                    self.documents.iter().position(|tab| tab.id == document_id)
                {
                    self.apply_tab_resume(document_ix, cx);
                }
            }
        }
        self.record_recent_paths(recorded_paths, window, cx);
        for cache_write in cache_writes {
            self.persistence
                .state_tasks
                .push(cx.spawn(async move |_, cx| {
                    if let Err(error) = cx
                        .background_spawn(async move { cache_write.persist() })
                        .await
                    {
                        log::error!("索引缓存未能保存：{error:#}");
                    }
                }));
        }

        if !warnings.is_empty() {
            window.push_notification(warnings.join("；"), cx);
        }
        if final_phase && errors.is_empty() {
            self.activity = Activity::Ready;
        } else if final_phase {
            let message: SharedString = errors.join("；").into();
            window.push_notification(message.clone(), cx);
            self.activity = Activity::Error;
        }
        if !final_phase || previous_active_id != self.active_document().map(|tab| tab.id) {
            self.sync_active_document(window, cx);
        } else {
            self.selected_source_row = self.active_document().and_then(|tab| {
                let table = tab.log_table.read(cx);
                table
                    .active_log_row()
                    .and_then(|row_ix| table.delegate().source_row(row_ix))
            });
        }
        let active_document_was_installed = self
            .active_document()
            .is_some_and(|tab| installed_document_ids.contains(&tab.id));
        if active_document_was_installed {
            self.refresh_active_document_surfaces_atomically(window, cx);
        }
        cx.notify();
    }

    fn install_completed_documents(
        &mut self,
        opened: Vec<(PathBuf, Result<PreparedDocument>)>,
        active_path: Option<&std::path::Path>,
        target_indices: &BTreeMap<PathBuf, usize>,
        opening_ids: &BTreeMap<PathBuf, u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut accepted = Vec::new();
        for (path, result) in opened {
            if let Some(expected_id) = path_buf_map_get(opening_ids, &path).copied() {
                let still_open = self.documents.iter().any(|tab| {
                    tab.id == expected_id
                        && paths_match(tab.document.path(), &path)
                        && matches!(
                            tab.load_state,
                            DocumentLoadState::Opening
                                | DocumentLoadState::Preview
                                | DocumentLoadState::IndexFailed
                        )
                });
                if !still_open {
                    continue;
                }
                if result.is_err()
                    && let Some(tab) = self.documents.iter_mut().find(|tab| tab.id == expected_id)
                {
                    tab.load_state = DocumentLoadState::IndexFailed;
                }
            }
            accepted.push((path, result));
        }
        self.install_documents(
            accepted,
            active_path,
            target_indices,
            None,
            true,
            window,
            cx,
        );
        self.complete_pending_directory_result_jump(window, cx);
        self.persist_workspace_order(window, cx);
    }

    fn apply_tab_resume(&mut self, document_ix: usize, cx: &mut Context<Self>) {
        if self
            .documents
            .get(document_ix)
            .is_none_or(|tab| tab.load_state != DocumentLoadState::Ready)
        {
            return;
        }
        let row_height = self.log_row_height();
        let resume = self.documents[document_ix]
            .pending_resume
            .take()
            .unwrap_or_else(|| self.documents[document_ix].session_base.resume.clone());
        {
            let tab = &mut self.documents[document_ix];
            tab.auto_follow = resume.viewer.auto_follow;
            tab.results_visible = restored_results_visible(
                resume.current_search.results_visible,
                tab.result_mode,
                !tab.marked_rows.is_empty(),
            );
            tab.selection_table =
                restored_selection_table(resume.active_region, tab.results_visible);

            let result_count = tab.result_table.read(cx).delegate().row_count();
            let selected_result_ix = resume
                .current_search
                .selected_source_row
                .and_then(|source_row| tab.result_row_ix(source_row, cx))
                .or(resume.current_search.selected_result_ix)
                .filter(|_| result_count > 0)
                .map(|ix| ix.min(result_count.saturating_sub(1)));
            if let Some(row_ix) = selected_result_ix {
                tab.restoring_result_selection = true;
                tab.result_table.update(cx, |table, cx| {
                    restore_current_result_selection(table, row_ix, cx)
                });
            } else {
                tab.result_table.update(cx, |table, cx| {
                    table.delegate().clear_row_selection();
                    table.delegate().set_active_log_row(None);
                    table.clear_selection(cx);
                });
            }
        }

        let tab = &self.documents[document_ix];
        Self::restore_persisted_local_viewport(
            tab,
            WrappedRegion::Log,
            resume.viewer.viewport,
            row_height,
            cx,
        );
        Self::restore_persisted_local_viewport(
            tab,
            WrappedRegion::Results,
            resume.current_search.viewport,
            row_height,
            cx,
        );
        if self.active_ix == Some(document_ix)
            && self.global_search.scope == SearchScope::CurrentFile
        {
            self.active_log_region = restored_log_region(resume.active_region, tab.results_visible);
        }
    }

    fn complete_pending_directory_result_jump(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(pending) = self.pending_directory_result_jump.clone() else {
            return;
        };
        let Some(document_ix) = self.documents.iter().position(|tab| {
            paths_match(tab.document.path(), &pending.path)
                && tab.load_state == DocumentLoadState::Ready
        }) else {
            self.pending_directory_result_jump = None;
            return;
        };
        self.pending_directory_result_jump = None;
        if !pending.matches(&self.documents[document_ix].document) {
            Self::notify_stale_directory_result(window, cx);
            return;
        }
        self.activate_tab(document_ix, window, cx);
        let Some(tab) = self.documents.get_mut(document_ix) else {
            return;
        };
        tab.auto_follow = false;
        tab.selection_table = SelectionTable::Log;
        if !tab.select_and_center_log_source_row(pending.source_row, cx) {
            window.push_notification(
                crate::tr!(
                    "该目录结果行在当前文件中已不存在，请重新搜索",
                    "That directory result line no longer exists in the current file. Search again."
                ),
                cx,
            );
            return;
        }
        self.selected_source_row = Some(pending.source_row);
        cx.notify();
    }

    fn upgrade_loading_document(
        &mut self,
        document_ix: usize,
        prepared: PreparedDocument,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let highlight_matches = self.app_settings.highlight_matches;
        let tab = &mut self.documents[document_ix];
        let previous_state = tab.load_state;
        let selected_source_row = {
            let table = tab.log_table.read(cx);
            table
                .active_log_row()
                .and_then(|row_ix| table.delegate().source_row(row_ix))
        };
        if previous_state == DocumentLoadState::Opening
            && let Some(session) = prepared.session.as_ref()
        {
            tab.custom_title = session
                .custom_title
                .as_deref()
                .map(str::trim)
                .filter(|title| !title.is_empty())
                .map(str::to_owned);
            tab.title = tab
                .custom_title
                .clone()
                .unwrap_or_else(|| prepared.document.file_name())
                .into();
            tab.search_query = SearchQuery {
                text: session.query_text.clone(),
                case_sensitive: session.case_sensitive,
                regex: session.regex,
                max_results: self.app_settings.search_result_limit(),
            };
            tab.result_mode = ResultMode::from_database(session.result_mode);
            tab.result_mode_select.update(cx, |select, cx| {
                select.set_selected_index(
                    Some(IndexPath::new(tab.result_mode.select_index())),
                    window,
                    cx,
                );
            });
            tab.pending_restore_marked_rows = session.marked_rows.clone();
            tab.pending_restore_row = session.selected_row;
            tab.pending_resume = Some(session.resume.clone());
            tab.keyword_color_rules = session.keyword_color_rules.clone();
            tab.resolved_color_rules = installable_color_rules(
                prepared.color_labels_snapshot.as_deref(),
                prepared.resolved_color_rules.clone(),
                &tab.keyword_color_rules,
                &self.color_labels,
            );
            tab.show_line_numbers = session.show_line_numbers;
            tab.show_row_separators = session.show_row_separators;
            tab.log_viewport.set_word_wrap(session.word_wrap);
            tab.result_viewport.set_word_wrap(session.word_wrap);
            tab.uses_default_view_options = false;
            tab.results_visible = restored_results_visible(
                session.resume.current_search.results_visible,
                tab.result_mode,
                !tab.pending_restore_marked_rows.is_empty(),
            );
            tab.selection_table =
                restored_selection_table(session.resume.active_region, tab.results_visible);
            tab.refresh_view_options(cx);
            for table in [tab.log_table.clone(), tab.result_table.clone()] {
                table.update(cx, |table, cx| {
                    table
                        .delegate_mut()
                        .set_color_rules(tab.resolved_color_rules.clone());
                    table.refresh(cx);
                });
            }
        }
        tab.document = prepared.document;
        tab.search_result = prepared.search_result;
        tab.search_matcher = prepared.search_matcher;
        if prepared.load_state == DocumentLoadState::Ready {
            let pending_marks = std::mem::take(&mut tab.pending_restore_marked_rows);
            tab.marked_rows.extend(pending_marks.iter());
            tab.marked_rows
                .retain_below(tab.document.source_line_count());
        } else {
            tab.marked_rows = tab
                .pending_restore_marked_rows
                .iter()
                .filter(|row| tab.document.contains_source_row(*row))
                .collect();
        }
        let result_rows = tab.compute_result_rows();
        tab.log_viewport.invalidate_wrapped();
        tab.result_viewport.invalidate_wrapped();

        let marked_rows = tab.marked_rows.clone();
        tab.log_table.update(cx, |table, cx| {
            table.delegate_mut().replace_with_all(tab.document.clone());
            table.delegate_mut().set_marked_rows(marked_rows.clone());
            table
                .delegate_mut()
                .set_matched_rows(tab.search_result.line_indices.clone());
            table.delegate_mut().set_search_matcher(
                highlight_matches
                    .then(|| tab.search_matcher.clone())
                    .flatten(),
            );
            table.refresh_log_rows(cx);
        });
        tab.result_table.update(cx, |table, cx| {
            table
                .delegate_mut()
                .replace_with_rows(tab.document.clone(), result_rows);
            table.delegate_mut().set_marked_rows(marked_rows);
            table
                .delegate_mut()
                .set_matched_rows(tab.search_result.line_indices.clone());
            table.delegate_mut().set_search_matcher(
                highlight_matches
                    .then(|| tab.search_matcher.clone())
                    .flatten(),
            );
            table.refresh_log_rows(cx);
        });

        let restore_row = if prepared.load_state == DocumentLoadState::Ready {
            tab.pending_restore_row.take().or(selected_source_row)
        } else {
            tab.pending_restore_row.or(selected_source_row)
        };
        if let Some(row) = restore_row.and_then(|row| tab.document.local_row(row)) {
            tab.log_table
                .update(cx, |table, cx| table.set_active_log_row(row, cx));
        }
        tab.load_state = prepared.load_state;
        if prepared.load_state == DocumentLoadState::Ready {
            self.apply_tab_resume(document_ix, cx);
        }
        if self.active_ix == Some(document_ix) {
            self.refresh_active_document_surfaces_atomically(window, cx);
        }
    }

    fn auto_follow_candidate(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<(u64, PathBuf, u64, Option<SystemTime>)> {
        if self.open_task.is_some() {
            return None;
        }
        let tab = self.documents.get_mut(self.active_ix?)?;
        if tab.load_state != DocumentLoadState::Ready || !tab.auto_follow {
            return None;
        }

        let visible_rows = tab.log_table.read(cx).visible_range().rows().clone();
        if visible_rows.end > 0 && visible_rows.end < tab.document.line_count() {
            tab.auto_follow = false;
            cx.notify();
            return None;
        }

        Some((
            tab.id,
            tab.document.path().to_path_buf(),
            tab.document.metadata().file_size,
            tab.document.metadata().modified,
        ))
    }

    fn reload_active(&mut self, _: &ReloadActive, window: &mut Window, cx: &mut Context<Self>) {
        let Some(active_ix) = self.active_ix else {
            return;
        };
        let document_id = self.documents[active_ix].id;
        self.reload_document(document_id, false, ReloadStrategy::Full, window, cx);
    }

    fn reload_document(
        &mut self,
        document_id: u64,
        follow_end: bool,
        strategy: ReloadStrategy,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.open_task.is_some() {
            return;
        }
        let Some(document_ix) = self.documents.iter().position(|tab| tab.id == document_id) else {
            return;
        };
        self.cancel_search_for(document_id);
        let tab = &mut self.documents[document_ix];
        let follow_end = follow_end || tab.auto_follow;
        tab.search_revision += 1;
        let revision = tab.search_revision;
        let previous_document = tab.document.clone();
        let query = tab.search_query.clone();
        let results_visible = tab.results_visible;
        let selected_source_row = {
            let table = tab.log_table.read(cx);
            table
                .active_log_row()
                .and_then(|row_ix| table.delegate().source_row(row_ix))
        };
        self.activity = Activity::Opening;
        cx.notify();

        self.open_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let document = Arc::new(match strategy {
                        ReloadStrategy::Full => LogDocument::open(previous_document.path())?,
                        ReloadStrategy::ExtendAppend => previous_document.refresh()?.0,
                    });
                    let search_matcher = SearchMatcher::new(&query)?;
                    let search_result = if query.text.is_empty() {
                        SearchResult::default()
                    } else {
                        search_document_with_matcher(&document, &query, search_matcher.as_ref())
                    };
                    Ok::<_, anyhow::Error>((document, search_result, query, search_matcher))
                })
                .await;

            _ = this.update_in(cx, |this, window, cx| {
                let highlight_matches = this.app_settings.highlight_matches;
                let search_result_limit = this.app_settings.search_result_limit();
                let Some(tab) = this.documents.iter_mut().find(|tab| tab.id == document_id) else {
                    this.open_task = None;
                    this.open_queued_external_paths_if_idle(window, cx);
                    return;
                };
                if tab.search_revision != revision {
                    this.open_task = None;
                    this.open_queued_external_paths_if_idle(window, cx);
                    return;
                }

                let reloaded = match result {
                    Ok((document, search_result, query, search_matcher)) => {
                        tab.document = document;
                        tab.search_query.text = query.text;
                        tab.search_query.max_results = search_result_limit;
                        tab.search_result = search_result;
                        tab.search_matcher = search_matcher;
                        let pending_marks = std::mem::take(&mut tab.pending_restore_marked_rows);
                        tab.marked_rows.extend(pending_marks.iter());
                        tab.marked_rows.retain_below(tab.document.line_count());
                        let result_rows = tab.compute_result_rows();
                        tab.log_viewport.invalidate_wrapped();
                        tab.result_viewport.invalidate_wrapped();
                        let marked_rows = tab.marked_rows.clone();
                        let selected_result_row = (!follow_end)
                            .then(|| {
                                selected_source_row
                                    .and_then(|source_row| result_rows.position(source_row))
                            })
                            .flatten();
                        tab.log_table.update(cx, |table, cx| {
                            table.delegate_mut().replace_with_all(tab.document.clone());
                            table.delegate_mut().set_marked_rows(marked_rows.clone());
                            table
                                .delegate_mut()
                                .set_matched_rows(tab.search_result.line_indices.clone());
                            table.delegate_mut().set_search_matcher(
                                highlight_matches
                                    .then(|| tab.search_matcher.clone())
                                    .flatten(),
                            );
                            table.refresh_log_rows(cx);
                        });
                        tab.result_table.update(cx, |table, cx| {
                            table
                                .delegate_mut()
                                .replace_with_rows(tab.document.clone(), result_rows);
                            table.delegate_mut().set_marked_rows(marked_rows);
                            table
                                .delegate_mut()
                                .set_matched_rows(tab.search_result.line_indices.clone());
                            table.delegate_mut().set_search_matcher(
                                highlight_matches
                                    .then(|| tab.search_matcher.clone())
                                    .flatten(),
                            );
                            if let Some(row_ix) = selected_result_row {
                                table.set_active_log_row(row_ix, cx);
                            } else {
                                table.clear_selection(cx);
                            }
                            table.refresh_log_rows(cx);
                        });
                        tab.results_visible = results_visible;
                        tab.load_state = DocumentLoadState::Ready;
                        tab.pending_restore_row = None;

                        if tab.document.line_count() > 0 {
                            let row = if follow_end {
                                tab.document.line_count() - 1
                            } else {
                                selected_source_row
                                    .unwrap_or_default()
                                    .min(tab.document.line_count() - 1)
                            };
                            tab.log_table
                                .update(cx, |table, cx| table.set_active_log_row(row, cx));
                        }
                        this.activity = Activity::Ready;
                        true
                    }
                    Err(error) => {
                        if follow_end {
                            tab.auto_follow = false;
                        }
                        let message: SharedString = error.to_string().into();
                        window.push_notification(message.clone(), cx);
                        this.activity = Activity::Error;
                        false
                    }
                };
                if reloaded {
                    if this
                        .active_document()
                        .is_some_and(|tab| tab.id == document_id)
                    {
                        this.refresh_active_document_surfaces_atomically(window, cx);
                    }
                    let invalidated = this.invalidate_all_open_results_for_reload(document_id);
                    this.refresh_global_result_rows(window, cx);
                    if let Some(visible_results_invalidated) = invalidated {
                        if visible_results_invalidated {
                            window.push_notification(
                                crate::tr!(
                                    "文件已更新，请重新执行全部打开文件搜索",
                                    "The file changed. Run the all-open-files search again."
                                ),
                                cx,
                            );
                        }
                        this.schedule_workspace_search_state_save(window, cx);
                    }
                }
                this.open_task = None;
                this.open_queued_external_paths_if_idle(window, cx);
                cx.notify();
            });
        }));
    }

    fn copy_current_line(
        &mut self,
        _: &CopyCurrentLine,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.copy_selected_line(false, window, cx);
    }

    fn copy_current_line_with_number(
        &mut self,
        _: &CopyCurrentLineWithNumber,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.copy_selected_line(true, window, cx);
    }

    fn copy_selected_line(
        &mut self,
        include_line_number: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.line_copy_revision = self.line_copy_revision.saturating_add(1);
        if let Some(cancellation) = self.line_copy_cancellation.take() {
            cancellation.cancel();
        }
        self.line_copy_task = None;
        let revision = self.line_copy_revision;
        if !include_line_number {
            let selected_text = TextSelection::selected_text(window, cx);
            if !selected_text.trim().is_empty() {
                cx.write_to_clipboard(ClipboardItem::new_string(selected_text));
                window.push_notification(crate::tr!("已复制所选文字", "Selected text copied"), cx);
                return;
            }
        }
        if self.active_log_region == LogRegion::GlobalResults && self.global_search.results_visible
        {
            let selected_documents = self
                .global_table
                .read(cx)
                .delegate()
                .selected_match_documents();
            if selected_documents.is_empty() {
                window.push_notification(
                    crate::tr!(
                        "请先选择要复制的全局结果行",
                        "Select global result lines to copy first"
                    ),
                    cx,
                );
                return;
            }
            self.start_line_copy(
                selected_documents,
                include_line_number,
                LineCopyScope::Global,
                revision,
                window,
                cx,
            );
            return;
        }
        let Some(tab) = self.active_document() else {
            return;
        };
        let selected_rows = tab.selected_source_rows_compressed(cx);
        if selected_rows.is_empty() {
            window.push_notification(
                crate::tr!("请先选择要复制的日志行", "Select log lines to copy first"),
                cx,
            );
            return;
        }
        self.start_line_copy(
            vec![(tab.document.clone(), selected_rows)],
            include_line_number,
            LineCopyScope::Local,
            revision,
            window,
            cx,
        );
    }

    fn start_line_copy(
        &mut self,
        documents: Vec<(Arc<LogDocument>, CompressedRows)>,
        include_line_number: bool,
        scope: LineCopyScope,
        revision: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let cancellation = SearchCancellation::default();
        self.line_copy_cancellation = Some(cancellation.clone());
        self.line_copy_task = Some(cx.spawn_in(window, async move |this, cx| {
            let copied = cx
                .background_spawn(async move {
                    collect_log_lines_for_clipboard(
                        documents,
                        include_line_number,
                        &cancellation,
                    )
                })
                .await;
            _ = this.update_in(cx, |this, window, cx| {
                if this.line_copy_revision != revision {
                    return;
                }
                this.line_copy_task = None;
                this.line_copy_cancellation = None;
                let copied = match copied {
                    DocumentLineTask::Completed(copied) => copied,
                    DocumentLineTask::Cancelled => return,
                    DocumentLineTask::SourceUnavailable => {
                        window.push_notification(
                            crate::tr!(
                                "所选日志的文件内容已改变，请重新加载后再复制",
                                "The selected log file changed. Reload it before copying."
                            ),
                            cx,
                        );
                        return;
                    }
                };
                if copied.text.is_empty() {
                    window.push_notification(
                        match scope {
                            LineCopyScope::Local => crate::tr!(
                                "所选日志行已不可用，请重新选择",
                                "The selected log lines are no longer available. Select them again."
                            ),
                            LineCopyScope::Global => crate::tr!(
                                "所选全局结果已不可用，请重新选择",
                                "The selected global results are no longer available. Select them again."
                            ),
                        },
                        cx,
                    );
                    return;
                }
                cx.write_to_clipboard(ClipboardItem::new_string(copied.text));
                let notification = match scope {
                    LineCopyScope::Global => crate::tr_args!(
                        "已复制 {} 条全局结果",
                        "Copied {} global results",
                        copied.count
                    ),
                    LineCopyScope::Local if copied.count == 1 => crate::tr_args!(
                        "已复制第 {} 行",
                        "Copied line {}",
                        copied.first_source_row.unwrap_or_default() + 1
                    ),
                    LineCopyScope::Local => {
                        crate::tr_args!("已复制 {} 行", "Copied {} lines", copied.count)
                    }
                };
                window.push_notification(notification, cx);
            });
        }));
    }

    fn select_all_rows(&mut self, _: &SelectAllRows, window: &mut Window, cx: &mut Context<Self>) {
        TextSelection::clear(window, cx);
        if self.active_log_region == LogRegion::GlobalResults && self.global_search.results_visible
        {
            self.global_table.update(cx, |table, cx| {
                table.delegate().select_all_rows();
                cx.notify();
            });
            self.status_surface.update(cx, |_, cx| cx.notify());
            self.schedule_workspace_search_state_save(window, cx);
            return;
        }
        let Some(tab) = self.active_document() else {
            return;
        };
        let table = match tab.selection_table {
            SelectionTable::Log => tab.log_table.clone(),
            SelectionTable::Results => tab.result_table.clone(),
        };
        table.update(cx, |table, cx| {
            if table.delegate().selected_rows_count() == 0
                && table.delegate().source_row(0).is_some()
            {
                table.set_active_log_row(0, cx);
            }
            table.delegate().select_all_rows();
            cx.notify();
        });
        self.status_surface.update(cx, |_, cx| cx.notify());
        self.schedule_checkpoint(tab.id, window, cx);
    }

    fn copy_file_path(&mut self, _: &CopyFilePath, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.active_document() else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(
            tab.document.path().display().to_string(),
        ));
        window.push_notification(crate::tr!("已复制文件路径", "File path copied"), cx);
    }

    fn copy_document_encoding(
        &mut self,
        document_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.documents.iter().find(|tab| tab.id == document_id) else {
            return;
        };
        let encoding = tab.document.metadata().encoding_name.clone();
        cx.write_to_clipboard(ClipboardItem::new_string(encoding.clone()));
        window.push_notification(
            crate::tr_args!(
                "已复制编码名称：{encoding}",
                "Encoding name copied: {encoding}"
            ),
            cx,
        );
    }

    fn reload_document_encoding(
        &mut self,
        document_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.reload_document(document_id, false, ReloadStrategy::Full, window, cx);
    }

    fn build_encoding_menu(
        menu: PopupMenu,
        document_id: u64,
        encoding_name: SharedString,
        workspace: Entity<Self>,
        window: &mut Window,
    ) -> PopupMenu {
        let reload = {
            let workspace = workspace.clone();
            window.listener_for(&workspace, move |this, _, window, cx| {
                this.reload_document_encoding(document_id, window, cx)
            })
        };
        let copy = window.listener_for(&workspace, move |this, _, window, cx| {
            this.copy_document_encoding(document_id, window, cx)
        });
        menu.item(
            PopupMenuItem::new(crate::tr_args!(
                "当前编码：{encoding_name}",
                "Current encoding: {encoding_name}"
            ))
            .disabled(true),
        )
        .item(
            PopupMenuItem::new(crate::tr!("重新检测并加载", "Detect and reload")).on_click(reload),
        )
        .item(PopupMenuItem::new(crate::tr!("复制编码名称", "Copy encoding name")).on_click(copy))
    }

    fn open_go_to_line(&mut self, _: &GoToLine, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.active_document() else {
            return;
        };
        if tab.load_state != DocumentLoadState::Ready {
            window.push_notification(
                crate::tr!(
                    "完整索引建立后即可按行号定位",
                    "Go to line will be available after the full index is built"
                ),
                cx,
            );
            return;
        }
        let line_count = tab.document.line_count();
        if line_count == 0 {
            window.push_notification(
                crate::tr!(
                    "当前文件没有可定位的日志行",
                    "The current file has no log line to locate"
                ),
                cx,
            );
            return;
        }

        let workspace = cx.entity();
        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(crate::tr_args!(
                "输入 1 到 {line_count} 之间的行号",
                "Enter a line number from 1 to {line_count}"
            ))
        });
        let focus = input.focus_handle(cx);
        window.defer(cx, move |window, cx| focus.focus(window, cx));

        window.open_dialog(cx, move |dialog, _, _| {
            let input_for_confirm = input.clone();
            let workspace_for_confirm = workspace.clone();
            dialog
                .title(crate::tr!("转到行", "Go to line"))
                .child(
                    v_flex()
                        .gap_3()
                        .child(crate::tr!("输入源日志中的行号。确认后会选择该行并滚动到可见位置。", "Enter a source log line number. The line will be selected and scrolled into view."))
                        .child(Input::new(&input)),
                )
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(crate::tr!("转到", "Go"))
                        .cancel_text(crate::tr!("取消", "Cancel"))
                        .show_cancel(true),
                )
                .on_ok(move |_, window, cx| {
                    let value = input_for_confirm.read(cx).value().trim().to_string();
                    let Ok(line_number) = value.parse::<usize>() else {
                        window.push_notification(crate::tr!("请输入有效的正整数行号", "Enter a valid positive line number"), cx);
                        return false;
                    };
                    let outcome = workspace_for_confirm.update(cx, |workspace, cx| {
                        let Some(tab) = workspace.active_document() else {
                            return Err(crate::tr!("当前没有活动日志文件", "There is no active log file").to_string());
                        };
                        let current_line_count = tab.document.line_count();
                        if !(1..=current_line_count).contains(&line_number) {
                            return Err(crate::tr_args!("行号应在 1 到 {current_line_count} 之间", "Line number must be from 1 to {current_line_count}"));
                        }

                        let source_row = line_number - 1;
                        let table = tab.log_table.clone();
                        table.update(cx, |table, cx| {
                            table.set_active_log_row(source_row, cx);
                        });
                        workspace.selected_source_row = Some(source_row);
                        cx.notify();
                        Ok(())
                    });
                    match outcome {
                        Ok(()) => true,
                        Err(message) => {
                            window.push_notification(message, cx);
                            false
                        }
                    }
                })
        });
    }

    fn cycle_color_label(
        &mut self,
        _: &CycleColorLabel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selected_text = TextSelection::selected_text(window, cx);
        let (_, target) = match self.context_color_target(Some(selected_text.as_str()), cx) {
            Ok(target) => target,
            Err(message) => {
                window.push_notification(message, cx);
                return;
            }
        };
        self.start_color_rule_action(target, ColorRuleAction::Cycle, window, cx);
    }

    fn toggle_marked_row(
        &mut self,
        _: &ToggleMarkedRow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active_log_region == LogRegion::GlobalResults && self.global_search.results_visible
        {
            let selected_matches = self.global_table.read(cx).delegate().selection_snapshot();
            if selected_matches.is_empty() {
                window.push_notification(
                    crate::tr!(
                        "请先选择要标记的全局结果行",
                        "Select global result lines to mark first"
                    ),
                    cx,
                );
                return;
            }
            let Some(selected_by_document) = self.resolve_global_mark_targets(&selected_matches)
            else {
                window.push_notification(
                    if self.global_search.scope == SearchScope::Directory {
                        crate::tr!(
                            "请打开所有选中结果对应的文件；若文件内容已改变，请重新搜索",
                            "Open every file for the selected results. If a file changed, search again."
                        )
                    } else {
                        crate::tr!(
                            "所选结果所属文件已不可用，请重新搜索",
                            "A file for the selected results is unavailable. Search again."
                        )
                    },
                    cx,
                );
                return;
            };
            let is_marking = selected_by_document.iter().any(|(document_id, rows)| {
                self.documents
                    .iter()
                    .find(|tab| tab.id == *document_id)
                    .is_some_and(|tab| !tab.marked_rows.contains_all(rows))
            });
            let mut changed_documents = Vec::new();
            let mut changed_rows = 0_usize;
            let row_height = self.log_row_height();
            for (document_id, rows) in selected_by_document {
                let Some(tab_ix) = self.documents.iter().position(|tab| tab.id == document_id)
                else {
                    continue;
                };
                let tab = &mut self.documents[tab_ix];
                if rows.is_empty() {
                    continue;
                }
                if is_marking {
                    tab.marked_rows.insert_rows(&rows);
                    tab.pending_restore_marked_rows.insert_rows(&rows);
                } else {
                    tab.marked_rows.remove_rows(&rows);
                    tab.pending_restore_marked_rows.remove_rows(&rows);
                }
                changed_rows = changed_rows.saturating_add(rows.len());
                let marked_rows = tab.marked_rows.clone();
                tab.log_table.update(cx, |table, cx| {
                    table.delegate_mut().set_marked_rows(marked_rows);
                    table.refresh(cx);
                });
                tab.refresh_result_rows(row_height, cx);
                if is_marking && tab.result_mode.includes_marks() {
                    tab.results_visible = true;
                }
                changed_documents.push(document_id);
            }
            self.refresh_global_result_rows(window, cx);
            if self
                .active_document()
                .is_some_and(|tab| changed_documents.contains(&tab.id))
            {
                self.refresh_active_document_surfaces_atomically(window, cx);
            }
            self.schedule_workspace_search_state_save(window, cx);
            for document_id in changed_documents {
                self.schedule_checkpoint(document_id, window, cx);
            }
            window.push_notification(
                crate::tr_args!(
                    "{} {changed_rows} 条全局结果",
                    "{} {changed_rows} global results",
                    if is_marking {
                        crate::tr!("已标记", "Marked")
                    } else {
                        crate::tr!("已取消标记", "Unmarked")
                    },
                ),
                cx,
            );
            cx.notify();
            return;
        }
        let Some(active_ix) = self.active_ix else {
            return;
        };
        let selected_rows = self.documents[active_ix].selected_source_rows_compressed(cx);
        if selected_rows.is_empty() {
            window.push_notification(
                crate::tr!("请先选择要标记的日志行", "Select log lines to mark first"),
                cx,
            );
            return;
        }
        let row_height = self.log_row_height();
        let (document_id, is_marking) = {
            let tab = &mut self.documents[active_ix];
            let selection_is_valid = selected_rows.first().is_some_and(|row| {
                tab.document.contains_source_row(row)
                    && selected_rows
                        .get(selected_rows.len().saturating_sub(1))
                        .is_some_and(|row| tab.document.contains_source_row(row))
            });
            if !selection_is_valid {
                window.push_notification(
                    crate::tr!(
                        "所选日志行已不可用，请重新选择",
                        "The selected log lines are no longer available. Select them again."
                    ),
                    cx,
                );
                return;
            }

            let is_marking = !tab.marked_rows.contains_all(&selected_rows);
            if is_marking {
                tab.marked_rows.insert_rows(&selected_rows);
                tab.pending_restore_marked_rows.insert_rows(&selected_rows);
            } else {
                tab.marked_rows.remove_rows(&selected_rows);
                tab.pending_restore_marked_rows.remove_rows(&selected_rows);
            }
            let marked_rows = tab.marked_rows.clone();
            tab.log_table.update(cx, |table, cx| {
                table.delegate_mut().set_marked_rows(marked_rows);
                table.refresh(cx);
            });
            tab.refresh_result_rows(row_height, cx);
            if is_marking && tab.result_mode.includes_marks() {
                tab.results_visible = true;
            }
            (tab.id, is_marking)
        };
        if is_marking
            && self.global_search.result_mode.includes_marks()
            && self.global_search.selected_documents.contains(&document_id)
        {
            self.global_search.results_visible = true;
        }
        self.refresh_global_result_rows(window, cx);
        self.refresh_active_document_surfaces_atomically(window, cx);
        self.schedule_workspace_search_state_save(window, cx);
        let action = if is_marking {
            crate::tr!("已标记", "Marked")
        } else {
            crate::tr!("已取消标记", "Unmarked")
        };
        if selected_rows.len() == 1 {
            let source_row = selected_rows
                .first()
                .expect("a one-row selection has a first row");
            window.push_notification(
                crate::tr_args!("{action}第 {} 行", "{action} line {}", source_row + 1),
                cx,
            );
        } else {
            window.push_notification(
                crate::tr_args!("{action} {} 行", "{action} {} lines", selected_rows.len()),
                cx,
            );
        }
        self.schedule_checkpoint(document_id, window, cx);
        cx.notify();
    }

    fn focus_search(&mut self, _: &FocusSearch, window: &mut Window, cx: &mut Context<Self>) {
        let focus_handle = self.query.focus_handle(cx);
        focus_handle.focus(window, cx);
        self.query
            .update(cx, |state, cx| state.select_all(window, cx));
    }

    fn remember_user_log_region(&mut self, region: LogRegion) {
        self.last_user_log_region = region;
        self.active_log_region = region;
    }

    fn toggle_case_sensitive(
        &mut self,
        _: &ToggleCaseSensitive,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_search_defaults(!self.case_sensitive, self.regex, window, cx);
    }

    fn toggle_regex(&mut self, _: &ToggleRegex, window: &mut Window, cx: &mut Context<Self>) {
        self.set_search_defaults(self.case_sensitive, !self.regex, window, cx);
    }

    fn jump_to_start(&mut self, _: &JumpToStart, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.active_document() else {
            return;
        };
        if tab.document.line_count() == 0 {
            window.push_notification(
                crate::tr!("当前文件没有日志行", "The current file has no log lines"),
                cx,
            );
            return;
        }
        tab.log_table
            .update(cx, |table, cx| table.set_active_log_row(0, cx));
        self.selected_source_row = Some(0);
        cx.notify();
    }

    fn toggle_fullscreen(
        &mut self,
        _: &ToggleFullscreen,
        window: &mut Window,
        _: &mut Context<Self>,
    ) {
        window.toggle_fullscreen();
    }

    fn new_window(&mut self, _: &NewWindow, window: &mut Window, cx: &mut Context<Self>) {
        if let Err(error) = crate::open_workspace_window(cx, false, Vec::new()) {
            window.push_notification(
                crate::tr_args!(
                    "无法打开新窗口：{error}",
                    "Couldn’t open a new window: {error}"
                ),
                cx,
            );
        }
    }

    fn jump_to_end(&mut self, _: &JumpToEnd, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.active_document() else {
            return;
        };
        if tab.load_state != DocumentLoadState::Ready {
            window.push_notification(
                crate::tr!(
                    "完整索引建立后即可跳到文件末尾",
                    "Jump to the end will be available after the full index is built"
                ),
                cx,
            );
            return;
        }
        let line_count = tab.document.line_count();
        if line_count == 0 {
            window.push_notification(
                crate::tr!("当前文件没有日志行", "The current file has no log lines"),
                cx,
            );
            return;
        }
        let last_row = line_count - 1;
        tab.log_table
            .update(cx, |table, cx| table.set_active_log_row(last_row, cx));
        self.selected_source_row = Some(last_row);
        cx.notify();
    }

    fn toggle_auto_follow(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(active_ix) = self.active_ix else {
            return;
        };
        let tab = &mut self.documents[active_ix];
        if tab.load_state != DocumentLoadState::Ready {
            window.push_notification(
                crate::tr!(
                    "完整索引建立后即可开启末尾跟随",
                    "Follow end will be available after the full index is built"
                ),
                cx,
            );
            return;
        }
        tab.auto_follow = !tab.auto_follow;
        if tab.auto_follow && tab.document.line_count() > 0 {
            let last_row = tab.document.line_count() - 1;
            tab.log_table
                .update(cx, |table, cx| table.set_active_log_row(last_row, cx));
            self.selected_source_row = Some(last_row);
            window.push_notification(crate::tr!("已开启末尾跟随", "Follow end enabled"), cx);
        } else if !tab.auto_follow {
            window.push_notification(crate::tr!("已关闭末尾跟随", "Follow end disabled"), cx);
        }
        let document_id = tab.id;
        self.schedule_checkpoint(document_id, window, cx);
        cx.notify();
    }

    fn toggle_line_numbers(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(active_ix) = self.active_ix else {
            return;
        };
        let document_id = {
            let tab = &mut self.documents[active_ix];
            tab.show_line_numbers = !tab.show_line_numbers;
            tab.uses_default_view_options = false;
            tab.refresh_view_options(cx);
            tab.id
        };
        self.schedule_checkpoint(document_id, window, cx);
        cx.notify();
    }

    fn toggle_row_separators(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(active_ix) = self.active_ix else {
            return;
        };
        let document_id = {
            let tab = &mut self.documents[active_ix];
            tab.show_row_separators = !tab.show_row_separators;
            tab.uses_default_view_options = false;
            tab.refresh_view_options(cx);
            tab.id
        };
        self.schedule_checkpoint(document_id, window, cx);
        cx.notify();
    }

    fn highlight_styles(
        highlights: &[(Range<usize>, TextHighlight)],
        cx: &App,
    ) -> Vec<(Range<usize>, HighlightStyle)> {
        highlights
            .iter()
            .cloned()
            .map(|(range, highlight)| (range, text_highlight_style(highlight, cx)))
            .collect()
    }

    fn log_region_surface(
        &self,
        document_id: u64,
        region: WrappedRegion,
    ) -> Option<Entity<LogRegionSurface>> {
        if region == WrappedRegion::GlobalResults {
            return Some(self.global_surface.clone());
        }
        let tab = self.documents.iter().find(|tab| tab.id == document_id)?;
        Some(if region == WrappedRegion::Results {
            tab.result_surface.clone()
        } else {
            tab.log_surface.clone()
        })
    }

    fn is_text_selection_origin_in_log_region(&self, position: Point<Pixels>) -> bool {
        let Some(tab) = self.active_document() else {
            return false;
        };
        let log_bounds = self
            .row_drag_bounds
            .get(&(tab.id, WrappedRegion::Log))
            .copied();
        let result_bounds = match self.global_search.scope {
            SearchScope::CurrentFile if tab.results_visible => self
                .row_drag_bounds
                .get(&(tab.id, WrappedRegion::Results))
                .copied(),
            SearchScope::AllOpenFiles | SearchScope::Directory
                if self.global_search.results_visible =>
            {
                self.row_drag_bounds
                    .get(&(0, WrappedRegion::GlobalResults))
                    .copied()
            }
            _ => None,
        };
        point_in_text_selection_regions(position, [log_bounds, result_bounds].into_iter().flatten())
    }

    fn update_wrapped_height(
        &mut self,
        document_id: u64,
        region: WrappedRegion,
        row_ix: usize,
        height: Pixels,
        base_height: Pixels,
        cx: &mut Context<Self>,
    ) {
        let changed = match region {
            WrappedRegion::Log | WrappedRegion::Results => self
                .documents
                .iter()
                .position(|tab| tab.id == document_id)
                .is_some_and(|tab_ix| {
                    let state = if region == WrappedRegion::Log {
                        &self.documents[tab_ix].log_viewport
                    } else {
                        &self.documents[tab_ix].result_viewport
                    };
                    state.queue_wrapped_measured_height(row_ix, height, base_height)
                }),
            WrappedRegion::GlobalResults => {
                self.global_viewport
                    .queue_wrapped_measured_height(row_ix, height, base_height)
            }
        };
        if changed && let Some(surface) = self.log_region_surface(document_id, region) {
            surface.update(cx, |_, cx| cx.notify());
        }
    }

    fn select_wrapped_log_row(
        &mut self,
        document_id: u64,
        region: WrappedRegion,
        row_ix: usize,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab_ix) = self.documents.iter().position(|tab| tab.id == document_id) else {
            return;
        };
        let focus = if region == WrappedRegion::Results {
            self.documents[tab_ix].result_focus_handle.clone()
        } else {
            self.documents[tab_ix].log_focus_handle.clone()
        };
        focus.focus(window, cx);
        self.remember_user_log_region(if region == WrappedRegion::Results {
            LogRegion::CurrentResults
        } else {
            LogRegion::Body
        });
        let table = if region == WrappedRegion::Results {
            self.documents[tab_ix].result_table.clone()
        } else {
            self.documents[tab_ix].log_table.clone()
        };
        if event.modifiers.control || event.modifiers.shift || event.click_count >= 3 {
            GlobalState::suppress_text_selection(cx);
            TextSelection::clear(window, cx);
        }
        table.update(cx, |table, _| {
            table.delegate().begin_pointer_selection(
                row_ix,
                event.modifiers.control,
                event.modifiers.shift,
                event.click_count,
            );
        });
        window.defer(cx, move |_, cx| {
            table.update(cx, |table, cx| {
                table.set_active_log_row(row_ix, cx);
            });
        });
    }

    fn handle_row_drag_move(
        &mut self,
        document_id: u64,
        region: WrappedRegion,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let document_id = if region == WrappedRegion::GlobalResults {
            0
        } else {
            document_id
        };
        if !event.dragging() {
            self.end_row_drag_selection(document_id, region, window, cx);
            return;
        }
        if let Some(drag) = self
            .row_drag_selection
            .as_mut()
            .filter(|drag| drag.document_id == document_id && drag.region == region)
        {
            drag.pointer = event.position;
            self.schedule_row_drag_frame(window, cx);
            return;
        }
        let (start_row, text_selection_allowed) = if region == WrappedRegion::GlobalResults {
            let delegate = self.global_table.read(cx);
            let delegate = delegate.delegate();
            let Some(start_row) = delegate.pointer_drag_anchor() else {
                return;
            };
            (start_row, delegate.pointer_text_selection_allowed())
        } else {
            let Some(tab) = self.documents.iter().find(|tab| tab.id == document_id) else {
                return;
            };
            let table = if region == WrappedRegion::Results {
                &tab.result_table
            } else {
                &tab.log_table
            };
            let delegate = table.read(cx);
            let delegate = delegate.delegate();
            let Some(start_row) = delegate.pointer_drag_anchor() else {
                return;
            };
            (start_row, delegate.pointer_text_selection_allowed())
        };
        let mode = if text_selection_allowed {
            RowDragMode::Text
        } else {
            RowDragMode::Lines
        };
        self.row_drag_selection = Some(RowDragSelection {
            document_id,
            region,
            pointer: event.position,
            start_row,
            target_row: start_row,
            mode,
        });
        self.schedule_row_drag_frame(window, cx);
    }

    fn end_row_drag_selection(
        &mut self,
        document_id: u64,
        region: WrappedRegion,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let document_id = if region == WrappedRegion::GlobalResults {
            0
        } else {
            document_id
        };
        let active_drag = self
            .row_drag_selection
            .is_some_and(|drag| drag.document_id == document_id && drag.region == region);
        let pointer_state_active = if region == WrappedRegion::GlobalResults {
            let table = self.global_table.read(cx);
            let delegate = table.delegate();
            delegate.is_pointer_selecting() || delegate.is_text_selection_suppressed()
        } else {
            self.documents
                .iter()
                .find(|tab| tab.id == document_id)
                .is_some_and(|tab| {
                    let table = if region == WrappedRegion::Results {
                        &tab.result_table
                    } else {
                        &tab.log_table
                    };
                    let table = table.read(cx);
                    let delegate = table.delegate();
                    delegate.is_pointer_selecting() || delegate.is_text_selection_suppressed()
                })
        };
        if !active_drag && !pointer_state_active {
            return;
        }
        if active_drag {
            self.advance_row_drag_selection(cx);
        }
        let changed_row_selection = self.row_drag_selection.is_some_and(|drag| {
            drag.document_id == document_id && drag.region == region && drag.changed_row_selection()
        });
        let clear_text_selection = self.row_drag_selection.is_some_and(|drag| {
            drag.document_id == document_id
                && drag.region == region
                && drag.mode == RowDragMode::Lines
        });
        self.row_drag_selection = None;
        if clear_text_selection {
            TextSelection::clear(window, cx);
        }
        if region == WrappedRegion::GlobalResults {
            self.global_table.update(cx, |table, cx| {
                table.delegate().end_pointer_selection();
                cx.notify();
            });
            self.status_surface.update(cx, |_, cx| cx.notify());
            if changed_row_selection {
                self.schedule_log_region_state_save(document_id, region, window, cx);
            }
            return;
        }
        let Some(tab) = self.documents.iter().find(|tab| tab.id == document_id) else {
            return;
        };
        let table = if region == WrappedRegion::Results {
            &tab.result_table
        } else {
            &tab.log_table
        };
        table.update(cx, |table, cx| {
            table.delegate().end_pointer_selection();
            cx.notify();
        });
        self.status_surface.update(cx, |_, cx| cx.notify());
        self.schedule_log_region_state_save(document_id, region, window, cx);
    }

    fn end_all_row_drag_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.row_drag_selection.is_some() {
            self.advance_row_drag_selection(cx);
        }
        // Result replacement clears the delegate's pointer anchor before MouseUp. Keep the
        // workspace-owned drag target so its text-selection suppression is still released.
        let active_drag = self.row_drag_selection;
        let changed_global_row_selection = active_drag.is_some_and(|drag| {
            drag.region == WrappedRegion::GlobalResults && drag.changed_row_selection()
        });
        let clear_text_selection = self
            .row_drag_selection
            .is_some_and(|drag| drag.mode == RowDragMode::Lines);
        self.row_drag_selection = None;
        if clear_text_selection {
            TextSelection::clear(window, cx);
        }
        let mut ended_selection = false;
        let mut ended_document_selections = BTreeSet::new();
        for tab in &self.documents {
            for (region, table) in [
                (WrappedRegion::Log, &tab.log_table),
                (WrappedRegion::Results, &tab.result_table),
            ] {
                let active_drag_targets_table = active_drag
                    .is_some_and(|drag| drag.document_id == tab.id && drag.region == region);
                let needs_cleanup = {
                    let table = table.read(cx);
                    let delegate = table.delegate();
                    active_drag_targets_table
                        || delegate.is_pointer_selecting()
                        || delegate.is_text_selection_suppressed()
                };
                if !needs_cleanup {
                    continue;
                }
                ended_selection = true;
                ended_document_selections.insert(tab.id);
                table.update(cx, |table, cx| {
                    table.delegate().end_pointer_selection();
                    cx.notify();
                });
            }
        }
        let ended_global_selection = {
            let table = self.global_table.read(cx);
            let delegate = table.delegate();
            active_drag.is_some_and(|drag| drag.region == WrappedRegion::GlobalResults)
                || delegate.is_pointer_selecting()
                || delegate.is_text_selection_suppressed()
        };
        if ended_global_selection {
            ended_selection = true;
            self.global_table.update(cx, |table, cx| {
                table.delegate().end_pointer_selection();
                cx.notify();
            });
        }
        if ended_selection {
            self.status_surface.update(cx, |_, cx| cx.notify());
        }
        for document_id in ended_document_selections {
            self.schedule_checkpoint(document_id, window, cx);
        }
        if ended_global_selection && changed_global_row_selection {
            self.schedule_workspace_search_state_save(window, cx);
        }
    }

    fn schedule_row_drag_frame(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.row_drag_frame_scheduled || self.row_drag_selection.is_none() {
            return;
        }
        self.row_drag_frame_scheduled = true;
        cx.on_next_frame(window, |this, window, cx| {
            this.row_drag_frame_scheduled = false;
            if this.advance_row_drag_selection(cx) {
                this.schedule_row_drag_frame(window, cx);
            }
        });
    }

    fn advance_row_drag_selection(&mut self, cx: &mut Context<Self>) -> bool {
        const EDGE: f32 = 32.;
        let Some(drag) = self.row_drag_selection else {
            return false;
        };
        let Some(bounds) = self
            .row_drag_bounds
            .get(&(drag.document_id, drag.region))
            .copied()
        else {
            return false;
        };
        if drag.region == WrappedRegion::GlobalResults {
            return self.advance_global_row_drag_selection(drag, bounds, cx);
        }
        let Some(tab_ix) = self
            .documents
            .iter()
            .position(|tab| tab.id == drag.document_id)
        else {
            return false;
        };
        let base_height = self.log_row_height();
        // Hidden-header tables and wrapped lists both start at the region's top edge.
        // Treating one row as a header made an in-row text drag hit a neighbour.
        let content_top = bounds.origin.y;
        let content_bottom = bounds.origin.y + bounds.size.height;
        let viewport_height = (content_bottom - content_top).max(base_height);
        let visible_rows = (viewport_height / base_height).floor().max(1.) as usize;
        let distance_above = (content_top + px(EDGE) - drag.pointer.y).max(px(0.));
        let distance_below = (drag.pointer.y - (content_bottom - px(EDGE))).max(px(0.));
        let edge_direction = if distance_above > px(0.) {
            Some((-1_isize, distance_above))
        } else if distance_below > px(0.) {
            Some((1_isize, distance_below))
        } else {
            None
        };

        let table = if drag.region == WrappedRegion::Results {
            self.documents[tab_ix].result_table.clone()
        } else {
            self.documents[tab_ix].log_table.clone()
        };
        let count = table.read(cx).delegate().row_count();
        if count == 0 || !table.read(cx).delegate().is_pointer_selecting() {
            return false;
        }
        let viewport = if drag.region == WrappedRegion::Results {
            &self.documents[tab_ix].result_viewport
        } else {
            &self.documents[tab_ix].log_viewport
        };
        let current_top = viewport.first_visible(count, base_height);
        let text_selection_allowed = table.read(cx).delegate().pointer_text_selection_allowed();
        let crossed_viewport_edge =
            drag.pointer.y < content_top || drag.pointer.y >= content_bottom;
        let pointer_after = drag.pointer.y >= content_bottom;
        let direct_target = viewport
            .row_at_position(drag.pointer)
            .or_else(|| {
                crossed_viewport_edge
                    .then(|| viewport.visible_row_edge(pointer_after))
                    .flatten()
            })
            .unwrap_or(drag.target_row);
        let line_mode =
            !text_selection_allowed || direct_target != drag.start_row || crossed_viewport_edge;
        let edge_direction = line_mode.then_some(edge_direction).flatten();
        let (target, scroll_top, keep_scrolling) =
            if let Some((direction, distance)) = edge_direction {
                let step = ((distance.as_f32() / EDGE * 7.).ceil() as usize + 1).min(8);
                let scroll_top = if direction < 0 {
                    current_top.saturating_sub(step)
                } else {
                    current_top
                        .saturating_add(step)
                        .min(count.saturating_sub(visible_rows))
                };
                let target = if direction < 0 {
                    scroll_top
                } else {
                    scroll_top
                        .saturating_add(visible_rows.saturating_sub(1))
                        .min(count - 1)
                };
                (target, Some(scroll_top), true)
            } else {
                (direct_target, None, false)
            };

        let scroll_changed = scroll_top.is_some_and(|scroll_top| scroll_top != current_top);
        if let Some(scroll_top) = scroll_top.filter(|_| scroll_changed) {
            viewport.place_at_top(scroll_top, base_height);
        }
        let next_mode = if !line_mode {
            RowDragMode::Text
        } else {
            RowDragMode::Lines
        };
        let selection_changed = target != drag.target_row || next_mode != drag.mode;
        if !selection_changed {
            return keep_scrolling && scroll_changed;
        }
        if let Some(active_drag) = self.row_drag_selection.as_mut() {
            active_drag.mode = next_mode;
            active_drag.target_row = target;
        }
        table.update(cx, |table, cx| {
            table
                .delegate()
                .set_text_selection_suppressed(next_mode == RowDragMode::Lines);
            if next_mode == RowDragMode::Text || target == drag.start_row {
                table.delegate().restore_pointer_selection();
            } else {
                table.delegate().extend_pointer_selection(target);
            }
            cx.notify();
        });
        let source_row = table.read(cx).delegate().source_row(target);
        let tab = &mut self.documents[tab_ix];
        tab.auto_follow = false;
        tab.selection_table = if drag.region == WrappedRegion::Results {
            SelectionTable::Results
        } else {
            SelectionTable::Log
        };
        self.selected_source_row = source_row;
        keep_scrolling && scroll_changed
    }

    /// Runs from the viewport's prepaint observer before the wrapped-list child is prepainted.
    /// Priming the shared measurements here makes the child's first visible frame self-consistent.
    fn update_wrapped_layout(
        &mut self,
        document_id: u64,
        region: WrappedRegion,
        width: Pixels,
        viewport_height: Pixels,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        if width <= px(0.) || viewport_height <= px(0.) {
            return;
        }
        let base_height = self.log_row_height();
        let horizontal_padding = log_cell_horizontal_padding(cx);
        let text_width = (width - horizontal_padding * 2.).max(px(0.));
        let changed = match region {
            WrappedRegion::Log | WrappedRegion::Results => {
                let Some(tab_ix) = self.documents.iter().position(|tab| tab.id == document_id)
                else {
                    return;
                };
                let viewport = if region == WrappedRegion::Results {
                    &self.documents[tab_ix].result_viewport
                } else {
                    &self.documents[tab_ix].log_viewport
                };
                if !viewport.is_wrapped() {
                    return;
                }
                let table = if region == WrappedRegion::Results {
                    self.documents[tab_ix].result_table.clone()
                } else {
                    self.documents[tab_ix].log_table.clone()
                };
                let (count, font_size, font_family, key, preferred) = {
                    let table = table.read(cx);
                    let delegate = table.delegate();
                    let font_size = delegate.log_font_size();
                    let font_family = delegate.resolved_font_family(cx);
                    (
                        delegate.row_count(),
                        font_size,
                        font_family.clone(),
                        Self::wrapped_layout_key(
                            delegate.content_revision(),
                            width,
                            font_size,
                            font_family,
                            base_height,
                            window.rem_size(),
                            horizontal_padding,
                        ),
                        table.active_log_row(),
                    )
                };
                let viewport = if region == WrappedRegion::Results {
                    &self.documents[tab_ix].result_viewport
                } else {
                    &self.documents[tab_ix].log_viewport
                };
                let changed =
                    viewport.invalidate_wrapped_layout_preserving_position(key, preferred);
                viewport.wrapped_sizes(count, base_height);
                let range = wrapped_viewport_measurement_range(
                    viewport.wrapped_first_visible_row(),
                    viewport_height,
                    base_height,
                    count,
                );
                let unknown_rows = range
                    .filter(|row_ix| !viewport.has_known_wrapped_row_height(*row_ix))
                    .collect::<Vec<_>>();
                let rows = {
                    let table = table.read(cx);
                    unknown_rows
                        .into_iter()
                        .filter_map(|row_ix| {
                            table
                                .delegate()
                                .wrapped_row(row_ix)
                                .map(|row| (row_ix, row.text.display().clone()))
                        })
                        .collect::<Vec<_>>()
                };
                let heights = rows.into_iter().map(|(row_ix, line)| {
                    (
                        row_ix,
                        Self::measure_wrapped_line_height(
                            line,
                            text_width,
                            font_size,
                            &font_family,
                            base_height,
                            window,
                        ),
                    )
                });
                viewport.prime_wrapped_measured_heights(count, base_height, heights);
                changed
            }
            WrappedRegion::GlobalResults => {
                if !self.global_viewport.is_wrapped() {
                    return;
                }
                let (count, font_size, font_family, key, preferred) = {
                    let table = self.global_table.read(cx);
                    let delegate = table.delegate();
                    let font_size = delegate.log_font_size();
                    let font_family = delegate.resolved_font_family(cx);
                    (
                        delegate.rows_len(),
                        font_size,
                        font_family.clone(),
                        Self::wrapped_layout_key(
                            delegate.content_revision(),
                            width,
                            font_size,
                            font_family,
                            base_height,
                            window.rem_size(),
                            horizontal_padding,
                        ),
                        table.active_log_row(),
                    )
                };
                let changed = self
                    .global_viewport
                    .invalidate_wrapped_layout_preserving_position(key, preferred);
                self.global_viewport.wrapped_sizes(count, base_height);
                let range = wrapped_viewport_measurement_range(
                    self.global_viewport.wrapped_first_visible_row(),
                    viewport_height,
                    base_height,
                    count,
                );
                let unknown_rows = range
                    .filter(|row_ix| !self.global_viewport.has_known_wrapped_row_height(*row_ix))
                    .collect::<Vec<_>>();
                let rows = {
                    let table = self.global_table.read(cx);
                    unknown_rows
                        .into_iter()
                        .filter_map(|row_ix| match table.delegate().wrapped_row(row_ix)? {
                            WrappedGlobalRow::Match { text, .. } => {
                                Some((row_ix, text.display().clone()))
                            }
                            WrappedGlobalRow::Group { .. } => None,
                        })
                        .collect::<Vec<_>>()
                };
                let heights = rows.into_iter().map(|(row_ix, line)| {
                    (
                        row_ix,
                        Self::measure_wrapped_line_height(
                            line,
                            text_width,
                            font_size,
                            &font_family,
                            base_height,
                            window,
                        ),
                    )
                });
                self.global_viewport
                    .prime_wrapped_measured_heights(count, base_height, heights);
                changed
            }
        };
        if changed && let Some(surface) = self.log_region_surface(document_id, region) {
            surface.update(cx, |_, cx| cx.notify());
        }
    }

    fn advance_global_row_drag_selection(
        &mut self,
        drag: RowDragSelection,
        bounds: Bounds<Pixels>,
        cx: &mut Context<Self>,
    ) -> bool {
        const EDGE: f32 = 32.;
        let base_height = self.log_row_height();
        let content_top = bounds.origin.y;
        let content_bottom = bounds.origin.y + bounds.size.height;
        let viewport_height = (content_bottom - content_top).max(base_height);
        let visible_rows = (viewport_height / base_height).floor().max(1.) as usize;
        let count = self.global_table.read(cx).delegate().rows_len();
        if count == 0 || !self.global_table.read(cx).delegate().is_pointer_selecting() {
            return false;
        }
        let current_top = self.global_viewport.first_visible(count, base_height);
        let text_selection_allowed = self
            .global_table
            .read(cx)
            .delegate()
            .pointer_text_selection_allowed();
        let crossed_viewport_edge =
            drag.pointer.y < content_top || drag.pointer.y >= content_bottom;
        let pointer_after = drag.pointer.y >= content_bottom;
        let direct_candidate = self
            .global_viewport
            .row_at_position(drag.pointer)
            .or_else(|| {
                crossed_viewport_edge
                    .then(|| self.global_viewport.visible_row_edge(pointer_after))
                    .flatten()
            })
            .unwrap_or(drag.target_row);
        let line_mode =
            !text_selection_allowed || direct_candidate != drag.start_row || crossed_viewport_edge;
        let distance_above = (content_top + px(EDGE) - drag.pointer.y).max(px(0.));
        let distance_below = (drag.pointer.y - (content_bottom - px(EDGE))).max(px(0.));
        let edge = if !line_mode {
            None
        } else if distance_above > px(0.) {
            Some((-1_isize, distance_above))
        } else if distance_below > px(0.) {
            Some((1_isize, distance_below))
        } else {
            None
        };
        let (candidate, scroll_top, keep_scrolling) = if let Some((direction, distance)) = edge {
            let step = ((distance.as_f32() / EDGE * 7.).ceil() as usize + 1).min(8);
            let scroll_top = if direction < 0 {
                current_top.saturating_sub(step)
            } else {
                current_top
                    .saturating_add(step)
                    .min(count.saturating_sub(visible_rows))
            };
            let candidate = if direction < 0 {
                scroll_top
            } else {
                scroll_top
                    .saturating_add(visible_rows.saturating_sub(1))
                    .min(count - 1)
            };
            (candidate, Some(scroll_top), true)
        } else {
            (direct_candidate, None, false)
        };
        let prefer_after = candidate >= drag.start_row;
        let Some(target) = self
            .global_table
            .read(cx)
            .delegate()
            .nearest_match_row(candidate, prefer_after)
        else {
            return keep_scrolling;
        };
        let scroll_changed = scroll_top.is_some_and(|scroll_top| scroll_top != current_top);
        if let Some(scroll_top) = scroll_top.filter(|_| scroll_changed) {
            self.global_viewport.place_at_top(scroll_top, base_height);
        }
        let next_mode = if !line_mode {
            RowDragMode::Text
        } else {
            RowDragMode::Lines
        };
        let selection_changed = target != drag.target_row || next_mode != drag.mode;
        if !selection_changed {
            return keep_scrolling && scroll_changed;
        }
        if let Some(active_drag) = self.row_drag_selection.as_mut() {
            active_drag.mode = next_mode;
            active_drag.target_row = target;
        }
        self.global_table.update(cx, |table, cx| {
            table
                .delegate()
                .set_text_selection_suppressed(next_mode == RowDragMode::Lines);
            if next_mode == RowDragMode::Text || target == drag.start_row {
                table.delegate().restore_pointer_selection();
            } else {
                table.delegate().extend_pointer_selection(target);
            }
            cx.notify();
        });
        keep_scrolling && scroll_changed
    }

    fn render_wrapped_log_rows(
        &mut self,
        document_id: u64,
        region: WrappedRegion,
        visible_range: Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let _performance_scope = crate::ui_performance::scope("Workspace::render_wrapped_log_rows");
        let Some(tab_ix) = self.documents.iter().position(|tab| tab.id == document_id) else {
            return Vec::new();
        };
        let table = if region == WrappedRegion::Results {
            self.documents[tab_ix].result_table.clone()
        } else {
            self.documents[tab_ix].log_table.clone()
        };
        self.schedule_local_visible_lines(document_id, region, visible_range.clone(), cx);
        let (
            show_line_numbers,
            show_row_separators,
            show_line_number_row_separators,
            line_number_width,
            line_number_text_color,
            line_number_background_color,
            font_size,
            font_family,
        ) = {
            let table = table.read(cx);
            let delegate = table.delegate();
            (
                delegate.show_line_numbers(),
                delegate.show_row_separators(),
                delegate.show_line_number_row_separators(),
                delegate.line_number_width(),
                delegate.line_number_text_color(cx),
                delegate.line_number_background_color(cx),
                delegate.log_font_size(),
                delegate.resolved_font_family(cx),
            )
        };
        let base_height = self.log_row_height();
        let marker_width = line_marker_column_width();
        let fixed_columns_width = marker_width
            + if show_line_numbers {
                px(line_number_width as f32)
            } else {
                px(0.)
            };
        let workspace = cx.weak_entity();
        let suppress_text_selection = self.row_drag_selection.is_some_and(|drag| {
            drag.document_id == document_id
                && drag.region == region
                && drag.mode == RowDragMode::Lines
        });
        let rendered_row_bounds = {
            let wrapped = if region == WrappedRegion::Results {
                &self.documents[tab_ix].result_viewport
            } else {
                &self.documents[tab_ix].log_viewport
            };
            wrapped.retain_wrapped_visible_rows(&visible_range);
            wrapped.wrapped_row_bounds()
        };

        visible_range
            .filter_map(|row_ix| {
                let row = table.read(cx).delegate().wrapped_row(row_ix)?;
                let selected_above =
                    row_ix > 0 && table.read(cx).delegate().is_row_selected(row_ix - 1);
                let selected_below = row_ix + 1 < table.read(cx).delegate().row_count()
                    && table.read(cx).delegate().is_row_selected(row_ix + 1);
                let source_row = row.source_row;
                let source_unavailable = row.source_unavailable;
                let selection = {
                    let viewport = if region == WrappedRegion::Results {
                        &self.documents[tab_ix].result_viewport
                    } else {
                        &self.documents[tab_ix].log_viewport
                    };
                    viewport.wrapped_selection(source_row, &row.text, window, cx)
                };
                let styled_text = StyledText::new(row.text.display().clone())
                    .with_highlights(Self::highlight_styles(&row.highlights, cx));
                let severity = (!source_unavailable && row.highlight_severity)
                    .then(|| row.text.severity())
                    .flatten()
                    .map(|severity| severity_style(severity, cx));
                let measure_workspace = workspace.clone();
                let row_bounds = rendered_row_bounds.clone();
                let line = SelectableLogText::new(
                    selection,
                    source_row as u64,
                    row.text,
                    styled_text,
                    ui_theme::text_selection_highlight(cx),
                )
                .suppress_selection(suppress_text_selection)
                .word_boundary_characters(self.app_settings.word_boundary_characters.clone())
                .on_measure(move |height, _, cx| {
                    _ = measure_workspace.update(cx, |this, cx| {
                        this.update_wrapped_height(
                            document_id,
                            region,
                            row_ix,
                            height,
                            base_height,
                            cx,
                        );
                    });
                });
                Some(
                    div()
                        .id(format!(
                            "wrapped-log-row-{document_id}-{}-{source_row}",
                            region as u8,
                        ))
                        .on_prepaint(move |bounds, _, _| {
                            row_bounds.borrow_mut().insert(row_ix, bounds);
                        })
                        .relative()
                        .w_full()
                        .min_h(base_height)
                        .flex()
                        .items_start()
                        .when_some(severity, |row, style| {
                            row.bg(style.background)
                                .child(severity_accent_overlay(style.accent))
                        })
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                                this.select_wrapped_log_row(
                                    document_id,
                                    region,
                                    row_ix,
                                    event,
                                    window,
                                    cx,
                                );
                            }),
                        )
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                                this.prepare_wrapped_log_context(
                                    document_id,
                                    region,
                                    row_ix,
                                    window,
                                    cx,
                                );
                            }),
                        )
                        .child(
                            h_flex()
                                .w(marker_width)
                                .self_stretch()
                                .flex_none()
                                .justify_center()
                                .child(line_marker(row.marked, row.matched, cx)),
                        )
                        .when(show_line_numbers, |row| {
                            row.child(
                                log_line_number_cell(
                                    source_row,
                                    font_size,
                                    base_height,
                                    line_number_text_color,
                                    line_number_background_color,
                                    show_line_number_row_separators,
                                    cx,
                                )
                                .w(px(line_number_width as f32))
                                .self_stretch()
                                .flex_none(),
                            )
                        })
                        .child(
                            div()
                                .relative()
                                .min_w_0()
                                .flex_1()
                                .overflow_hidden()
                                .whitespace_normal()
                                .px(log_cell_horizontal_padding(cx))
                                .text_size(px(font_size as f32))
                                .line_height(base_height)
                                .font_family(font_family.clone())
                                .when(source_unavailable, |cell| {
                                    cell.text_color(cx.theme().danger)
                                })
                                .when(row.selected, |cell| {
                                    cell.bg(log_row_selection_color(cx)).child(
                                        log_row_selection_overlay(
                                            !selected_above,
                                            !selected_below,
                                            cx,
                                        ),
                                    )
                                })
                                .when(show_row_separators && !row.selected, |cell| {
                                    cell.child(log_row_separator_overlay(false, cx))
                                })
                                .child(line),
                        )
                        .child(log_fixed_column_divider_overlay(fixed_columns_width, cx))
                        .into_any_element(),
                )
            })
            .collect()
    }

    fn render_wrapped_log_table(
        &self,
        document_id: u64,
        region: WrappedRegion,
        surface: Entity<LogRegionSurface>,
        workspace: WeakEntity<Workspace>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let _performance_scope =
            crate::ui_performance::scope("Workspace::render_wrapped_log_table");
        let Some(tab) = self.documents.iter().find(|tab| tab.id == document_id) else {
            return div().into_any_element();
        };
        let table = if region == WrappedRegion::Results {
            &tab.result_table
        } else {
            &tab.log_table
        };
        let delegate = table.read(cx).delegate();
        let count = delegate.row_count();
        let base_height = self.log_row_height();
        let wrapped = if region == WrappedRegion::Results {
            &tab.result_viewport
        } else {
            &tab.log_viewport
        };
        if wrapped.wrapped_base_height() != base_height {
            wrapped.ensure_wrapped_measurement_anchor(table.read(cx).active_log_row());
        }
        let sizes = wrapped.wrapped_sizes(count, base_height);
        let scroll_handle = wrapped.wrapped_scroll_handle();
        let list_scroll = scroll_handle.clone();
        let logical_scroll = wrapped.wrapped_logical_scroll_handle(count, base_height);
        let list_id = format!("wrapped-{}-{}", document_id, region as u8);
        let scrollbar_background = *cx.theme().tokens.table;

        let element = v_flex()
            .size_full()
            .min_h_0()
            .bg(cx.theme().tokens.table)
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .key_context("DataTable")
                    .child(crate::ui_performance::element(
                        "WrappedLogVirtualList::request_layout",
                        "WrappedLogVirtualList::prepaint",
                        "WrappedLogVirtualList::paint",
                        sparse_v_virtual_list(
                            surface,
                            list_id,
                            sizes,
                            move |_, range, window, cx| {
                                workspace
                                    .update(cx, |workspace, cx| {
                                        workspace.render_wrapped_log_rows(
                                            document_id,
                                            region,
                                            range,
                                            window,
                                            cx,
                                        )
                                    })
                                    .unwrap_or_default()
                            },
                        )
                        .track_scroll(&list_scroll)
                        .size_full(),
                    ))
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .right_0()
                            .bottom_0()
                            .w(Scrollbar::width())
                            .bg(scrollbar_background)
                            .child(
                                persistent_log_scrollbar(
                                    Scrollbar::vertical(&logical_scroll)
                                        .id(format!(
                                            "wrapped-log-vertical-scrollbar-{document_id}-{}",
                                            region as u8
                                        ))
                                        .viewport_from_layout(),
                                    scrollbar_background,
                                )
                                .max_fps(60),
                            ),
                    ),
            );
        crate::ui_performance::element(
            "WrappedLogTable::request_layout",
            "WrappedLogTable::prepaint",
            "WrappedLogTable::paint",
            element,
        )
        .into_any_element()
    }

    fn select_wrapped_global_row(
        &mut self,
        row_ix: usize,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.global_results_focus_handle.focus(window, cx);
        self.remember_user_log_region(LogRegion::GlobalResults);
        let is_match = matches!(
            self.global_table.read(cx).delegate().row(row_ix),
            Some(GlobalSearchRow::Match { .. })
        );
        if is_match && (event.modifiers.control || event.modifiers.shift || event.click_count >= 3)
        {
            GlobalState::suppress_text_selection(cx);
            TextSelection::clear(window, cx);
        }
        self.global_table.update(cx, |table, _| {
            if is_match {
                table.delegate().begin_pointer_selection(
                    row_ix,
                    event.modifiers.control,
                    event.modifiers.shift,
                    event.click_count,
                );
            }
        });
        if is_match {
            let table = self.global_table.clone();
            window.defer(cx, move |_, cx| {
                table.update(cx, |table, cx| {
                    table.set_active_log_row(row_ix, cx);
                });
            });
        } else {
            self.global_table
                .update(cx, |table, cx| table.set_active_log_row(row_ix, cx));
        }
    }

    fn prepare_wrapped_log_context(
        &mut self,
        document_id: u64,
        region: WrappedRegion,
        row_ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.remember_user_log_region(if region == WrappedRegion::Results {
            LogRegion::CurrentResults
        } else {
            LogRegion::Body
        });
        let Some(tab) = self.documents.iter_mut().find(|tab| tab.id == document_id) else {
            return;
        };
        let focus = if region == WrappedRegion::Results {
            tab.result_focus_handle.clone()
        } else {
            tab.log_focus_handle.clone()
        };
        focus.focus(window, cx);
        tab.selection_table = if region == WrappedRegion::Results {
            SelectionTable::Results
        } else {
            SelectionTable::Log
        };
        let table = if region == WrappedRegion::Results {
            &tab.result_table
        } else {
            &tab.log_table
        };
        table.update(cx, |table, cx| {
            table.delegate().prepare_context_selection(row_ix);
            table.set_active_log_row(row_ix, cx);
        });
        self.selected_source_row = table.read(cx).delegate().source_row(row_ix);
        cx.notify();
    }

    fn context_color_target(
        &self,
        selected_text: Option<&str>,
        cx: &App,
    ) -> std::result::Result<(usize, ColorKeywordTarget), String> {
        if self.active_log_region == LogRegion::GlobalResults {
            let selected_groups = self
                .global_table
                .read(cx)
                .delegate()
                .selected_match_groups();
            let Some((document_id, rows)) = selected_groups.first() else {
                return Err(crate::tr!("请先选择日志行", "Select log lines first").to_string());
            };
            if selected_groups.len() > 1 {
                return Err(crate::tr!(
                    "颜色标签一次只能应用到同一文件的全局结果",
                    "A color label can be applied only to global results from one file at a time"
                )
                .to_string());
            }
            let active_ix = self
                .presentation_document_ix_for_global_result(*document_id)
                .ok_or_else(|| {
                    if self.global_search.scope == SearchScope::Directory {
                        crate::tr!(
                            "请重新搜索，或打开与该结果内容一致的文件后再应用颜色标签",
                            "Search again, or open the same file snapshot before applying a color label"
                        )
                        .to_string()
                    } else {
                        crate::tr!(
                            "所选结果所属文件已关闭",
                            "The file containing the selected results has been closed"
                        )
                        .to_string()
                    }
                })?;
            let tab = &self.documents[active_ix];
            let selection = selected_text
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map_or_else(
                    || ColorKeywordSelection::Rows(rows.clone()),
                    |text| ColorKeywordSelection::Text(text.to_string()),
                );
            return Ok((
                active_ix,
                ColorKeywordTarget {
                    document_id: tab.id,
                    document: tab.document.clone(),
                    selection,
                },
            ));
        }
        let active_ix = self.active_ix.ok_or_else(|| {
            crate::tr!("当前没有活动日志文件", "There is no active log file").to_string()
        })?;
        let tab = &self.documents[active_ix];
        let selection = selected_text
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map_or_else(
                || ColorKeywordSelection::Rows(tab.selected_source_rows_compressed(cx)),
                |text| ColorKeywordSelection::Text(text.to_string()),
            );
        Ok((
            active_ix,
            ColorKeywordTarget {
                document_id: tab.id,
                document: tab.document.clone(),
                selection,
            },
        ))
    }

    fn start_color_rule_action(
        &mut self,
        target: ColorKeywordTarget,
        action: ColorRuleAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cancel_color_rule_action();
        let revision = self.color_rule_revision;
        let collect_keywords = !matches!(
            &action,
            ColorRuleAction::Apply {
                clear_all: true,
                ..
            }
        );
        let Some(tab) = self.documents.iter().find(|tab| {
            tab.id == target.document_id && Arc::ptr_eq(&tab.document, &target.document)
        }) else {
            window.push_notification(
                crate::tr!(
                    "目标日志已刷新或关闭，请重新选择",
                    "The target log was refreshed or closed. Select it again."
                ),
                cx,
            );
            return;
        };
        let rules = tab.keyword_color_rules.clone();
        let labels = self.color_labels.clone();
        let last_color_label_id = self.last_color_label_id.clone();
        let cancellation = SearchCancellation::default();
        self.color_rule_cancellation = Some(cancellation.clone());
        self.color_rule_task = Some(cx.spawn_in(window, async move |this, cx| {
            let prepared = cx
                .background_spawn(async move {
                    prepare_color_rule_update(
                        target,
                        collect_keywords,
                        action,
                        rules,
                        labels,
                        last_color_label_id,
                        &cancellation,
                    )
                })
                .await;
            _ = this.update_in(cx, |this, window, cx| {
                if this.color_rule_revision != revision {
                    return;
                }
                this.color_rule_task = None;
                this.color_rule_cancellation = None;
                let prepared = match prepared {
                    DocumentLineTask::Completed(prepared) => prepared,
                    DocumentLineTask::Cancelled => return,
                    DocumentLineTask::SourceUnavailable => {
                        window.push_notification(
                            crate::tr!(
                                "所选日志的文件内容已改变，请重新加载后再应用颜色标签",
                                "The selected log file changed. Reload it before applying a color label."
                            ),
                            cx,
                        );
                        return;
                    }
                };
                this.finish_color_rule_update(prepared, window, cx);
            });
        }));
    }

    fn cancel_color_rule_action(&mut self) {
        self.color_rule_revision = self.color_rule_revision.saturating_add(1);
        if let Some(cancellation) = self.color_rule_cancellation.take() {
            cancellation.cancel();
        }
        self.color_rule_task = None;
    }

    fn finish_color_rule_update(
        &mut self,
        prepared: PreparedColorRuleUpdate,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(active_ix) = self.documents.iter().position(|tab| {
            tab.id == prepared.document_id && Arc::ptr_eq(&tab.document, &prepared.document)
        }) else {
            window.push_notification(
                crate::tr!(
                    "目标日志已刷新或关闭，请重新选择",
                    "The target log was refreshed or closed. Select it again."
                ),
                cx,
            );
            return;
        };
        if self.documents[active_ix].keyword_color_rules != prepared.expected_rules
            || self.color_labels != prepared.expected_labels
        {
            window.push_notification(
                crate::tr!(
                    "颜色设置已发生变化，请重新应用",
                    "Color settings changed. Apply the selection again."
                ),
                cx,
            );
            return;
        }
        let notification = match &prepared.outcome {
            ColorRuleOutcome::EmptyKeywords => {
                window.push_notification(
                    crate::tr!(
                        "请先选择包含文字的日志行",
                        "Select log lines containing text first"
                    ),
                    cx,
                );
                return;
            }
            ColorRuleOutcome::MissingLabels => {
                window.push_notification(
                    crate::tr!(
                        "请先在“颜色标签…”中添加标签",
                        "Add a label in Color labels… first"
                    ),
                    cx,
                );
                return;
            }
            ColorRuleOutcome::MissingLabel => {
                window.push_notification(
                    crate::tr!("颜色标签已不存在", "The color label no longer exists"),
                    cx,
                );
                return;
            }
            ColorRuleOutcome::CycleRemoved { count } => crate::tr_args!(
                "已移除 {} 行文字的颜色标签",
                "Removed color labels from {} lines of text",
                count
            ),
            ColorRuleOutcome::CycleApplied { label, count } => crate::tr_args!(
                "已用“{}”高亮 {} 行文字",
                "Highlighted “{}” in {} lines of text",
                label.localized_name(),
                count
            ),
            ColorRuleOutcome::Applied => {
                crate::tr!("已应用颜色标签", "Color label applied").to_string()
            }
            ColorRuleOutcome::Removed => {
                crate::tr!("已移除颜色标签", "Color label removed").to_string()
            }
            ColorRuleOutcome::Cleared => crate::tr!(
                "已清除当前文件的所有颜色",
                "Cleared all colors from the current file"
            )
            .to_string(),
        };
        let Some(resolved) = prepared.resolved else {
            debug_assert!(
                false,
                "successful color updates must resolve their matchers"
            );
            return;
        };
        self.documents[active_ix].keyword_color_rules = prepared.rules;
        self.documents[active_ix].resolved_color_rules = resolved.clone();
        self.last_color_label_id = prepared.last_color_label_id;
        for table in [
            self.documents[active_ix].log_table.clone(),
            self.documents[active_ix].result_table.clone(),
        ] {
            table.update(cx, |table, cx| {
                table.delegate_mut().set_color_rules(resolved.clone());
                table.refresh(cx);
            });
        }
        let document_id = self.documents[active_ix].id;
        self.refresh_global_result_rows(window, cx);
        self.schedule_checkpoint(document_id, window, cx);
        window.push_notification(notification, cx);
        cx.notify();
    }

    fn open_document_ix_for_global_result(&self, document_id: u64) -> Option<usize> {
        let result_path = self
            .global_search
            .results
            .get(&document_id)
            .map(|result| result.path.as_path());
        self.documents.iter().position(|tab| {
            tab.id == document_id
                || result_path.is_some_and(|path| paths_match(tab.document.path(), path))
        })
    }

    fn presentation_document_ix_for_global_result(&self, document_id: u64) -> Option<usize> {
        let document_ix = self.open_document_ix_for_global_result(document_id)?;
        let Some(result) = self.global_search.results.get(&document_id) else {
            return Some(document_ix);
        };
        let open_document = &self.documents.get(document_ix)?.document;
        result_snapshot_matches_document(&result.path, &result.document, open_document)
            .then_some(document_ix)
    }

    fn resolve_global_mark_targets(
        &self,
        selected_rows: &BTreeMap<u64, CompressedRows>,
    ) -> Option<BTreeMap<u64, CompressedRows>> {
        let directory_results = self.global_search.result_scope == Some(SearchScope::Directory);
        group_result_rows_by_document(selected_rows, |result_document_id, rows| {
            let document_ix = if directory_results {
                self.presentation_document_ix_for_global_result(result_document_id)?
            } else {
                self.documents
                    .iter()
                    .position(|tab| tab.id == result_document_id)?
            };
            let tab = self.documents.get(document_ix)?;
            let first = rows.first()?;
            let last = rows.get(rows.len().saturating_sub(1))?;
            if !tab.document.contains_source_row(first) || !tab.document.contains_source_row(last) {
                return None;
            }
            Some(tab.id)
        })
    }

    fn apply_context_color_label(
        &mut self,
        label_id: Option<String>,
        selected_text: Option<String>,
        clear_all: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (_, target) = match self.context_color_target(selected_text.as_deref(), cx) {
            Ok(target) => target,
            Err(message) => {
                window.push_notification(message, cx);
                return;
            }
        };
        self.start_color_rule_action(
            target,
            ColorRuleAction::Apply {
                label_id,
                clear_all,
            },
            window,
            cx,
        );
    }

    fn context_mark_label(&self, cx: &App) -> &'static str {
        if self.active_log_region == LogRegion::GlobalResults {
            let selected = self.global_table.read(cx).delegate().selection_snapshot();
            if let Some(targets) = (!selected.is_empty())
                .then(|| self.resolve_global_mark_targets(&selected))
                .flatten()
                && targets.iter().all(|(document_id, rows)| {
                    self.documents
                        .iter()
                        .find(|tab| tab.id == *document_id)
                        .is_some_and(|tab| tab.marked_rows.contains_all(rows))
                })
            {
                crate::tr!("取消标记", "Unmark")
            } else {
                crate::tr!("标记", "Mark")
            }
        } else if self.active_document().is_some_and(|tab| {
            let rows = tab.selected_source_rows_compressed(cx);
            !rows.is_empty() && tab.marked_rows.contains_all(&rows)
        }) {
            crate::tr!("取消标记", "Unmark")
        } else {
            crate::tr!("标记", "Mark")
        }
    }

    fn context_color_label_id(&self, selected_text: Option<&str>, cx: &App) -> Option<String> {
        let selected_text = selected_text
            .map(str::trim)
            .filter(|text| !text.is_empty())?;
        let (tab_ix, _) = self.context_color_target(Some(selected_text), cx).ok()?;
        self.documents[tab_ix]
            .keyword_color_rules
            .iter()
            .find(|rule| {
                rule.enabled && rule.case_sensitive && rule.keyword.as_str() == selected_text
            })
            .and_then(|rule| rule.label_id.clone())
    }

    fn build_log_context_menu(
        menu: PopupMenu,
        workspace: Entity<Self>,
        context: LogContextMenuContext,
        window: &mut Window,
        cx: &mut Context<PopupMenu>,
    ) -> PopupMenu {
        let selected_text =
            (!context.selected_text.trim().is_empty()).then_some(context.selected_text);
        let has_row_selection = match workspace.read(cx).active_log_region {
            LogRegion::GlobalResults => {
                workspace
                    .read(cx)
                    .global_table
                    .read(cx)
                    .delegate()
                    .selected_rows_count()
                    > 0
            }
            _ => workspace
                .read(cx)
                .active_document()
                .is_some_and(|tab| tab.selected_rows_count(cx) > 0),
        };
        if selected_text.is_none() && !has_row_selection {
            if !context.include_results {
                return menu;
            }
            let mut menu = menu.item(
                PopupMenuItem::new(crate::tr!("在新标签页打开", "Open in new tab"))
                    .action(Box::new(OpenSearchResultsInNewTab))
                    .disabled(context.export_disabled),
            );
            if context.include_global_merge {
                menu = menu.item(
                    PopupMenuItem::new(crate::tr!("新标签页合并结果", "Merge results in new tab"))
                        .action(Box::new(MergeSearchResultsInNewTab))
                        .disabled(context.export_disabled),
                );
            }
            return menu.item(
                PopupMenuItem::new(crate::tr!("保存到文件…", "Save to file…"))
                    .action(Box::new(SaveSearchResultsToFile))
                    .disabled(context.export_disabled),
            );
        }
        let copy_text = selected_text.clone();
        let copy = window.listener_for(&workspace, move |this, _, window, cx| {
            if let Some(text) = copy_text.clone() {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
                window.push_notification(crate::tr!("已复制所选文字", "Selected text copied"), cx);
            } else {
                this.copy_selected_line(false, window, cx);
            }
        });
        let mark_label = workspace.read(cx).context_mark_label(cx);
        let labels = workspace.read(cx).color_labels.clone();
        let color_state_known = selected_text.is_some();
        let current_label_id = workspace
            .read(cx)
            .context_color_label_id(selected_text.as_deref(), cx);
        let color_target = selected_text.clone();
        let color_workspace = workspace.clone();
        let mark_workspace = workspace.clone();
        let mut menu = menu
            .item(PopupMenuItem::new(crate::tr!("复制", "Copy")).on_click(copy))
            .submenu(
                crate::tr!("颜色标签", "Color labels"),
                window,
                cx,
                move |menu, window, cx| {
                    let none_target = color_target.clone();
                    let clear_target = color_target.clone();
                    let none_workspace = color_workspace.clone();
                    let clear_workspace = color_workspace.clone();
                    let mut menu = menu.check_side(Side::Right).item(
                        PopupMenuItem::new(crate::tr!("无", "None"))
                            .checked(color_state_known && current_label_id.is_none())
                            .on_click(window.listener_for(
                                &none_workspace,
                                move |this, _, window, cx| {
                                    this.apply_context_color_label(
                                        None,
                                        none_target.clone(),
                                        false,
                                        window,
                                        cx,
                                    );
                                },
                            )),
                    );
                    for label in labels.clone() {
                        let label_id = label.id.clone();
                        let target = color_target.clone();
                        let color_swatch = Icon::empty()
                            .rounded(cx.theme().radius / 2.)
                            .border_1()
                            .border_color(cx.theme().input)
                            .bg(color_with_alpha(label.color, label.alpha));
                        menu = menu.item(
                            PopupMenuItem::new(label.localized_name())
                                .icon(color_swatch)
                                .checked(current_label_id.as_deref() == Some(label_id.as_str()))
                                .on_click(window.listener_for(
                                    &color_workspace,
                                    move |this, _, window, cx| {
                                        this.apply_context_color_label(
                                            Some(label_id.clone()),
                                            target.clone(),
                                            false,
                                            window,
                                            cx,
                                        );
                                    },
                                )),
                        );
                    }
                    menu.separator().item(
                        PopupMenuItem::new(crate::tr!("清除所有颜色", "Clear all colors"))
                            .on_click(window.listener_for(
                                &clear_workspace,
                                move |this, _, window, cx| {
                                    this.apply_context_color_label(
                                        None,
                                        clear_target.clone(),
                                        true,
                                        window,
                                        cx,
                                    );
                                },
                            )),
                    )
                },
            )
            .item(PopupMenuItem::new(mark_label).on_click(window.listener_for(
                &mark_workspace,
                move |this, _, window, cx| {
                    this.toggle_marked_row(&ToggleMarkedRow, window, cx);
                },
            )));
        if context.include_results {
            menu = menu.separator().item(
                PopupMenuItem::new(crate::tr!("在新标签页打开", "Open in new tab"))
                    .action(Box::new(OpenSearchResultsInNewTab))
                    .disabled(context.export_disabled),
            );
            if context.include_global_merge {
                menu = menu.item(
                    PopupMenuItem::new(crate::tr!("新标签页合并结果", "Merge results in new tab"))
                        .action(Box::new(MergeSearchResultsInNewTab))
                        .disabled(context.export_disabled),
                );
            }
            menu = menu.item(
                PopupMenuItem::new(crate::tr!("保存到文件…", "Save to file…"))
                    .action(Box::new(SaveSearchResultsToFile))
                    .disabled(context.export_disabled),
            );
        }
        menu
    }

    fn capture_local_row_viewport_anchor(
        tab: &DocumentTab,
        region: WrappedRegion,
        row_height: Pixels,
        cx: &App,
    ) -> Option<RowViewportAnchor<LogRowKey>> {
        let (table, viewport) = if region == WrappedRegion::Results {
            (&tab.result_table, &tab.result_viewport)
        } else {
            (&tab.log_table, &tab.log_viewport)
        };
        let table_state = table.read(cx);
        let count = table_state.delegate().row_count();
        if count == 0 {
            return None;
        }
        let position =
            viewport.capture_viewport_position(count, table_state.active_log_row(), row_height)?;
        Some(RowViewportAnchor {
            key: table_state.delegate().row_key(position.row_ix)?,
            viewport_y: position.viewport_y,
            fallback_ix: position.row_ix,
        })
    }

    fn capture_local_viewport_anchor(
        tab: &DocumentTab,
        region: WrappedRegion,
        row_height: Pixels,
        cx: &App,
    ) -> Option<ViewportAnchor<LogRowKey>> {
        let anchor = Self::capture_local_row_viewport_anchor(tab, region, row_height, cx)?;
        let viewport = if region == WrappedRegion::Results {
            &tab.result_viewport
        } else {
            &tab.log_viewport
        };
        Some(ViewportAnchor {
            key: anchor.key,
            viewport_y: anchor.viewport_y,
            at_end: viewport.is_at_end(),
            fallback_ix: anchor.fallback_ix,
        })
    }

    fn capture_persisted_local_viewport(
        tab: &DocumentTab,
        region: WrappedRegion,
        row_height: Pixels,
        cx: &App,
    ) -> Option<ViewportBookmark> {
        let (table, viewport) = if region == WrappedRegion::Results {
            (&tab.result_table, &tab.result_viewport)
        } else {
            (&tab.log_table, &tab.log_viewport)
        };
        let table_state = table.read(cx);
        let count = table_state.delegate().row_count();
        if count == 0 {
            return None;
        }
        let position = viewport.capture_viewport_position(count, None, row_height)?;
        let source_row = table_state.delegate().source_row(position.row_ix)?;
        Some(
            ViewportBookmark::new(
                source_row,
                position.viewport_y.as_f32(),
                viewport.horizontal_offset().as_f32(),
                viewport.is_at_end(),
            )
            .with_anchor_row_height(
                viewport
                    .effective_row_height(position.row_ix, row_height)
                    .as_f32(),
            ),
        )
    }

    fn restore_persisted_local_viewport(
        tab: &DocumentTab,
        region: WrappedRegion,
        bookmark: Option<ViewportBookmark>,
        row_height: Pixels,
        cx: &mut App,
    ) {
        let Some(bookmark) = bookmark else {
            return;
        };
        let (table, viewport) = if region == WrappedRegion::Results {
            (&tab.result_table, &tab.result_viewport)
        } else {
            (&tab.log_table, &tab.log_viewport)
        };
        let key = LogRowKey::Row {
            document_id: tab.id,
            source_row: bookmark.anchor_source_row,
        };
        let row_count = table.read(cx).delegate().row_count();
        let restored_ix = table.read(cx).delegate().row_ix_for_key(key);
        let fallback_ix = restored_ix.unwrap_or_default();
        let viewport_y = px(bookmark.anchor_viewport_y());
        if viewport.is_wrapped() {
            if let Some(restored_ix) = restored_ix {
                // A negative viewport position means the visible frame begins inside a wrapped
                // logical row. Prime its persisted height before resolving the sparse scroll
                // offset; otherwise the row is initially treated as one line and can be skipped
                // before its current layout reports the real height. The offset-derived minimum
                // keeps pre-height bookmarks compatible.
                let minimum_visible_height = (-viewport_y + row_height).max(row_height);
                let anchor_height = bookmark
                    .anchor_row_height()
                    .map(px)
                    .unwrap_or(row_height)
                    .max(row_height)
                    .max(minimum_visible_height);
                viewport.prime_wrapped_measured_heights(
                    row_count,
                    row_height,
                    [(restored_ix, anchor_height)],
                );
            } else {
                viewport.wrapped_sizes(row_count, row_height);
            }
        }
        Self::restore_local_viewport_anchor(
            tab,
            region,
            Some(ViewportAnchor {
                key,
                viewport_y,
                at_end: bookmark.at_end,
                fallback_ix,
            }),
            row_height,
            cx,
        );
        viewport.set_horizontal_offset(px(bookmark.horizontal_offset()));
    }

    fn capture_global_row_viewport_anchor(
        &self,
        row_height: Pixels,
        cx: &App,
    ) -> Option<RowViewportAnchor<LogRowKey>> {
        let table = self.global_table.read(cx);
        let count = table.delegate().rows_len();
        if count == 0 {
            return None;
        }
        let position = self.global_viewport.capture_viewport_position(
            count,
            table.active_log_row(),
            row_height,
        )?;
        Some(RowViewportAnchor {
            key: table.delegate().row_key(position.row_ix)?,
            viewport_y: position.viewport_y,
            fallback_ix: position.row_ix,
        })
    }

    fn capture_global_viewport_anchor(
        &self,
        row_height: Pixels,
        cx: &App,
    ) -> Option<ViewportAnchor<LogRowKey>> {
        let anchor = self.capture_global_row_viewport_anchor(row_height, cx)?;
        Some(ViewportAnchor {
            key: anchor.key,
            viewport_y: anchor.viewport_y,
            at_end: self.global_viewport.is_at_end(),
            fallback_ix: anchor.fallback_ix,
        })
    }

    fn position_local_row_viewport_anchor(
        tab: &DocumentTab,
        region: WrappedRegion,
        anchor: Option<RowViewportAnchor<LogRowKey>>,
        row_height: Pixels,
        cx: &mut App,
    ) {
        let Some(anchor) = anchor else {
            return;
        };
        let (table, viewport) = if region == WrappedRegion::Results {
            (&tab.result_table, &tab.result_viewport)
        } else {
            (&tab.log_table, &tab.log_viewport)
        };
        let row_ix = {
            let table = table.read(cx);
            let delegate = table.delegate();
            let Some(row_ix) = delegate.nearest_row_ix_for_key(anchor.key).or_else(|| {
                let row_count = delegate.row_count();
                (row_count > 0).then(|| anchor.fallback_ix.min(row_count - 1))
            }) else {
                return;
            };
            row_ix
        };
        viewport.restore_viewport(row_ix, anchor.viewport_y, false, row_height);
    }

    fn restore_local_viewport_anchor(
        tab: &DocumentTab,
        region: WrappedRegion,
        anchor: Option<ViewportAnchor<LogRowKey>>,
        row_height: Pixels,
        cx: &mut App,
    ) {
        let Some(anchor) = anchor else {
            return;
        };
        let viewport = if region == WrappedRegion::Results {
            &tab.result_viewport
        } else {
            &tab.log_viewport
        };
        if anchor.at_end {
            viewport.scroll_to_end();
            return;
        }
        Self::position_local_row_viewport_anchor(
            tab,
            region,
            Some(RowViewportAnchor {
                key: anchor.key,
                viewport_y: anchor.viewport_y,
                fallback_ix: anchor.fallback_ix,
            }),
            row_height,
            cx,
        );
    }

    fn position_global_row_viewport_anchor(
        &self,
        anchor: Option<RowViewportAnchor<LogRowKey>>,
        row_height: Pixels,
        cx: &mut App,
    ) {
        let Some(anchor) = anchor else {
            return;
        };
        let row_ix = {
            let table = self.global_table.read(cx);
            let delegate = table.delegate();
            let Some(row_ix) = delegate
                .nearest_row_ix_for_key(anchor.key)
                .or_else(|| delegate.closest_match_row(anchor.fallback_ix))
                .or_else(|| {
                    let row_count = delegate.rows_len();
                    (row_count > 0).then(|| anchor.fallback_ix.min(row_count - 1))
                })
            else {
                return;
            };
            row_ix
        };
        self.global_viewport
            .restore_viewport(row_ix, anchor.viewport_y, false, row_height);
    }

    fn restore_global_viewport_anchor(
        &self,
        anchor: Option<ViewportAnchor<LogRowKey>>,
        row_height: Pixels,
        cx: &mut App,
    ) {
        let Some(anchor) = anchor else {
            return;
        };
        if anchor.at_end {
            self.global_viewport.scroll_to_end();
            return;
        }
        self.position_global_row_viewport_anchor(
            Some(RowViewportAnchor {
                key: anchor.key,
                viewport_y: anchor.viewport_y,
                fallback_ix: anchor.fallback_ix,
            }),
            row_height,
            cx,
        );
    }

    fn extend_log_selection(
        &mut self,
        direction: i32,
        page: bool,
        edge: Option<bool>,
        cx: &mut Context<Self>,
    ) {
        if self.active_log_region == LogRegion::GlobalResults {
            let table = self.global_table.clone();
            let state = table.read(cx);
            let count = state.delegate().rows_len();
            if count == 0 {
                return;
            }
            let current = state.active_log_row().unwrap_or_default();
            let step = if page {
                state.visible_range().rows().len().max(1)
            } else {
                1
            };
            let candidate = match edge {
                Some(false) => 0,
                Some(true) => count - 1,
                None if direction < 0 => current.saturating_sub(step),
                None => current.saturating_add(step).min(count - 1),
            };
            let Some(target) = state
                .delegate()
                .nearest_match_row(candidate, direction >= 0)
            else {
                return;
            };
            table.update(cx, |table, cx| {
                table.delegate().extend_keyboard_selection(target);
                table.set_active_log_row(target, cx);
                table.scroll_to_row(target, cx);
            });
            cx.stop_propagation();
            cx.notify();
            return;
        }
        let Some(tab) = self.active_document() else {
            return;
        };
        let table = match self.active_log_region {
            LogRegion::CurrentResults if tab.results_visible => tab.result_table.clone(),
            _ => tab.log_table.clone(),
        };
        let state = table.read(cx);
        let count = state.delegate().row_count();
        if count == 0 {
            return;
        }
        let current = state.active_log_row().unwrap_or_default();
        let step = if page {
            state.visible_range().rows().len().max(1)
        } else {
            1
        };
        let target = match edge {
            Some(false) => 0,
            Some(true) => count - 1,
            None if direction < 0 => current.saturating_sub(step),
            None => current.saturating_add(step).min(count - 1),
        };
        table.update(cx, |table, cx| {
            table.delegate().extend_keyboard_selection(target);
            table.set_active_log_row(target, cx);
            table.scroll_to_row(target, cx);
        });
        cx.stop_propagation();
        cx.notify();
    }

    fn extend_selection_up(
        &mut self,
        _: &ExtendSelectionUp,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.extend_log_selection(-1, false, None, cx);
    }

    fn extend_selection_down(
        &mut self,
        _: &ExtendSelectionDown,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.extend_log_selection(1, false, None, cx);
    }

    fn extend_selection_page_up(
        &mut self,
        _: &ExtendSelectionPageUp,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.extend_log_selection(-1, true, None, cx);
    }

    fn extend_selection_page_down(
        &mut self,
        _: &ExtendSelectionPageDown,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.extend_log_selection(1, true, None, cx);
    }

    fn extend_selection_first(
        &mut self,
        _: &ExtendSelectionFirst,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.extend_log_selection(-1, false, Some(false), cx);
    }

    fn extend_selection_last(
        &mut self,
        _: &ExtendSelectionLast,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.extend_log_selection(1, false, Some(true), cx);
    }

    fn prepare_wrapped_global_context(
        &mut self,
        row_ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.global_results_focus_handle.focus(window, cx);
        self.remember_user_log_region(LogRegion::GlobalResults);
        self.global_table.update(cx, |table, cx| {
            if matches!(
                table.delegate().row(row_ix),
                Some(GlobalSearchRow::Match { .. })
            ) {
                table.delegate().prepare_context_selection(row_ix);
            }
            table.set_active_log_row(row_ix, cx);
        });
        cx.notify();
    }

    fn prepare_wrapped_global_group_context(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.global_results_focus_handle.focus(window, cx);
        self.remember_user_log_region(LogRegion::GlobalResults);
        self.global_table.update(cx, |table, cx| {
            table.delegate().clear_row_selection();
            table.clear_selection(cx);
        });
        cx.notify();
    }

    fn render_wrapped_global_rows(
        &mut self,
        visible_range: Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let _performance_scope =
            crate::ui_performance::scope("Workspace::render_wrapped_global_rows");
        self.schedule_global_visible_lines(visible_range.clone(), cx);
        let (
            font_size,
            font_family,
            line_number_width,
            line_number_text_color,
            line_number_background_color,
            show_line_number_row_separators,
            show_row_separators,
        ) = {
            let table = self.global_table.read(cx);
            (
                table.delegate().log_font_size(),
                table.delegate().resolved_font_family(cx),
                table.delegate().line_number_width(),
                table.delegate().line_number_text_color(cx),
                table.delegate().line_number_background_color(cx),
                table.delegate().show_line_number_row_separators(),
                table.delegate().show_row_separators(),
            )
        };
        let base_height = self.log_row_height();
        let marker_width = line_marker_column_width();
        let fixed_columns_width = marker_width + px(line_number_width as f32);
        let workspace = cx.weak_entity();
        let suppress_text_selection = self.row_drag_selection.is_some_and(|drag| {
            drag.region == WrappedRegion::GlobalResults && drag.mode == RowDragMode::Lines
        });
        self.global_viewport
            .retain_wrapped_visible_rows(&visible_range);
        let rendered_row_bounds = self.global_viewport.wrapped_row_bounds();

        visible_range
            .filter_map(|row_ix| {
                let row = self.global_table.read(cx).delegate().wrapped_row(row_ix)?;
                let row_bounds = rendered_row_bounds.clone();
                match row {
                    WrappedGlobalRow::Group {
                        document_id,
                        title,
                        path,
                        result_count,
                        truncated,
                        failure,
                        collapsed,
                    } => Some(
                        div()
                            .id(("wrapped-global-group", document_id))
                            .on_prepaint(move |bounds, _, _| {
                                row_bounds.borrow_mut().insert(row_ix, bounds);
                            })
                            .relative()
                            .w_full()
                            .h(base_height)
                            .flex_none()
                            .overflow_hidden()
                            .flex()
                            .items_center()
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                                    this.select_wrapped_global_row(row_ix, event, window, cx);
                                }),
                            )
                            .on_mouse_down(
                                MouseButton::Right,
                                cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                                    this.prepare_wrapped_global_group_context(window, cx);
                                }),
                            )
                            .child(
                                GlobalSearchGroupHeader::new(
                                    title,
                                    path,
                                    result_count,
                                    font_family.clone(),
                                    font_size,
                                )
                                .truncated(truncated)
                                .failure(failure)
                                .collapsed(collapsed),
                            )
                            .into_any_element(),
                    ),
                    WrappedGlobalRow::Match {
                        document_id,
                        source_row,
                        text,
                        selected,
                        marked,
                        matched,
                        highlight_severity,
                        source_unavailable,
                        highlights,
                    } => {
                        let selected_above = row_ix > 0
                            && self
                                .global_table
                                .read(cx)
                                .delegate()
                                .is_row_selected(row_ix - 1);
                        let selected_below = row_ix + 1
                            < self.global_table.read(cx).delegate().rows_len()
                            && self
                                .global_table
                                .read(cx)
                                .delegate()
                                .is_row_selected(row_ix + 1);
                        let selection = self.global_viewport.wrapped_selection(
                            (document_id, source_row),
                            &text,
                            window,
                            cx,
                        );
                        let styled_text = StyledText::new(text.display().clone())
                            .with_highlights(Self::highlight_styles(&highlights, cx));
                        let severity = (!source_unavailable && highlight_severity)
                            .then(|| text.severity())
                            .flatten()
                            .map(|severity| severity_style(severity, cx));
                        let measure_workspace = workspace.clone();
                        let row_bounds = rendered_row_bounds.clone();
                        let selectable = SelectableLogText::new(
                            selection,
                            document_id.rotate_left(32) ^ source_row as u64,
                            text,
                            styled_text,
                            ui_theme::text_selection_highlight(cx),
                        )
                        .word_boundary_characters(
                            self.app_settings.word_boundary_characters.clone(),
                        )
                        .suppress_selection(suppress_text_selection)
                        .on_measure(move |height, _, cx| {
                            _ = measure_workspace.update(cx, |this, cx| {
                                this.update_wrapped_height(
                                    document_id,
                                    WrappedRegion::GlobalResults,
                                    row_ix,
                                    height,
                                    base_height,
                                    cx,
                                );
                            });
                        });
                        Some(
                            div()
                                .id(format!("wrapped-global-result-{document_id}-{source_row}"))
                                .on_prepaint(move |bounds, _, _| {
                                    row_bounds.borrow_mut().insert(row_ix, bounds);
                                })
                                .relative()
                                .w_full()
                                .min_h(base_height)
                                .flex()
                                .items_start()
                                .when_some(severity, |row, style| {
                                    row.bg(style.background)
                                        .child(severity_accent_overlay(style.accent))
                                })
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                                        this.select_wrapped_global_row(row_ix, event, window, cx);
                                    }),
                                )
                                .on_mouse_down(
                                    MouseButton::Right,
                                    cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                                        this.prepare_wrapped_global_context(row_ix, window, cx);
                                    }),
                                )
                                .child(
                                    h_flex()
                                        .w(marker_width)
                                        .self_stretch()
                                        .flex_none()
                                        .justify_center()
                                        .child(line_marker(marked, matched, cx)),
                                )
                                .child(
                                    log_line_number_cell(
                                        source_row,
                                        font_size,
                                        base_height,
                                        line_number_text_color,
                                        line_number_background_color,
                                        show_line_number_row_separators,
                                        cx,
                                    )
                                    .w(px(line_number_width as f32))
                                    .self_stretch()
                                    .flex_none(),
                                )
                                .child(
                                    div()
                                        .relative()
                                        .min_w_0()
                                        .flex_1()
                                        .overflow_hidden()
                                        .whitespace_normal()
                                        .px(log_cell_horizontal_padding(cx))
                                        .text_size(px(font_size as f32))
                                        .line_height(base_height)
                                        .font_family(font_family.clone())
                                        .when(source_unavailable, |cell| {
                                            cell.text_color(cx.theme().danger)
                                        })
                                        .when(selected, |cell| {
                                            cell.bg(log_row_selection_color(cx)).child(
                                                log_row_selection_overlay(
                                                    !selected_above,
                                                    !selected_below,
                                                    cx,
                                                ),
                                            )
                                        })
                                        .when(show_row_separators && !selected, |cell| {
                                            cell.child(log_row_separator_overlay(false, cx))
                                        })
                                        .child(selectable),
                                )
                                .child(log_fixed_column_divider_overlay(fixed_columns_width, cx))
                                .into_any_element(),
                        )
                    }
                }
            })
            .collect()
    }

    fn render_wrapped_global_table(
        &self,
        surface: Entity<LogRegionSurface>,
        workspace: WeakEntity<Workspace>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let _performance_scope =
            crate::ui_performance::scope("Workspace::render_wrapped_global_table");
        let delegate = self.global_table.read(cx).delegate();
        let count = delegate.rows_len();
        let base_height = self.log_row_height();
        if self.global_viewport.wrapped_base_height() != base_height {
            self.global_viewport
                .ensure_wrapped_measurement_anchor(self.global_table.read(cx).active_log_row());
        }
        let sizes = self.global_viewport.wrapped_sizes(count, base_height);
        let scroll_handle = self.global_viewport.wrapped_scroll_handle();
        let list_scroll = scroll_handle.clone();
        let logical_scroll = self
            .global_viewport
            .wrapped_logical_scroll_handle(count, base_height);
        let scrollbar_background = *cx.theme().tokens.table;

        v_flex()
            .size_full()
            .min_h_0()
            .bg(cx.theme().tokens.table)
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .key_context("DataTable")
                    .child(
                        sparse_v_virtual_list(
                            surface,
                            "wrapped-global-results",
                            sizes,
                            move |_, range, window, cx| {
                                workspace
                                    .update(cx, |workspace, cx| {
                                        workspace.render_wrapped_global_rows(range, window, cx)
                                    })
                                    .unwrap_or_default()
                            },
                        )
                        .track_scroll(&list_scroll)
                        .size_full(),
                    )
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .right_0()
                            .bottom_0()
                            .w(Scrollbar::width())
                            .bg(scrollbar_background)
                            .child(
                                persistent_log_scrollbar(
                                    Scrollbar::vertical(&logical_scroll)
                                        .id("wrapped-global-results-vertical-scrollbar")
                                        .viewport_from_layout(),
                                    scrollbar_background,
                                )
                                .max_fps(60),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn render_headerless_data_table<D>(
        table: &Entity<TableState<D>>,
        vertical_scroll_handle: AtomicUniformScrollHandle,
        row_height: Pixels,
        cx: &mut Context<Self>,
    ) -> AnyElement
    where
        D: TableDelegate,
    {
        let _performance_scope =
            crate::ui_performance::scope("Workspace::render_headerless_data_table");
        let (horizontal_scroll_handle, horizontal_content_width) = {
            let table = table.read(cx);
            let delegate = table.delegate();
            let horizontal_content_width = (0..delegate.columns_count(cx))
                .map(|col_ix| delegate.column(col_ix, cx).width)
                .fold(px(0.), |width, column_width| width + column_width);
            (
                table.horizontal_scroll_handle.clone(),
                horizontal_content_width,
            )
        };
        let table_id = table.entity_id();
        let scrollbar_background = *cx.theme().tokens.table;
        let scrollbar_width = Scrollbar::width();
        // The horizontal track extends beneath the vertical scrollbar gutter. Model the
        // same extra width as content so its maximum offset still matches the table viewport.
        let horizontal_scrollbar_content_width = horizontal_content_width + scrollbar_width;

        let element = v_flex()
            .size_full()
            .min_w_0()
            .min_h_0()
            .child(
                h_flex()
                    .w_full()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .child(
                        div()
                            .relative()
                            .h_full()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .child(crate::ui_performance::element(
                                "LogDataTable::request_layout",
                                "LogDataTable::prepaint",
                                "LogDataTable::paint",
                                DataTable::new(table)
                                    .with_size(row_height)
                                    .bordered(false)
                                    .scrollbar_visible(false, false),
                            )),
                    )
                    .child(
                        div()
                            .relative()
                            .h_full()
                            .w(scrollbar_width)
                            .flex_none()
                            .bg(scrollbar_background)
                            .child(
                                persistent_log_scrollbar(
                                    Scrollbar::vertical(&vertical_scroll_handle)
                                        .id(("log-vertical-scrollbar", table_id))
                                        .viewport_from_layout(),
                                    scrollbar_background,
                                )
                                .max_fps(60),
                            ),
                    ),
            )
            .child(
                div()
                    .relative()
                    .w_full()
                    .h(scrollbar_width)
                    .flex_none()
                    .bg(scrollbar_background)
                    .child(
                        persistent_log_scrollbar(
                            Scrollbar::horizontal(&horizontal_scroll_handle)
                                .id(("log-horizontal-scrollbar", table_id))
                                .scroll_size(size(
                                    horizontal_scrollbar_content_width,
                                    scrollbar_width,
                                ))
                                .viewport_from_layout(),
                            scrollbar_background,
                        )
                        .max_fps(60),
                    ),
            );
        crate::ui_performance::element(
            "HeaderlessLogTable::request_layout",
            "HeaderlessLogTable::prepaint",
            "HeaderlessLogTable::paint",
            element,
        )
        .into_any_element()
    }

    fn render_log_region_surface(
        &mut self,
        document_id: u64,
        region: WrappedRegion,
        surface: Entity<LogRegionSurface>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let _performance_scope =
            crate::ui_performance::scope("Workspace::render_log_region_surface");
        let row_height = self.log_row_height();
        if region == WrappedRegion::GlobalResults {
            let key = (0, WrappedRegion::GlobalResults);
            let target = if let Some(offset) = self.global_viewport.take_pending_scrollbar_offset()
            {
                self.pending_log_scroll_frames.clear(key);
                Some(LogScrollFrameTarget::Scrollbar(offset))
            } else {
                self.pending_log_scroll_frames.take(key)
            };
            if let Some(target) = target {
                self.prepare_global_scroll_frame(target, row_height, window, cx);
            }
            if self.global_viewport.is_wrapped() {
                return self.render_wrapped_global_table(surface, cx.weak_entity(), cx);
            }
            return Self::render_headerless_data_table(
                &self.global_table,
                self.global_viewport.atomic_fixed_scroll_handle(),
                row_height,
                cx,
            );
        }
        let Some(tab_ix) = self.documents.iter().position(|tab| tab.id == document_id) else {
            return div().into_any_element();
        };
        let key = (document_id, region);
        let pending_offset = if region == WrappedRegion::Results {
            self.documents[tab_ix]
                .result_viewport
                .take_pending_scrollbar_offset()
        } else {
            self.documents[tab_ix]
                .log_viewport
                .take_pending_scrollbar_offset()
        };
        let target = if let Some(offset) = pending_offset {
            self.pending_log_scroll_frames.clear(key);
            Some(LogScrollFrameTarget::Scrollbar(offset))
        } else {
            self.pending_log_scroll_frames.take(key)
        };
        if let Some(target) = target {
            self.prepare_local_scroll_frame(document_id, region, target, row_height, window, cx);
        }
        let tab = &self.documents[tab_ix];
        let table = if region == WrappedRegion::Results {
            &tab.result_table
        } else {
            &tab.log_table
        };
        let viewport = if region == WrappedRegion::Results {
            &tab.result_viewport
        } else {
            &tab.log_viewport
        };
        if viewport.is_wrapped() {
            self.render_wrapped_log_table(document_id, region, surface, cx.weak_entity(), cx)
        } else {
            Self::render_headerless_data_table(
                table,
                viewport.atomic_fixed_scroll_handle(),
                row_height,
                cx,
            )
        }
    }

    fn render_new_tab_workspace(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let _performance_scope =
            crate::ui_performance::scope("Workspace::render_new_tab_workspace");
        let opening = self.open_task.is_some();
        div()
            .id("empty-workspace-scroll")
            .size_full()
            .min_h_0()
            .overflow_y_scroll()
            .child(
                v_flex()
                    .min_h_full()
                    .w_full()
                    .items_center()
                    .justify_center()
                    .px_10()
                    .py_8()
                    .child(
                        v_flex()
                            .w_full()
                            .max_w(rems(76.))
                            .gap_5()
                            .child(
                                v_flex()
                                    .w_full()
                                    .gap_5()
                                    .pb_4()
                                    .child(
                                        v_flex()
                                            .gap_2()
                                            .child(
                                                div().text_size(rems(1.75)).child(crate::tr!("开始查看日志", "Start viewing logs")),
                                            )
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .text_color(cx.theme().muted_foreground)
                                                    .child(
                                                        crate::tr!("打开日志文件，或从最近查看过的文件继续。", "Open a log file or continue from a recently viewed file."),
                                                    ),
                                            ),
                                    )
                                    .child(
                                        Button::new("empty-open-files")
                                            .primary()
                                            .w(rems(14.))
                                            .h(rems(3.))
                                            .max_w_full()
                                            .icon(IconName::FolderOpen)
                                            .label(crate::tr!("打开日志文件", "Open log file"))
                                            .rounded(cx.theme().radius_lg * 2.)
                                            .shadow_lg()
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.open_files(&OpenFiles, window, cx);
                                            })),
                                    ),
                            )
                            .when(
                                !self.history_loading && !self.pinned_files.is_empty(),
                                |this| this.child(self.render_pinned_files(opening, cx)),
                            )
                            .when(
                                !self.history_loading && !self.last_workspace_files.is_empty(),
                                |this| this.child(self.render_last_workspace_files(opening, cx)),
                            )
                            .child(self.render_recent_files(opening, cx)),
                    ),
            )
    }

    fn render_document_workspace(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let _performance_scope =
            crate::ui_performance::scope("Workspace::render_document_workspace");
        let tab = self.active_document().expect("active document must exist");
        let results_visible = match self.global_search.scope {
            SearchScope::CurrentFile => tab.results_visible,
            SearchScope::AllOpenFiles | SearchScope::Directory => {
                self.global_search.results_visible
            }
        };
        let result_menu_busy = self.result_export_task.is_some();
        let local_result_menu_disabled =
            tab.result_row_count(cx) == 0 || result_menu_busy || self.open_task.is_some();
        let global_result_menu_disabled = self.global_table.read(cx).delegate().results_count()
            == 0
            || result_menu_busy
            || self.open_task.is_some();
        let result_drag_workspace = cx.entity();
        let global_drag_workspace = cx.entity();
        let log_drag_workspace = cx.entity();
        let result_wheel_workspace = result_drag_workspace.clone();
        let global_wheel_workspace = global_drag_workspace.clone();
        let log_wheel_workspace = log_drag_workspace.clone();
        let local_result_context_workspace = cx.entity();
        let global_result_context_workspace = cx.entity();
        let log_context_workspace = cx.entity();
        let document_id = tab.id;
        let marker_width = line_marker_column_width();
        let local_line_number_width = if tab.show_line_numbers {
            px(tab.log_table.read(cx).delegate().line_number_width() as f32)
        } else {
            px(0.)
        };
        let global_line_number_width =
            px(self.global_table.read(cx).delegate().line_number_width() as f32);
        let result_content = match self.global_search.scope {
            SearchScope::CurrentFile => v_flex()
                .w_full()
                .flex_1()
                .min_h_0()
                .border_t_1()
                .border_color(cx.theme().border)
                .child(
                    div()
                        .relative()
                        .flex_1()
                        .min_h_0()
                        .key_context(LOG_TABLE_CONTEXT)
                        .track_focus(&tab.result_focus_handle)
                        .tab_index(0)
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _: &MouseDownEvent, window, cx| {
                                if let Some(tab) = this.active_document() {
                                    tab.result_focus_handle.focus(window, cx);
                                }
                                this.remember_user_log_region(LogRegion::CurrentResults);
                            }),
                        )
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(|this, _: &MouseDownEvent, window, cx| {
                                if let Some(tab) = this.active_document() {
                                    tab.result_focus_handle.focus(window, cx);
                                }
                                this.remember_user_log_region(LogRegion::CurrentResults);
                            }),
                        )
                        .on_prepaint(move |bounds, window, cx| {
                            result_drag_workspace.update(cx, |workspace, cx| {
                                workspace
                                    .row_drag_bounds
                                    .insert((document_id, WrappedRegion::Results), bounds);
                                workspace.update_wrapped_layout(
                                    document_id,
                                    WrappedRegion::Results,
                                    (bounds.size.width - marker_width - local_line_number_width)
                                        .max(px(0.)),
                                    bounds.size.height,
                                    window,
                                    cx,
                                );
                            });
                        })
                        .on_mouse_move(cx.listener(move |this, event, window, cx| {
                            this.handle_row_drag_move(
                                document_id,
                                WrappedRegion::Results,
                                event,
                                window,
                                cx,
                            );
                        }))
                        .child(Self::capture_log_wheel(
                            result_wheel_workspace,
                            document_id,
                            WrappedRegion::Results,
                        ))
                        .child(tab.result_surface.clone())
                        .when(
                            self.quick_find.open
                                && self.quick_find.target
                                    == Some(QuickFindTarget::Results(document_id)),
                            |region| region.child(self.render_quick_find_bar(cx)),
                        )
                        .context_menu(move |menu, window, cx| {
                            Self::build_log_context_menu(
                                menu,
                                local_result_context_workspace.clone(),
                                LogContextMenuContext {
                                    selected_text: TextSelection::selected_text(window, cx),
                                    include_results: true,
                                    include_global_merge: false,
                                    export_disabled: local_result_menu_disabled,
                                },
                                window,
                                cx,
                            )
                        })
                        .text_selection_scope(tab.result_text_selection_scope),
                )
                .into_any_element(),
            SearchScope::AllOpenFiles | SearchScope::Directory => v_flex()
                .w_full()
                .flex_1()
                .min_h_0()
                .border_t_1()
                .border_color(cx.theme().border)
                .child(
                    div()
                        .relative()
                        .flex_1()
                        .min_h_0()
                        .key_context(LOG_TABLE_CONTEXT)
                        .track_focus(&self.global_results_focus_handle)
                        .tab_index(0)
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _: &MouseDownEvent, window, cx| {
                                this.global_results_focus_handle.focus(window, cx);
                                this.remember_user_log_region(LogRegion::GlobalResults);
                            }),
                        )
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(|this, _: &MouseDownEvent, window, cx| {
                                this.global_results_focus_handle.focus(window, cx);
                                this.remember_user_log_region(LogRegion::GlobalResults);
                            }),
                        )
                        .on_prepaint(move |bounds, window, cx| {
                            global_drag_workspace.update(cx, |workspace, cx| {
                                workspace
                                    .row_drag_bounds
                                    .insert((0, WrappedRegion::GlobalResults), bounds);
                                workspace.update_wrapped_layout(
                                    document_id,
                                    WrappedRegion::GlobalResults,
                                    (bounds.size.width - marker_width - global_line_number_width)
                                        .max(px(0.)),
                                    bounds.size.height,
                                    window,
                                    cx,
                                );
                            });
                        })
                        .on_mouse_move(cx.listener(move |this, event, window, cx| {
                            this.handle_row_drag_move(
                                document_id,
                                WrappedRegion::GlobalResults,
                                event,
                                window,
                                cx,
                            );
                        }))
                        .child(Self::capture_log_wheel(
                            global_wheel_workspace,
                            document_id,
                            WrappedRegion::GlobalResults,
                        ))
                        .child(self.global_surface.clone())
                        .when(
                            self.quick_find.open
                                && self.quick_find.target == Some(QuickFindTarget::GlobalResults),
                            |region| region.child(self.render_quick_find_bar(cx)),
                        )
                        .context_menu(move |menu, window, cx| {
                            Self::build_log_context_menu(
                                menu,
                                global_result_context_workspace.clone(),
                                LogContextMenuContext {
                                    selected_text: TextSelection::selected_text(window, cx),
                                    include_results: true,
                                    include_global_merge: true,
                                    export_disabled: global_result_menu_disabled,
                                },
                                window,
                                cx,
                            )
                        })
                        .text_selection_scope(self.global_text_selection_scope),
                )
                .into_any_element(),
        };
        let search_panel = v_flex()
            .id("search-panel")
            .size_full()
            .min_h_0()
            .bg(cx.theme().background)
            .child(self.render_search_bar(window, cx))
            .when(results_visible, |panel| panel.child(result_content))
            .when(!results_visible, |panel| {
                panel.child(
                    v_flex()
                        .flex_1()
                        .min_h_0()
                        .items_center()
                        .justify_center()
                        .gap_1()
                        .border_t_1()
                        .border_color(cx.theme().border)
                        .text_color(cx.theme().muted_foreground)
                        .child(Icon::new(IconName::Search).small())
                        .child(div().text_sm().child(crate::tr!(
                            "输入关键词后开始搜索",
                            "Enter keywords to start searching"
                        ))),
                )
            });
        let search_panel_height = self
            .search_panel_height
            .unwrap_or(cx.theme().font_size * 16.);
        let scrollbar_overlap = Scrollbar::width() / 3.;
        let resize_bounds = self.search_panel_resize_bounds.clone();
        let search_panel_resize_hit_area = div()
            .id("search-panel-resize-hit-area")
            .occlude()
            .absolute()
            .top(-scrollbar_overlap)
            .left_0()
            .w_full()
            .h(scrollbar_overlap + SEARCH_BAR_VERTICAL_INSET)
            .cursor_row_resize()
            .on_prepaint(move |bounds, _, _| resize_bounds.set(Some(bounds)));
        let search_panel_resize_event_layer = self.render_search_panel_resize_event_layer(cx);
        let search_panel_workspace = cx.weak_entity();
        div()
            .id("document-split")
            .relative()
            .size_full()
            .min_h_0()
            .child(
                v_resizable("log-and-search-results")
                    .with_state(&self.search_panel_state)
                    .on_resize(move |state, window, cx| {
                        let height = state.read(cx).sizes().get(1).copied();
                        let Some(height) = height else {
                            return;
                        };
                        _ = search_panel_workspace.update(cx, |workspace, cx| {
                            workspace.remember_search_panel_height(height, window, cx);
                        });
                    })
                    .child(
                        resizable_panel().child(
                            div()
                                .relative()
                                .size_full()
                                .min_h_0()
                                .key_context(LOG_TABLE_CONTEXT)
                                .track_focus(&tab.log_focus_handle)
                                .tab_index(0)
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, _: &MouseDownEvent, window, cx| {
                                        if let Some(tab) = this.active_document() {
                                            tab.log_focus_handle.focus(window, cx);
                                        }
                                        this.remember_user_log_region(LogRegion::Body);
                                    }),
                                )
                                .on_mouse_down(
                                    MouseButton::Right,
                                    cx.listener(|this, _: &MouseDownEvent, window, cx| {
                                        if let Some(tab) = this.active_document() {
                                            tab.log_focus_handle.focus(window, cx);
                                        }
                                        this.remember_user_log_region(LogRegion::Body);
                                    }),
                                )
                                .on_prepaint(move |bounds, window, cx| {
                                    log_drag_workspace.update(cx, |workspace, cx| {
                                        workspace
                                            .row_drag_bounds
                                            .insert((document_id, WrappedRegion::Log), bounds);
                                        workspace.update_wrapped_layout(
                                            document_id,
                                            WrappedRegion::Log,
                                            (bounds.size.width
                                                - marker_width
                                                - local_line_number_width)
                                                .max(px(0.)),
                                            bounds.size.height,
                                            window,
                                            cx,
                                        );
                                    });
                                })
                                .on_mouse_move(cx.listener(move |this, event, window, cx| {
                                    this.handle_row_drag_move(
                                        document_id,
                                        WrappedRegion::Log,
                                        event,
                                        window,
                                        cx,
                                    );
                                }))
                                .child(Self::capture_log_wheel(
                                    log_wheel_workspace,
                                    document_id,
                                    WrappedRegion::Log,
                                ))
                                .child(tab.log_surface.clone())
                                .when(
                                    self.quick_find.open
                                        && self.quick_find.target
                                            == Some(QuickFindTarget::Log(document_id)),
                                    |region| region.child(self.render_quick_find_bar(cx)),
                                )
                                .context_menu(move |menu, window, cx| {
                                    Self::build_log_context_menu(
                                        menu,
                                        log_context_workspace.clone(),
                                        LogContextMenuContext {
                                            selected_text: TextSelection::selected_text(window, cx),
                                            include_results: false,
                                            include_global_merge: false,
                                            export_disabled: false,
                                        },
                                        window,
                                        cx,
                                    )
                                })
                                .text_selection_scope(tab.log_text_selection_scope),
                        ),
                    )
                    .child(
                        resizable_panel()
                            .size(search_panel_height)
                            // 搜索面板折叠高 50px，展开下限为 197px（50px 工具栏 + 147px 结果区）。
                            .size_range(px(197.)..Pixels::MAX)
                            .child(search_panel)
                            .child(search_panel_resize_hit_area),
                    ),
            )
            .child(search_panel_resize_event_layer)
    }

    fn render_status_bar(&self, cx: &App) -> impl IntoElement {
        let _performance_scope = crate::ui_performance::scope("Workspace::render_status_bar");
        let marked_count = self
            .active_document()
            .map_or(0, |tab| tab.marked_rows.len());
        let selected_count = if self.active_log_region == LogRegion::GlobalResults
            && self.global_search.results_visible
        {
            self.global_table.read(cx).delegate().selected_rows_count()
        } else {
            self.active_document()
                .map_or(0, |tab| tab.selected_rows_count(cx))
        };
        let right = self
            .selected_source_row
            .map(|row| {
                if selected_count > 1 {
                    crate::tr_args!(
                        "第 {} 行 · 已选 {} 行 · {} 个标记",
                        "Line {} · {} lines selected · {} marks",
                        row + 1,
                        selected_count,
                        marked_count,
                    )
                } else {
                    crate::tr_args!(
                        "第 {} 行 · {} 个标记",
                        "Line {} · {} marks",
                        row + 1,
                        marked_count
                    )
                }
            })
            .unwrap_or_else(|| format!("core {}", crate::build_info::VERSION));

        StatusBar::new()
            .h(px(30.))
            .px(px(12.))
            .gap(px(8.))
            .text_size(px(11.))
            .bg(ui_theme::footer_material(&ui_theme::palette(cx)))
            .right(right)
    }

    fn render_file_drop_observer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let _performance_scope =
            crate::ui_performance::scope("Workspace::render_file_drop_observer");
        let workspace = cx.weak_entity();
        let row_drag_workspace = workspace.clone();
        let row_drag_mouse_down_workspace = workspace.clone();
        canvas(
            |_, _, _| (),
            move |_, _, window, _cx| {
                window.on_mouse_event(move |event: &FileDropEvent, phase, _window, cx| {
                    if !phase.bubble() {
                        return;
                    }
                    let next_state = match event {
                        FileDropEvent::Entered { .. } | FileDropEvent::Pending { .. } => None,
                        FileDropEvent::Exited
                        | FileDropEvent::Submit { .. }
                        | FileDropEvent::Ended => Some((false, None)),
                    };
                    if let Some((next_visible, next_transfer)) = next_state {
                        _ = workspace.update(cx, |workspace, cx| {
                            if workspace.file_drop_visible != next_visible
                                || workspace.file_drop_tab_transfer != next_transfer
                            {
                                workspace.file_drop_visible = next_visible;
                                workspace.file_drop_tab_transfer = next_transfer;
                                cx.notify();
                            }
                        });
                    }
                });
                window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                    if !phase.capture() {
                        return;
                    }
                    _ = row_drag_workspace.update(cx, |workspace, cx| {
                        workspace.end_all_row_drag_selection(window, cx);
                    });
                    Workspace::finish_cross_window_tab_drag(event, window, cx);
                });
                window.on_mouse_event(move |_: &MouseDownEvent, phase, window, cx| {
                    if !phase.capture() {
                        return;
                    }
                    _ = row_drag_mouse_down_workspace.update(cx, |workspace, cx| {
                        if workspace.row_drag_selection.is_some() {
                            TextSelection::end(window, cx);
                            workspace.end_all_row_drag_selection(window, cx);
                        }
                    });
                });
            },
        )
        .absolute()
        .size_full()
    }
}

impl Focusable for Workspace {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for WorkspaceStatusSurface {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _performance_scope = crate::ui_performance::scope("WorkspaceStatusSurface::render");
        let workspace = self.workspace.clone();
        let element = workspace
            .update(cx, |workspace, cx| {
                workspace.render_status_bar(cx).into_any_element()
            })
            .unwrap_or_else(|_| div().into_any_element());
        crate::ui_performance::element(
            "WorkspaceStatusSurface::request_layout",
            "WorkspaceStatusSurface::prepaint",
            "WorkspaceStatusSurface::paint",
            element,
        )
    }
}

impl Render for LogRegionSurface {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _performance_scope = crate::ui_performance::scope("LogRegionSurface::render");
        let workspace = self.workspace.clone();
        let document_id = self.document_id;
        let region = self.region;
        let surface = cx.entity();
        let element = workspace
            .update(cx, |workspace, cx| {
                workspace.render_log_region_surface(document_id, region, surface, window, cx)
            })
            .unwrap_or_else(|_| div().into_any_element());
        crate::ui_performance::element(
            "LogRegionSurface::request_layout",
            "LogRegionSurface::prepaint",
            "LogRegionSurface::paint",
            element,
        )
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _performance_scope = crate::ui_performance::scope("Workspace::render");
        // 行高要按当前显示器的缩放对齐到设备像素，窗口换屏后这里会拿到新的缩放。
        self.scale_factor = window.scale_factor();
        let has_other_window = cx
            .global::<WorkspaceWindowRegistry>()
            .previous_window(window.window_handle())
            .is_some();
        let (file_drop_title, file_drop_description) = match self.file_drop_tab_transfer {
            Some(TabTransferMode::Move) => (
                crate::tr!("松开以移动标签", "Release to move the tab"),
                crate::tr!(
                    "完整会话会插入此位置；目标确认后才关闭源标签",
                    "The complete session will be inserted here; the source closes only after the destination confirms"
                ),
            ),
            Some(TabTransferMode::Copy) => (
                crate::tr!("松开以复制标签", "Release to copy the tab"),
                crate::tr!(
                    "完整会话会插入此位置；源标签保持不变",
                    "The complete session will be inserted here; the source tab remains unchanged"
                ),
            ),
            None => (
                crate::tr!("松开以打开日志", "Release to open logs"),
                crate::tr!(
                    "支持同时拖入多个文件，文件夹会被忽略",
                    "You can drop multiple files; folders are ignored"
                ),
            ),
        };
        let drop_overlay_top = window.rem_size() * 0.5;
        let drop_hint_top = window.rem_size() * 4.;
        let drop_enter_travel = window.rem_size() * 0.5;
        let colors = ui_theme::palette(cx);
        let element = v_flex()
            .id("vclogg2-workspace")
            .relative()
            .key_context(WORKSPACE_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    if !this.is_text_selection_origin_in_log_region(event.position) {
                        GlobalState::suppress_text_selection(cx);
                    }
                }),
            )
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                this.open_dropped_paths(paths, window, cx);
            }))
            .on_drag_move::<ExternalPaths>(cx.listener(
                |this, event: &DragMoveEvent<ExternalPaths>, _, cx| {
                    let next_visible = event.bounds.contains(&event.event.position);
                    let next_transfer = None;
                    if this.file_drop_visible != next_visible
                        || this.file_drop_tab_transfer != next_transfer
                    {
                        this.file_drop_visible = next_visible;
                        this.file_drop_tab_transfer = next_transfer;
                        cx.notify();
                    }
                },
            ))
            .on_drag_move::<DraggedTab>(move |event: &DragMoveEvent<DraggedTab>, window, cx| {
                let dragged = event.drag(cx).clone();
                Self::track_cross_window_tab_drag(&dragged, event, window, cx);
            })
            .on_action(cx.listener(Self::open_files))
            .on_action(cx.listener(Self::new_window))
            .on_action(cx.listener(Self::reload_active))
            .on_action(cx.listener(Self::close_active_tab))
            .on_action(cx.listener(Self::copy_current_line))
            .on_action(cx.listener(Self::copy_current_line_with_number))
            .on_action(cx.listener(Self::select_all_rows))
            .on_action(cx.listener(Self::extend_selection_up))
            .on_action(cx.listener(Self::extend_selection_down))
            .on_action(cx.listener(Self::extend_selection_page_up))
            .on_action(cx.listener(Self::extend_selection_page_down))
            .on_action(cx.listener(Self::extend_selection_first))
            .on_action(cx.listener(Self::extend_selection_last))
            .on_action(cx.listener(Self::copy_file_path))
            .on_action(cx.listener(Self::open_go_to_line))
            .on_action(cx.listener(Self::toggle_marked_row))
            .on_action(cx.listener(Self::cycle_color_label))
            .on_action(cx.listener(Self::focus_search))
            .on_action(cx.listener(Self::open_quick_find))
            .on_action(cx.listener(Self::open_settings_action))
            .on_action(cx.listener(Self::start_search_action))
            .on_action(cx.listener(Self::cancel_search_action))
            .on_action(cx.listener(Self::clear_search_action))
            .on_action(cx.listener(Self::toggle_case_sensitive))
            .on_action(cx.listener(Self::toggle_regex))
            .on_action(cx.listener(Self::toggle_word_wrap))
            .on_action(cx.listener(Self::open_search_results_in_new_tab_action))
            .on_action(cx.listener(Self::merge_search_results_in_new_tab_action))
            .on_action(cx.listener(Self::save_search_results_to_file_action))
            .on_action(cx.listener(Self::select_wrapped_up))
            .on_action(cx.listener(Self::select_wrapped_down))
            .on_action(cx.listener(Self::select_wrapped_page_up))
            .on_action(cx.listener(Self::select_wrapped_page_down))
            .on_action(cx.listener(Self::select_wrapped_first))
            .on_action(cx.listener(Self::select_wrapped_last))
            .on_action(cx.listener(Self::jump_to_start))
            .on_action(cx.listener(Self::jump_to_end))
            .on_action(cx.listener(Self::toggle_fullscreen))
            .size_full()
            .min_h_0()
            .bg(ui_theme::ambient_base(&colors))
            .text_color(cx.theme().foreground)
            .children(ui_theme::ambient_glow_layers(&colors))
            .child(self.render_title_bar(window, cx))
            .child(self.render_file_toolbar(cx))
            .child(self.render_tabs(has_other_window, cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .when(self.active_document().is_none(), |this| {
                        this.child(self.render_new_tab_workspace(cx))
                    })
                    .when(self.active_document().is_some(), |this| {
                        this.child(self.render_document_workspace(window, cx))
                    }),
            )
            .child(self.status_surface.clone())
            .child(self.render_file_drop_observer(cx))
            .when(
                self.file_drop_visible && self.file_drop_tab_transfer.is_none(),
                |this| {
                    this.child(
                        div()
                            .id("file-drop-overlay")
                            .absolute()
                            .right_2()
                            .bottom_2()
                            .left_2()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(cx.theme().radius_lg)
                            .border_2()
                            .border_dashed()
                            .border_color(cx.theme().primary)
                            .bg(cx.theme().popover)
                            .text_color(cx.theme().popover_foreground)
                            .child(
                                v_flex()
                                    .items_center()
                                    .gap_2()
                                    .child(Icon::new(IconName::FolderOpen).size_8())
                                    .child(div().text_lg().font_semibold().child(file_drop_title))
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(file_drop_description),
                                    ),
                            )
                            .with_animation(
                                "file-drop-overlay-enter",
                                Animation::new(TRANSIENT_SURFACE_ENTER_DURATION)
                                    .with_easing(ease_out_cubic),
                                move |overlay, delta| {
                                    overlay
                                        .top(drop_overlay_top - drop_enter_travel * (1. - delta))
                                        .opacity(delta)
                                },
                            ),
                    )
                },
            )
            .when(
                self.file_drop_visible && self.file_drop_tab_transfer.is_some(),
                |this| {
                    this.child(
                        h_flex()
                            .id("tab-transfer-drop-hint")
                            .absolute()
                            .right_4()
                            .gap_2()
                            .px_3()
                            .py_2()
                            .rounded(cx.theme().radius_lg)
                            .border_1()
                            .border_color(cx.theme().primary)
                            .bg(cx.theme().popover)
                            .text_color(cx.theme().popover_foreground)
                            .shadow_lg()
                            .child(Icon::new(IconName::FolderOpen).size_5())
                            .child(
                                v_flex()
                                    .gap_1()
                                    .child(div().font_semibold().child(file_drop_title))
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(file_drop_description),
                                    ),
                            )
                            .with_animation(
                                "tab-transfer-drop-hint-enter",
                                Animation::new(TRANSIENT_SURFACE_ENTER_DURATION)
                                    .with_easing(ease_out_cubic),
                                move |hint, delta| {
                                    hint.top(drop_hint_top - drop_enter_travel * (1. - delta))
                                        .opacity(delta)
                                },
                            ),
                    )
                },
            )
            .child(crate::modal_event_layer::render_foreground_pointer_barrier())
            .children(Root::render_dialog_layer(window, cx))
            .children(Root::render_sheet_layer(window, cx))
            .children(Root::render_notification_layer(window, cx));
        crate::ui_performance::element(
            "Workspace::request_layout",
            "Workspace::prepaint",
            "Workspace::paint",
            element,
        )
    }
}

mod document_tasks;
use document_tasks::*;

#[cfg(test)]
#[path = "workspace/result_snapshot_tests.rs"]
mod result_snapshot_tests;

fn recent_file_label(recent: &RecentFile) -> String {
    let name = recent
        .path
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_else(|| recent.path.as_os_str().to_string_lossy());
    let parent = recent
        .path
        .parent()
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    let age = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_secs()).ok())
        .map(|now| now.saturating_sub(recent.last_opened_at))
        .unwrap_or_default();
    let age = match age {
        0..60 => crate::tr!("刚刚", "Just now").to_string(),
        60..3600 => crate::tr_args!("{} 分钟前", "{} minutes ago", age / 60),
        3600..86400 => crate::tr_args!("{} 小时前", "{} hours ago", age / 3600),
        _ => crate::tr_args!("{} 天前", "{} days ago", age / 86400),
    };
    format!("{name} — {parent} · {age}")
}

fn empty_file_button_content(
    path: &Path,
    last_opened_at: Option<i64>,
    cx: &App,
) -> impl IntoElement {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.as_os_str().to_string_lossy().into_owned());
    let parent = path
        .parent()
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    let opened_at = last_opened_at
        .and_then(|timestamp| DateTime::from_timestamp(timestamp, 0))
        .map(|timestamp| {
            timestamp
                .with_timezone(&Local)
                .format("%m/%d %H:%M")
                .to_string()
        })
        .unwrap_or_default();

    h_flex()
        .w_full()
        .min_w_0()
        .justify_start()
        .gap_3()
        .child(
            div()
                .w_4()
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    svg()
                        .data(include_bytes!(
                            "../assets/icons/document-text-20-regular.svg"
                        ))
                        .size(rems(1.))
                        .text_color(cx.theme().primary),
                ),
        )
        .child(
            div()
                .w(rems(23.))
                .min_w_0()
                .flex_shrink_1()
                .truncate()
                .text_sm()
                .child(name),
        )
        .child(
            div()
                .min_w_0()
                .flex_1()
                .truncate()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(parent),
        )
        .child(
            div()
                .w(rems(7.))
                .flex_none()
                .text_right()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(opened_at),
        )
}

fn format_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.;
    const MIB: f64 = KIB * 1024.;
    const GIB: f64 = MIB * 1024.;

    let bytes_f64 = bytes as f64;
    if bytes_f64 >= GIB {
        format!("{:.1} GiB", bytes_f64 / GIB)
    } else if bytes_f64 >= MIB {
        format!("{:.1} MiB", bytes_f64 / MIB)
    } else if bytes_f64 >= KIB {
        format!("{:.1} KiB", bytes_f64 / KIB)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
#[path = "workspace/document_prepare_performance_tests.rs"]
mod document_prepare_performance_tests;
