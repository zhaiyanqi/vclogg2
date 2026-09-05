use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet, HashSet},
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
    actions::{SelectDown, SelectFirst, SelectLast, SelectPageDown, SelectPageUp, SelectUp},
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, ElementExt as _, FocusableExt as _, Icon, IconName,
    IndexPath, Root, Selectable as _, Side, Sizable as _, StyledExt as _, TitleBar, WindowExt as _,
    animation::ease_out_cubic,
    button::{Button, ButtonCustomVariant, ButtonRounded, ButtonVariants as _},
    checkbox::Checkbox,
    dialog::DialogFooter,
    h_flex,
    input::{Input, InputEvent, InputState},
    menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenu, PopupMenuItem},
    popover::Popover,
    resizable::{ResizableState, resizable_panel, v_resizable},
    scroll::ScrollableElement as _,
    select::{Select, SelectEvent, SelectState},
    status_bar::StatusBar,
    tab::{Tab, TabBar},
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
        LogTableCursor, LogTableDelegate, TextHighlight, VirtualLogListStateExt, line_marker,
        line_marker_column_width, log_cell_horizontal_padding, log_fixed_column_divider_overlay,
        log_level_accent_overlay, log_line_height, log_line_number_cell, log_row_selection_color,
        log_row_selection_overlay, log_row_separator_overlay, message_column_width,
        text_highlight_style,
    },
    path_identity::{
        PathMatchKey, decode_persisted_path, deduplicate_paths, encode_persisted_path,
        normalized_path_match_key, path_buf_map_get, path_buf_map_insert, path_buf_map_remove,
        path_match_key, path_match_map_get, path_match_set_contains, paths_match,
    },
    predefined_filters::{PredefinedFilter, query_includes_filter, toggle_filter_in_query},
    predefined_filters_dialog::{
        PredefinedFiltersDialog, PredefinedFiltersDialogEvent, predefined_filters_dialog_size,
    },
    rename_tab_dialog::RenameTabDialog,
    result_export::{self, ExportGroup, ResultExport},
    search_autocomplete::{
        SearchSuggestion, SearchSuggestionSource, apply_search_suggestion,
        search_autocomplete_needle, search_autocomplete_suggestions,
    },
    search_context::{
        PersistedDirectorySearchOptions, PersistedDirectorySearchSession,
        PersistedGlobalSearchContext, PersistedPathSelection, PersistedSearchQuery,
        PersistedSearchRowKey, PersistedSearchScope, PersistedSearchViewport, WorkspaceSearchState,
    },
    selectable_log_text::{LogText, LogTextSelection, SelectableLogText, TextSelectionCache},
    settings_dialog::{
        SettingsCategory, SettingsDialog, SettingsDialogEvent, SettingsNetworkSnapshot,
    },
    state_store::{
        AppSettings, CloudSettings, FileSessionState, LastWorkspaceFile, RecentFile,
        ShortcutSettings, StateStore, ThemePreference, normalize_search_history,
    },
    tab_resume::{PersistedLogRegion, TabResumeState, ViewportBookmark},
    ui_theme,
    virtual_log_lines::{LogRowKey, StagedVisibleLineLoadRequest, StagedVisibleLineLoadResult},
    virtual_log_list::{
        VirtualLogListDelegate, VirtualLogListEvent, VirtualLogListScrollHandle,
        VirtualLogListState, VirtualLogRow, VirtualLogViewport, v_virtual_log_list,
    },
    workspace_state::{
        CloudController, GlobalSearchDocumentResult, GlobalSearchResults, GlobalSearchState,
        PersistenceController, QuickFindBoundary, QuickFindDirection, QuickFindMatch,
        QuickFindSource, QuickFindSourceVersion, QuickFindState, QuickFindTarget, ResultMode,
        RowViewportAnchor, SearchController, SearchScope, SearchSessionState, SearchTarget,
        ViewportAnchor,
    },
};

const PREVIEW_BYTE_LIMIT: usize = 1024 * 1024;
const PREVIEW_LINE_LIMIT: usize = 200;
const MAX_DOCUMENT_PREPARE_WORKERS: usize = 4;
const SEARCH_SUGGESTION_ROW_HEIGHT_REMS: f32 = 3.25;
const GITHUB_RELEASES_URL: &str = "https://github.com/zhaiyanqi/vclogg2/releases";
const SEARCH_SUGGESTION_MAX_VISIBLE_ROWS: usize = 5;
const SEARCH_CONTROL_HEIGHT: Pixels = px(28.);
const SEARCH_BAR_VERTICAL_INSET: Pixels = px(6.);
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
    cached_complete_document: Option<Arc<LogDocument>>,
    session: Option<FileSessionState>,
    color_labels_snapshot: Option<Vec<ColorLabel>>,
    resolved_color_rules: Arc<ResolvedColorRules>,
    search_result: SearchResult,
    search_matcher: Option<SearchMatcher>,
    search_case_sensitive: bool,
    search_regex: bool,
    warning: Option<String>,
    load_state: DocumentLoadState,
    pending_index_cache: Option<PendingIndexCacheWrite>,
    upgrade_frame: Option<PreparedDocumentUpgradeFrame>,
}

struct DocumentUpgradeLoadJob {
    path: PathBuf,
    previous_document: Arc<LogDocument>,
    document: Arc<LogDocument>,
    result_rows: CompressedRows,
    log_request: StagedVisibleLineLoadRequest<usize>,
    result_request: StagedVisibleLineLoadRequest<usize>,
    log_anchor: Option<ViewportAnchor<LogRowKey>>,
    result_anchor: Option<ViewportAnchor<LogRowKey>>,
    log_measured_heights: BTreeMap<LogRowKey, Pixels>,
    result_measured_heights: BTreeMap<LogRowKey, Pixels>,
    row_height: Pixels,
    log_word_wrap: bool,
    result_word_wrap: bool,
    log_jump: Option<PreparedLogJump>,
}

struct PreparedDocumentUpgradeFrame {
    path: PathBuf,
    previous_document: Arc<LogDocument>,
    document: Arc<LogDocument>,
    result_rows: CompressedRows,
    log_lines: StagedVisibleLineLoadResult<usize>,
    result_lines: StagedVisibleLineLoadResult<usize>,
    log_anchor: Option<ViewportAnchor<LogRowKey>>,
    result_anchor: Option<ViewportAnchor<LogRowKey>>,
    log_measured_heights: BTreeMap<LogRowKey, Pixels>,
    result_measured_heights: BTreeMap<LogRowKey, Pixels>,
    row_height: Pixels,
    log_word_wrap: bool,
    result_word_wrap: bool,
    log_jump: Option<PreparedLogJump>,
}

struct DocumentFontViewportAnchors {
    document_id: u64,
    log: Option<RowViewportAnchor<LogRowKey>>,
    results: Option<RowViewportAnchor<LogRowKey>>,
}

struct FontViewportAnchors {
    documents: Vec<DocumentFontViewportAnchors>,
    global_results: Option<RowViewportAnchor<LogRowKey>>,
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

fn should_sync_active_document_controls(
    final_phase: bool,
    previous_active_id: Option<u64>,
    active_id: Option<u64>,
    active_session_was_restored: bool,
) -> bool {
    !final_phase || previous_active_id != active_id || active_session_was_restored
}

fn should_defer_directory_group_activation(
    pending_path: Option<&Path>,
    candidate_path: &Path,
    load_state: DocumentLoadState,
) -> bool {
    load_state != DocumentLoadState::Ready
        && pending_path.is_some_and(|pending_path| paths_match(pending_path, candidate_path))
}

fn next_persisted_search_restore_scope(
    active_scope: SearchScope,
    has_all_open: bool,
    has_directory: bool,
) -> Option<SearchScope> {
    [
        active_scope,
        SearchScope::AllOpenFiles,
        SearchScope::Directory,
    ]
    .into_iter()
    .find(|scope| match scope {
        SearchScope::AllOpenFiles => has_all_open,
        SearchScope::Directory => has_directory,
        SearchScope::CurrentFile => false,
    })
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
    search_options: Option<(bool, bool)>,
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

struct DocumentTab {
    id: u64,
    opened_at: i64,
    file: FileState,
    view: FileViewState,
    document: Arc<LogDocument>,
    session_base: FileSessionState,
    log_table: Entity<VirtualLogListState<LogTableDelegate, LogRowKey>>,
    result_table: Entity<VirtualLogListState<LogTableDelegate, LogRowKey>>,
    log_viewport: LogViewportState<usize>,
    result_viewport: LogViewportState<usize>,
    search_query: SearchQuery,
    search_result: SearchResult,
    search_matcher: Option<SearchMatcher>,
    result_mode: ResultMode,
    result_mode_select: Entity<SelectState<Vec<ResultMode>>>,
    // Document controls must stop notifying the workspace when their tab closes.
    _subscriptions: [Subscription; 3],
    search_revision: u64,
    log_jump_revision: u64,
    log_jump_task: Option<Task<()>>,
    result_replace_revision: u64,
    result_replace_task: Option<Task<()>>,
    result_replace_cancellation: Option<Arc<AtomicBool>>,
    results_visible: bool,
    restoring_result_selection: bool,
    load_state: DocumentLoadState,
}

struct PreparedTabFrame {
    document_id: u64,
    document: Arc<LogDocument>,
    log_revision: u64,
    result_revision: u64,
    log_jump: Option<PreparedLogJump>,
    log_lines: Option<StagedVisibleLineLoadResult<usize>>,
    result_lines: Option<StagedVisibleLineLoadResult<usize>>,
}

#[derive(Clone, Copy)]
struct PreparedLogJump {
    source_row: usize,
    row_ix: usize,
}

struct PreparedGlobalGroupToggle {
    plan: GlobalGroupTogglePlan,
    staged: Option<StagedVisibleLineLoadResult<(u64, usize)>>,
    anchor: Option<RowViewportAnchor<LogRowKey>>,
    measured_heights: BTreeMap<LogRowKey, Pixels>,
    row_height: Pixels,
}

struct PreparedLocalResultReplacement {
    document_id: u64,
    document: Arc<LogDocument>,
    previous_rows: CompressedRows,
    rows: CompressedRows,
    matched_rows: CompressedRows,
    marked_rows: CompressedRows,
    matcher: Option<SearchMatcher>,
    staged: StagedVisibleLineLoadResult<usize>,
    viewport_anchor: Option<ViewportAnchor<LogRowKey>>,
    measured_heights: BTreeMap<LogRowKey, Pixels>,
    row_height: Pixels,
    word_wrap: bool,
}

struct PreparedGlobalResultReplacement {
    expected_content_revision: u64,
    expected_layout_revision: u64,
    groups: Vec<GlobalSearchGroup>,
    matcher: Option<SearchMatcher>,
    staged: StagedVisibleLineLoadResult<(u64, usize)>,
    viewport_anchor: Option<ViewportAnchor<LogRowKey>>,
    measured_heights: BTreeMap<LogRowKey, Pixels>,
    row_height: Pixels,
    word_wrap: bool,
    restore_context: Option<SearchSessionState>,
}

struct ReloadReplacementInput {
    document_id: u64,
    revision: u64,
    previous_document: Arc<LogDocument>,
    document: Arc<LogDocument>,
    search_result: SearchResult,
    query: SearchQuery,
    search_matcher: Option<SearchMatcher>,
    results_visible: bool,
    follow_end: bool,
    selected_source_row: Option<usize>,
}

struct ReloadReplacementPlan {
    document_id: u64,
    revision: u64,
    previous_document: Arc<LogDocument>,
    document: Arc<LogDocument>,
    search_result: SearchResult,
    query: SearchQuery,
    search_matcher: Option<SearchMatcher>,
    marked_rows: CompressedRows,
    result_rows: CompressedRows,
    results_visible: bool,
    follow_end: bool,
    selected_source_row: Option<usize>,
    selected_result_row: Option<usize>,
    log_request: Option<StagedVisibleLineLoadRequest<usize>>,
    result_request: Option<StagedVisibleLineLoadRequest<usize>>,
    log_anchor: Option<ViewportAnchor<LogRowKey>>,
    result_anchor: Option<ViewportAnchor<LogRowKey>>,
    log_measured_heights: BTreeMap<LogRowKey, Pixels>,
    result_measured_heights: BTreeMap<LogRowKey, Pixels>,
    row_height: Pixels,
    log_word_wrap: bool,
    result_word_wrap: bool,
}

struct PreparedReloadReplacement {
    plan: ReloadReplacementPlan,
    log_lines: StagedVisibleLineLoadResult<usize>,
    result_lines: StagedVisibleLineLoadResult<usize>,
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

#[derive(Clone, Copy)]
struct WrappedFramePrimeOptions {
    minimum_viewport_height: Pixels,
    reset_for_mode_switch: bool,
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
    role: DisplayRegion,
    _workspace_subscription: Subscription,
    _table_subscription: Option<Subscription>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DisplayRegion {
    Log,
    SearchResults,
}

/// Stable retained host for one half of the main split. Session changes replace the projected
/// data behind this host; they do not replace the GPUI surface, focus identity, or selection
/// scope.
struct SharedDisplayState {
    surface: Entity<LogRegionSurface>,
    focus_handle: FocusHandle,
    text_selection_scope: TextSelectionScopeId,
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
    fn new(workspace: WeakEntity<Workspace>, role: DisplayRegion, cx: &mut Context<Self>) -> Self {
        let workspace_entity = workspace
            .upgrade()
            .expect("workspace is alive while creating its display surface");
        let workspace_subscription = cx.observe(&workspace_entity, |_, _, cx| cx.notify());
        Self {
            workspace,
            role,
            _workspace_subscription: workspace_subscription,
            _table_subscription: None,
        }
    }

    fn bind_table<D, K>(
        &mut self,
        table: &Entity<VirtualLogListState<D, K>>,
        cx: &mut Context<Self>,
    ) where
        D: 'static,
        K: 'static,
    {
        self._table_subscription = Some(cx.observe(table, |_, _, cx| cx.notify()));
    }
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

fn tab_switch_log_jump_preload_range(
    target_row: usize,
    row_count: usize,
    table_visible_rows: usize,
    measured_visible_rows: usize,
    window_visible_rows: usize,
) -> Range<usize> {
    centered_log_jump_preload_range(
        target_row,
        row_count,
        table_visible_rows
            .max(measured_visible_rows)
            .max(window_visible_rows),
    )
}

fn search_scope_switch_preload_range(
    anchor_row: usize,
    at_end: bool,
    row_count: usize,
    visible_row_count: usize,
) -> Range<usize> {
    if row_count == 0 {
        return 0..0;
    }
    let visible_row_count = visible_row_count.max(1);
    let anchor_row = if at_end {
        row_count.saturating_sub(1)
    } else {
        anchor_row.min(row_count - 1)
    };
    centered_log_jump_preload_range(anchor_row, row_count, visible_row_count)
}

fn log_font_layout_changed(current: &AppSettings, next: &AppSettings) -> bool {
    current.log_font_size != next.log_font_size
        || current.log_line_spacing != next.log_line_spacing
        || current.log_font_family != next.log_font_family
}

fn point_in_text_selection_regions(
    position: Point<Pixels>,
    regions: impl IntoIterator<Item = Bounds<Pixels>>,
) -> bool {
    regions.into_iter().any(|bounds| bounds.contains(&position))
}

fn restore_current_result_selection(
    table: &mut VirtualLogListState<LogTableDelegate, LogRowKey>,
    row_ix: usize,
    cx: &mut Context<VirtualLogListState<LogTableDelegate, LogRowKey>>,
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
            &self.file.marked_rows,
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
        let marked_rows = self.file.marked_rows.clone();
        let active_restored = self.result_table.update(cx, |table, cx| {
            if self.view.auto_follow {
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

    fn refresh_view_options(&self, cx: &mut App) {
        let show_line_numbers = self.view.show_line_numbers;
        let show_row_separators = self.view.show_row_separators;
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
        let table = match self.view.selection_table {
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
        let table = match self.view.selection_table {
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

struct ColorRulePropagationTarget {
    document_id: u64,
    document: Arc<LogDocument>,
    expected_rules: Vec<KeywordColorRule>,
}

struct ColorRuleSessionTarget {
    scope: SearchScope,
    expected_revision: u64,
    expected_rules: Vec<KeywordColorRule>,
}

struct PreparedColorRulePropagation {
    document_id: u64,
    document: Arc<LogDocument>,
    expected_rules: Vec<KeywordColorRule>,
    rules: Vec<KeywordColorRule>,
    resolved: Arc<ResolvedColorRules>,
}

struct PreparedColorRuleSession {
    scope: SearchScope,
    expected_revision: u64,
    expected_rules: Vec<KeywordColorRule>,
    rules: Vec<KeywordColorRule>,
    resolved: Arc<ResolvedColorRules>,
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

struct ColorRuleUpdateInput {
    target: ColorKeywordTarget,
    collect_keywords: bool,
    action: ColorRuleAction,
    rules: Vec<KeywordColorRule>,
    labels: Vec<ColorLabel>,
    last_color_label_id: Option<String>,
    propagation_targets: Vec<ColorRulePropagationTarget>,
    session_target: Option<ColorRuleSessionTarget>,
}

struct PreparedColorRuleUpdate {
    document_id: u64,
    document: Arc<LogDocument>,
    expected_rules: Vec<KeywordColorRule>,
    expected_labels: Vec<ColorLabel>,
    rules: Vec<KeywordColorRule>,
    resolved: Option<Arc<ResolvedColorRules>>,
    propagated_files: Vec<PreparedColorRulePropagation>,
    search_session: Option<PreparedColorRuleSession>,
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

struct DirectorySearchRun {
    cancelled: bool,
    results: Vec<DirectorySearchResult>,
    matcher: Option<SearchMatcher>,
    file_count: usize,
    open_error_count: usize,
    unreadable_directory_count: usize,
}

#[derive(Clone)]
struct PendingSearchResultJump {
    path: PathBuf,
    source_row: usize,
    expected_document: Arc<LogDocument>,
}

impl PendingSearchResultJump {
    fn matches(&self, document: &LogDocument) -> bool {
        result_snapshot_matches_document(&self.path, &self.expected_document, document)
    }
}

fn pending_search_result_jump(
    scope: Option<SearchScope>,
    result: Option<&GlobalSearchDocumentResult>,
    source_row: usize,
) -> Option<PendingSearchResultJump> {
    matches!(
        scope,
        Some(SearchScope::AllOpenFiles | SearchScope::Directory)
    )
    .then(|| {
        result.map(|result| PendingSearchResultJump {
            path: result.path.clone(),
            source_row,
            expected_document: result.document.clone(),
        })
    })
    .flatten()
}

fn prepared_pending_search_jump(
    pending: Option<&PendingSearchResultJump>,
    path: &Path,
    document: &LogDocument,
) -> Option<PreparedLogJump> {
    let pending =
        pending.filter(|pending| paths_match(&pending.path, path) && pending.matches(document))?;
    Some(PreparedLogJump {
        source_row: pending.source_row,
        row_ix: document.local_row(pending.source_row)?,
    })
}

fn resolved_prepared_search_jump(
    staged: Option<PreparedLogJump>,
    pending: Option<&PendingSearchResultJump>,
    path: &Path,
    document: &LogDocument,
) -> Option<PreparedLogJump> {
    staged.or_else(|| prepared_pending_search_jump(pending, path, document))
}

fn opening_restore_source_row(
    selected_result_jump: Option<PreparedLogJump>,
    persisted_source_row: Option<usize>,
) -> Option<usize> {
    selected_result_jump
        .map(|jump| jump.source_row)
        .or(persisted_source_row)
}

fn should_defer_search_result_jump(load_state: Option<DocumentLoadState>) -> bool {
    load_state != Some(DocumentLoadState::Ready)
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
    view_state: WorkspaceViewState,
    global_search: GlobalSearchState,
    global_table: Entity<VirtualLogListState<GlobalSearchTableDelegate, LogRowKey>>,
    log_viewer: SharedDisplayState,
    search_results_viewer: SharedDisplayState,
    global_viewport: LogViewportState<(u64, usize)>,
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
    pending_search_result_jump: Option<PendingSearchResultJump>,
    search_result_jump_revision: u64,
    pending_directory_group_activation: Option<PathBuf>,
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
    scroll_frame_tasks: BTreeMap<(u64, WrappedRegion), Task<()>>,
    scroll_frame_cancellations: BTreeMap<(u64, WrappedRegion), Arc<AtomicBool>>,
    active_scroll_frames: BTreeMap<(u64, WrappedRegion), (u64, LogScrollFrameTarget)>,
    next_scroll_frame_revision: u64,
    global_group_toggle_task: Option<Task<()>>,
    global_group_toggle_revision: u64,
    global_result_replace_task: Option<Task<()>>,
    global_result_replace_cancellation: Option<Arc<AtomicBool>>,
    global_result_replace_revision: u64,
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
    search_options_modified: bool,
    _subscriptions: Vec<Subscription>,
    history_dialog_subscription: Option<Subscription>,
    predefined_filters_dialog_subscription: Option<Subscription>,
    settings_dialog_subscription: Option<Subscription>,
}

impl Workspace {}

mod document_commands;
mod document_lifecycle;
mod document_opening;
mod log_presentation;
mod log_viewport;
mod preferences;
mod quick_find;
mod render_shell;
mod result_export_flow;
mod search_orchestration;
mod tab_lifecycle;
mod view_state;
mod viewport_orchestration;
mod window_registry;

use log_viewport::*;
use view_state::*;

impl Workspace {
    pub fn new(
        primary_window: bool,
        initial_documents: Vec<InitialDocument>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        window.set_window_title(crate::tr!("新标签页 — VCLogg2", "New tab — VCLogg2"));
        let (initial_app_settings, initial_search_options) = Self::initial_search_settings(cx);
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
        let global_table = cx.new(|_| {
            VirtualLogListState::new(GlobalSearchTableDelegate::new(), VirtualLogViewport::new())
        });
        let global_viewport = {
            let table = global_table.read(cx);
            LogViewportState::new(
                false,
                table.viewport().clone(),
                table.delegate().row_bounds_handle(),
            )
        };
        let log_surface = {
            let workspace = cx.weak_entity();
            cx.new(move |cx| LogRegionSurface::new(workspace, DisplayRegion::Log, cx))
        };
        let search_results_surface = {
            let workspace = cx.weak_entity();
            cx.new(move |cx| LogRegionSurface::new(workspace, DisplayRegion::SearchResults, cx))
        };
        let log_text_selection_scope = TextSelectionScopeId::default();
        let search_results_text_selection_scope = TextSelectionScopeId::default();
        let log_focus_handle = cx.focus_handle().tab_stop(true);
        let search_results_focus_handle = cx.focus_handle().tab_stop(true);
        let search_panel_state = cx.new(|_| ResizableState::default());
        cx.on_focus_in(&log_focus_handle, window, |this: &mut Workspace, _, cx| {
            this.active_log_region = LogRegion::Body;
            cx.notify();
        })
        .detach();
        cx.on_focus_in(
            &search_results_focus_handle,
            window,
            |this: &mut Workspace, _, cx| {
                this.active_log_region = match this.global_search.scope {
                    SearchScope::CurrentFile => LogRegion::CurrentResults,
                    SearchScope::AllOpenFiles | SearchScope::Directory => LogRegion::GlobalResults,
                };
                cx.notify();
            },
        )
        .detach();
        let log_viewer = SharedDisplayState {
            surface: log_surface,
            focus_handle: log_focus_handle,
            text_selection_scope: log_text_selection_scope,
        };
        let search_results_viewer = SharedDisplayState {
            surface: search_results_surface,
            focus_handle: search_results_focus_handle,
            text_selection_scope: search_results_text_selection_scope,
        };
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
            |this, table, event: &VirtualLogListEvent, window, cx| {
                if matches!(event, VirtualLogListEvent::ClearSelection) {
                    if table.read(cx).delegate().take_suppressed_table_clear() {
                        return;
                    }
                    table.read(cx).delegate().clear_row_selection();
                    table.read(cx).delegate().set_active_log_row(None);
                    this.schedule_workspace_search_state_save(window, cx);
                    cx.notify();
                    return;
                }
                let VirtualLogListEvent::SelectRow(row_ix) = event else {
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
                    this.search_results_viewer.focus_handle.focus(window, cx);
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
                            && !tab.file.marked_rows.is_empty()
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
                let Some(directory) = crate::app_paths::index_cache_dir() else {
                    return;
                };
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
                        let preserve_search_options = this.search_options_modified;
                        let local_search_options = preserve_search_options
                            .then_some((this.case_sensitive, this.regex));
                        let search_options = cx.update_global::<WorkspaceWindowRegistry, _>(
                            |registry, _| {
                                if let Some(search_options) = local_search_options {
                                    registry.search_options = Some(search_options);
                                    search_options
                                } else {
                                    *registry.search_options.get_or_insert((
                                        app_settings.default_case_sensitive,
                                        app_settings.default_use_regex,
                                    ))
                                }
                            },
                        );
                        app_settings.default_case_sensitive = search_options.0;
                        app_settings.default_use_regex = search_options.1;
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
                        Self::apply_theme_preference(&app_settings, window, cx);
                        this.app_settings = app_settings.clone();
                        this.restore_search_panel_height(search_panel_height, window, cx);
                        this.apply_global_search_options(
                            app_settings.default_case_sensitive,
                            app_settings.default_use_regex,
                        );
                        this.search_options_modified = preserve_search_options;
                        if preserve_search_options {
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
                            .filter(|tab| tab.view.uses_default_view_options)
                        {
                            tab.view.show_line_numbers = app_settings.default_show_line_numbers;
                            tab.view.show_row_separators = app_settings.default_show_row_separators;
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
            view_state: WorkspaceViewState::default(),
            global_search: GlobalSearchState::new(global_result_mode_select),
            global_table,
            log_viewer,
            search_results_viewer,
            global_viewport,
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
            pending_search_result_jump: None,
            search_result_jump_revision: 0,
            pending_directory_group_activation: None,
            next_document_id: 1,
            next_new_tab_id: 2,
            case_sensitive: initial_search_options.0,
            regex: initial_search_options.1,
            activity: Activity::Ready,
            selected_source_row: None,
            row_drag_bounds: BTreeMap::new(),
            row_drag_selection: None,
            row_drag_frame_scheduled: false,
            visible_line_tasks: BTreeMap::new(),
            pending_log_scroll_frames: PendingLogScrollFrames::default(),
            scroll_frame_tasks: BTreeMap::new(),
            scroll_frame_cancellations: BTreeMap::new(),
            active_scroll_frames: BTreeMap::new(),
            next_scroll_frame_revision: 0,
            global_group_toggle_task: None,
            global_group_toggle_revision: 0,
            global_result_replace_task: None,
            global_result_replace_cancellation: None,
            global_result_replace_revision: 0,
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
            app_settings: initial_app_settings,
            scale_factor: 1.,
            color_labels: default_color_labels(),
            last_color_label_id: None,
            color_labels_saving: false,
            predefined_filters_saving: false,
            pending_predefined_filters_save: None,
            settings_saving: false,
            search_options_modified: false,
            _subscriptions: subscriptions,
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
                .map(|tab| tab.file.title.clone())
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
        let role = self.role;
        let surface = cx.entity();
        let element = workspace
            .update(cx, |workspace, cx| {
                let Some(document_id) = workspace.active_document().map(|tab| tab.id) else {
                    return div().into_any_element();
                };
                let region = match role {
                    DisplayRegion::Log => WrappedRegion::Log,
                    DisplayRegion::SearchResults => match workspace.global_search.scope {
                        SearchScope::CurrentFile => WrappedRegion::Results,
                        SearchScope::AllOpenFiles | SearchScope::Directory => {
                            WrappedRegion::GlobalResults
                        }
                    },
                };
                let document_id = if region == WrappedRegion::GlobalResults {
                    0
                } else {
                    document_id
                };
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
                    this.child(render_shell::deferred_workspace_overlay(
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
                    ))
                },
            )
            .when(
                self.file_drop_visible && self.file_drop_tab_transfer.is_some(),
                |this| {
                    this.child(render_shell::deferred_workspace_overlay(
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
                    ))
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
                .line_height(relative(1.25))
                .child(name),
        )
        .child(
            div()
                .min_w_0()
                .flex_1()
                .truncate()
                .text_xs()
                .line_height(relative(1.25))
                .text_color(cx.theme().muted_foreground)
                .child(parent),
        )
        .child(
            div()
                .w(rems(7.))
                .flex_none()
                .text_right()
                .text_xs()
                .line_height(relative(1.25))
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
