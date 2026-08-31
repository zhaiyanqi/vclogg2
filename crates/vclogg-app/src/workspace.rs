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
    IndexPath, Root, Selectable as _, Side, Sizable as _, StyledExt as _, TitleBar,
    VirtualListScrollHandle, WindowExt as _,
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
    v_flex, v_virtual_list,
};
use rayon::prelude::{IntoParallelIterator as _, ParallelIterator as _};
use vclogg_core::{
    CompressedRows, LogDocument, PendingIndexCacheWrite, SearchCancellation, SearchMatcher,
    SearchProgress, SearchQuery, SearchResult, SearchRun, search_with_compiled_matcher,
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
        ColorLabel, KeywordColorRule, ResolvedColorRule, color_with_alpha, default_color_labels,
        resolve_color_rules,
    },
    color_labels_dialog::ColorLabelsDialog,
    directory_search_dialog::{
        DirectorySearchDialog, DirectorySearchOptions, enumerate_directory_search_paths,
    },
    global_search_files_dialog::{GlobalSearchFileOption, GlobalSearchFilesDialog},
    global_search_table::{
        GlobalSearchGroup, GlobalSearchGroupHeader, GlobalSearchRow, GlobalSearchTableDelegate,
        WrappedGlobalRow,
    },
    history_dialog::{HistoryDialog, HistoryDialogEvent},
    log_table::{
        LogTableCursor, LogTableDelegate, LogTableStateExt, TextHighlight, line_marker,
        line_marker_column_width, log_cell_horizontal_padding, log_fixed_column_divider_overlay,
        log_line_height, log_line_number_cell, log_row_selection_color, log_row_selection_overlay,
        log_row_separator_overlay, scroll_uniform_log_row_to_viewport_y, severity_accent_overlay,
        severity_style, text_highlight_style,
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
        WorkspaceSearchState, compress_rows,
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
    updater::{
        AvailableUpdate, DownloadedUpdate, UpdateClient, UpdateDownloadProgress, launch_installer,
    },
    virtual_log_lines::LogRowKey,
    workspace_state::{
        AppUpdateState, CloudController, GlobalSearchDocumentResult, GlobalSearchResults,
        GlobalSearchState, PersistenceController, QuickFindBoundary, QuickFindDirection,
        QuickFindMatch, QuickFindSource, QuickFindState, QuickFindTarget, ResultMode,
        RetainedGlobalSearchContext, RowViewportAnchor, SearchController, SearchScope,
        SearchTarget, UpdateController, ViewportAnchor,
    },
};

const WRAPPED_HEIGHT_CACHE_LIMIT: usize = 4096;
const PREVIEW_BYTE_LIMIT: usize = 1024 * 1024;
const PREVIEW_LINE_LIMIT: usize = 200;
const MAX_DOCUMENT_PREPARE_WORKERS: usize = 4;
const SEARCH_SUGGESTION_ROW_HEIGHT_REMS: f32 = 3.25;
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
    results_visible: bool,
    restoring_result_selection: bool,
    marked_rows: Arc<BTreeSet<usize>>,
    pending_restore_marked_rows: BTreeSet<usize>,
    keyword_color_rules: Vec<KeywordColorRule>,
    resolved_color_rules: Arc<[ResolvedColorRule]>,
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

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum WrappedRegion {
    Log,
    Results,
    GlobalResults,
}

struct WrappedListState<K> {
    item_sizes: Rc<RefCell<Rc<Vec<Size<Pixels>>>>>,
    base_height: Rc<Cell<Pixels>>,
    pending_heights: RefCell<BTreeMap<usize, Pixels>>,
    measured_rows: RefCell<VecDeque<usize>>,
    height_corrections: Rc<RefCell<Vec<(usize, Pixels)>>>,
    scroll_handle: VirtualListScrollHandle,
    text_selections: RefCell<TextSelectionCache<K>>,
    measurement_anchor: Rc<Cell<Option<RowViewportPosition>>>,
    scrollbar_measurement_pending: Rc<Cell<bool>>,
    layout_key: RefCell<Option<WrappedLayoutKey>>,
    row_bounds: Rc<RefCell<BTreeMap<usize, Bounds<Pixels>>>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LogViewportMode {
    Fixed,
    Wrapped,
}

#[derive(Clone)]
struct FixedListState {
    scroll_handle: UniformListScrollHandle,
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
                row_bounds,
            },
            wrapped: WrappedListState::default(),
        }
    }

    fn is_wrapped(&self) -> bool {
        self.mode.get() == LogViewportMode::Wrapped
    }

    fn set_word_wrap(&self, enabled: bool) {
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

    fn apply_wheel_scroll(
        &self,
        delta_y: Pixels,
        row_count: usize,
        row_height: Pixels,
        line_count: usize,
        line_scroll: bool,
        scale: f32,
    ) {
        if self.is_wrapped() {
            self.wrapped.clear_measurement_anchor();
            if line_scroll {
                self.wrapped
                    .apply_wheel_line_scroll(delta_y, row_count, line_count);
            } else {
                self.wrapped.scale_native_wheel_scroll(delta_y, scale);
            }
        } else if line_scroll {
            apply_uniform_wheel_line_scroll(
                &self.fixed.scroll_handle,
                delta_y,
                row_count,
                row_height,
                line_count,
            );
        } else {
            scale_uniform_wheel_scroll(&self.fixed.scroll_handle, delta_y, scale);
        }
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

    fn wrapped_scroll_handle(&self) -> VirtualListScrollHandle {
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

    fn wrapped_sizes(&self, count: usize, base_height: Pixels) -> Rc<Vec<Size<Pixels>>> {
        self.wrapped.sizes(count, base_height)
    }

    fn wrapped_logical_scroll_handle(
        &self,
        item_count: usize,
        slot_height: Pixels,
    ) -> LogicalVirtualScrollHandle {
        self.wrapped.logical_scroll_handle(item_count, slot_height)
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

    fn take_wrapped_scrollbar_measurement_request(&self) -> bool {
        self.wrapped.take_scrollbar_measurement_request()
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
    handle: VirtualListScrollHandle,
    item_sizes: Rc<RefCell<Rc<Vec<Size<Pixels>>>>>,
    height_corrections: Rc<RefCell<Vec<(usize, Pixels)>>>,
    measurement_anchor: Rc<Cell<Option<RowViewportPosition>>>,
    scrollbar_measurement_pending: Rc<Cell<bool>>,
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
                .item_sizes
                .borrow()
                .get(row)
                .map_or(self.slot_height, |item| item.height)
                .max(self.slot_height);
            let fraction = ((actual_top - actual_row_top) / actual_row_height).clamp(0., 1.);
            (self.slot_height * row as f32 + self.slot_height * fraction).clamp(px(0.), logical_max)
        };
        point(self.handle.offset().x, -logical_top)
    }

    fn set_offset(&self, offset: Point<Pixels>) {
        self.measurement_anchor.set(None);
        self.scrollbar_measurement_pending.set(true);
        let viewport_height = self.handle.bounds().size.height;
        let logical_height = self.slot_height * self.item_count as f32;
        let logical_max = (logical_height - viewport_height).max(px(0.));
        let requested_top = (-offset.y).clamp(px(0.), logical_max);
        let actual_top = if requested_top >= logical_max - px(0.5) {
            self.handle.max_offset().y.max(px(0.))
        } else {
            let row = (requested_top / self.slot_height).floor().max(0.) as usize;
            let row = row.min(self.item_count.saturating_sub(1));
            let logical_row_top = self.slot_height * row as f32;
            let fraction = ((requested_top - logical_row_top) / self.slot_height).clamp(0., 1.);
            let corrections = self.height_corrections.borrow();
            let actual_row_top = prefix_height_for(self.slot_height, &corrections, row);
            let actual_row_height = self
                .item_sizes
                .borrow()
                .get(row)
                .map_or(self.slot_height, |item| item.height)
                .max(self.slot_height);
            actual_row_top + actual_row_height * fraction
        };
        self.handle
            .set_offset(point(self.handle.offset().x, -actual_top));
    }

    fn content_size(&self) -> Size<Pixels> {
        size(
            self.handle.bounds().size.width,
            self.slot_height * self.item_count as f32,
        )
    }
}

fn prefix_height_for(
    base_height: Pixels,
    corrections: &[(usize, Pixels)],
    row_ix: usize,
) -> Pixels {
    let base = base_height * row_ix as f32;
    let correction_ix = corrections.partition_point(|(measured_row, _)| *measured_row < row_ix);
    base + correction_ix
        .checked_sub(1)
        .and_then(|ix| corrections.get(ix).map(|(_, correction)| *correction))
        .unwrap_or(px(0.))
}

fn row_for_absolute_y(
    count: usize,
    base_height: Pixels,
    corrections: &[(usize, Pixels)],
    target: Pixels,
) -> usize {
    let mut low = 0usize;
    let mut high = count;
    while low < high {
        let middle = low + (high - low) / 2;
        if prefix_height_for(base_height, corrections, middle.saturating_add(1)) > target {
            high = middle;
        } else {
            low = middle.saturating_add(1);
        }
    }
    low.min(count.saturating_sub(1))
}

fn centered_scroll_top(
    row_top: Pixels,
    row_height: Pixels,
    viewport_height: Pixels,
    max_top: Pixels,
) -> Pixels {
    (row_top + row_height / 2. - viewport_height / 2.).clamp(px(0.), max_top.max(px(0.)))
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

#[cfg(test)]
mod scroll_position_tests {
    use super::*;

    fn wrapped_layout_key_for_test() -> WrappedLayoutKey {
        WrappedLayoutKey {
            content_revision: 1,
            width: px(640.),
            rem_size: px(16.),
            font_family: "Consolas".into(),
            font_size: 13,
            base_height: px(19.),
            horizontal_padding: px(8.),
        }
    }

    fn assert_wrapped_layout_change_invalidates(update: impl FnOnce(&mut WrappedLayoutKey)) {
        let state = WrappedListState::<usize>::default();
        let current = wrapped_layout_key_for_test();
        assert!(state.invalidate_for_layout(current.clone()));
        state.prime_measured_heights(2, current.base_height, [(0, px(57.))]);
        assert_eq!(state.item_sizes.borrow()[0].height, px(57.));

        let mut next = current;
        update(&mut next);
        assert!(state.invalidate_for_layout(next));
        assert!(state.item_sizes.borrow().is_empty());
    }

    #[test]
    fn centers_a_row_when_the_viewport_has_room_on_both_sides() {
        assert_eq!(
            centered_scroll_top(px(400.), px(20.), px(200.), px(800.)),
            px(310.)
        );
    }

    #[test]
    fn keeps_centering_within_the_scrollable_edges() {
        assert_eq!(
            centered_scroll_top(px(20.), px(20.), px(200.), px(800.)),
            px(0.)
        );
        assert_eq!(
            centered_scroll_top(px(920.), px(20.), px(200.), px(820.)),
            px(820.)
        );
    }

    #[test]
    fn management_dialog_is_centered_in_the_viewport() {
        assert_eq!(centered_dialog_margin_top(px(900.), px(640.)), px(130.));
    }

    #[test]
    fn management_dialog_centering_clamps_small_viewports() {
        assert_eq!(centered_dialog_margin_top(px(480.), px(640.)), px(0.));
    }

    #[test]
    fn viewport_anchor_prefers_a_visible_selected_row() {
        assert_eq!(
            viewport_anchor_row(100, 20, Some(24), |row_ix| (20..30).contains(&row_ix)),
            24
        );
    }

    #[test]
    fn viewport_anchor_falls_back_to_the_first_visible_row() {
        assert_eq!(
            viewport_anchor_row(100, 20, Some(60), |row_ix| (20..30).contains(&row_ix)),
            20
        );
    }

    #[test]
    fn row_visibility_uses_the_row_and_viewport_edges() {
        assert!(row_intersects_viewport(px(-5.), px(20.), px(200.)));
        assert!(row_intersects_viewport(px(199.), px(20.), px(200.)));
        assert!(!row_intersects_viewport(px(-20.), px(20.), px(200.)));
        assert!(!row_intersects_viewport(px(200.), px(20.), px(200.)));
    }

    #[test]
    fn text_selection_origin_must_be_inside_a_log_region() {
        let log = Bounds::new(point(px(10.), px(20.)), size(px(300.), px(200.)));
        let results = Bounds::new(point(px(10.), px(240.)), size(px(300.), px(120.)));

        assert!(point_in_text_selection_regions(
            point(px(50.), px(80.)),
            [log, results]
        ));
        assert!(point_in_text_selection_regions(
            point(px(50.), px(300.)),
            [log, results]
        ));
        assert!(!point_in_text_selection_regions(
            point(px(5.), px(80.)),
            [log, results]
        ));
        assert!(!point_in_text_selection_regions(
            point(px(50.), px(380.)),
            [log, results]
        ));
    }

    #[test]
    fn row_height_lands_on_whole_device_pixels() {
        for scale_factor in [1., 1.25, 1.5, 1.75, 2.] {
            let snapped = snap_to_device_pixels(px(27.), scale_factor);
            let device_pixels = snapped.as_f32() * scale_factor;
            assert!(
                (device_pixels - device_pixels.round()).abs() < 1e-3,
                "{scale_factor} scale left {device_pixels} device pixels"
            );
            assert!((snapped.as_f32() - 27.).abs() <= 0.5 / scale_factor + 1e-3);
        }
    }

    #[test]
    fn snapped_row_height_keeps_every_row_pitch_equal() {
        let snapped = snap_to_device_pixels(px(27.), 1.25);
        for row_ix in 0..64 {
            let top = (snapped * row_ix as f32).as_f32() * 1.25;
            assert!(
                (top - top.round()).abs() < 1e-3,
                "row {row_ix} landed at {top}"
            );
        }
    }

    #[test]
    fn snapping_keeps_the_value_when_the_scale_factor_is_unusable() {
        assert_eq!(snap_to_device_pixels(px(27.), 0.), px(27.));
        assert_eq!(snap_to_device_pixels(px(27.), f32::NAN), px(27.));
    }

    #[test]
    fn wrapped_measurement_range_covers_the_first_expanded_frame() {
        assert_eq!(
            wrapped_viewport_measurement_range(20, px(100.), px(20.), 100),
            18..27
        );
        assert_eq!(
            wrapped_viewport_measurement_range(0, px(0.), px(20.), 2),
            0..2
        );
    }

    #[test]
    fn wrapped_measurements_are_invalidated_by_every_layout_dependency() {
        assert_wrapped_layout_change_invalidates(|key| key.content_revision += 1);
        assert_wrapped_layout_change_invalidates(|key| key.width += px(1.));
        assert_wrapped_layout_change_invalidates(|key| key.rem_size += px(1.));
        assert_wrapped_layout_change_invalidates(|key| key.font_family = "JetBrains Mono".into());
        assert_wrapped_layout_change_invalidates(|key| key.font_size += 1);
        assert_wrapped_layout_change_invalidates(|key| key.base_height += px(1.));
        assert_wrapped_layout_change_invalidates(|key| key.horizontal_padding += px(1.));
    }

    #[test]
    fn subpixel_width_noise_keeps_wrapped_measurements() {
        let state = WrappedListState::<usize>::default();
        let current = wrapped_layout_key_for_test();
        assert!(state.invalidate_for_layout(current.clone()));
        state.prime_measured_heights(1, current.base_height, [(0, px(57.))]);

        let mut next = current;
        next.width += px(0.25);
        assert!(!state.invalidate_for_layout(next));
        assert_eq!(state.item_sizes.borrow()[0].height, px(57.));
    }

    #[test]
    fn positions_a_wrapped_row_top_at_the_requested_viewport_y() {
        let state = WrappedListState::<usize>::default();
        state.sizes(100, px(20.));

        state.scroll_row_to_viewport_y(40, px(7.));

        assert_eq!(
            state.prefix_height(40) + state.scroll_handle.offset().y,
            px(7.)
        );
        assert_eq!(
            state.measurement_anchor.get(),
            Some(RowViewportPosition {
                row_ix: 40,
                viewport_y: px(7.),
            })
        );
    }

    #[test]
    fn measured_heights_reapply_the_exact_row_top_position() {
        let state = WrappedListState::<usize>::default();
        state.sizes(100, px(20.));
        state.scroll_row_to_viewport_y(40, px(-5.));

        assert!(state.queue_measured_height(10, px(60.), px(20.)));
        state.sizes(100, px(20.));

        assert_eq!(
            state.prefix_height(40) + state.scroll_handle.offset().y,
            px(-5.)
        );
    }

    #[test]
    fn mode_switch_uses_a_fresh_wrapped_scroll_owner() {
        let mut state = WrappedListState::<usize>::default();
        let previous = state.scroll_handle.base_handle().clone();
        previous.set_offset(point(px(0.), px(-300.)));
        state
            .scroll_handle
            .scroll_to_item(80, ScrollStrategy::Center);

        state.reset_scroll_for_mode_switch();
        state.sizes(100, px(20.));
        state.scroll_row_to_viewport_y(40, px(7.));

        assert_eq!(previous.offset().y, px(-300.));
        assert_eq!(
            state.prefix_height(40) + state.scroll_handle.offset().y,
            px(7.)
        );
    }

    #[test]
    fn viewport_routes_hit_testing_to_the_active_geometry_backend() {
        let fixed_bounds = Rc::new(RefCell::new(BTreeMap::from([
            (
                2,
                Bounds::new(point(px(0.), px(0.)), size(px(100.), px(20.))),
            ),
            (
                3,
                Bounds::new(point(px(0.), px(20.)), size(px(100.), px(20.))),
            ),
        ])));
        let viewport =
            LogViewportState::<usize>::new(false, UniformListScrollHandle::new(), fixed_bounds);
        viewport.wrapped_row_bounds().borrow_mut().insert(
            7,
            Bounds::new(point(px(0.), px(0.)), size(px(100.), px(20.))),
        );

        assert_eq!(viewport.row_at_position(point(px(4.), px(4.))), Some(2));
        assert_eq!(viewport.visible_row_edge(true), Some(3));

        viewport.set_word_wrap(true);

        assert_eq!(viewport.row_at_position(point(px(4.), px(4.))), Some(7));
        assert_eq!(viewport.visible_row_edge(false), Some(7));
    }

    #[test]
    fn viewport_layout_invalidation_preserves_the_visible_row_position() {
        let viewport =
            LogViewportState::<usize>::new(true, UniformListScrollHandle::new(), Rc::default());
        viewport.wrapped_sizes(100, px(20.));
        viewport
            .wrapped_scroll_handle()
            .set_offset(point(px(0.), px(-793.)));

        assert!(viewport.invalidate_wrapped_layout_preserving_position(
            wrapped_layout_key_for_test(),
            Some(40),
        ));
        viewport.wrapped_sizes(100, px(20.));

        assert_eq!(
            px(20.) * 40. + viewport.wrapped_scroll_handle().offset().y,
            px(7.)
        );
    }

    #[test]
    fn primed_heights_are_available_before_the_first_wrapped_frame() {
        let state = WrappedListState::<usize>::default();

        state.prime_measured_heights(100, px(20.), [(39, px(40.)), (40, px(60.)), (41, px(40.))]);
        state.scroll_row_to_viewport_y(40, px(7.));

        assert!(state.pending_heights.borrow().is_empty());
        assert_eq!(state.item_sizes.borrow()[40].height, px(60.));
        assert_eq!(
            state.prefix_height(40) + state.scroll_handle.offset().y,
            px(7.)
        );
    }

    #[test]
    fn measured_heights_follow_stable_rows_when_search_results_change() {
        let mut state = WrappedListState::<usize>::default();
        state.prime_measured_heights(3, px(20.), [(0, px(40.))]);
        assert!(state.queue_measured_height(1, px(60.), px(20.)));
        let previous_rows = [10usize, 20, 30];
        let next_rows = [20usize, 10, 40];
        let measured_heights =
            state.measured_heights_by_key(|row_ix| previous_rows.get(row_ix).copied());

        state.reset_with_remapped_heights(
            next_rows.len(),
            px(20.),
            measured_heights,
            |source_row| next_rows.iter().position(|row| row == source_row),
        );

        assert!(state.pending_heights.borrow().is_empty());
        assert_eq!(state.item_sizes.borrow()[0].height, px(60.));
        assert_eq!(state.item_sizes.borrow()[1].height, px(40.));
        assert_eq!(state.item_sizes.borrow()[2].height, px(20.));
    }

    #[test]
    fn matches_only_keeps_an_empty_match_set_empty() {
        let rows = compute_result_rows(ResultMode::MatchesOnly, None, &BTreeSet::from([2, 7]));

        assert!(rows.is_empty());
    }

    #[test]
    fn matches_and_marks_shows_marks_when_the_match_set_is_empty() {
        let rows = compute_result_rows(ResultMode::MatchesAndMarks, None, &BTreeSet::from([2, 7]));

        assert_eq!(rows.iter().collect::<Vec<_>>(), vec![2, 7]);
    }

    #[test]
    fn result_modes_report_whether_they_include_marks() {
        assert!(ResultMode::MatchesAndMarks.includes_marks());
        assert!(ResultMode::MarksOnly.includes_marks());
        assert!(!ResultMode::MatchesOnly.includes_marks());
    }

    #[test]
    fn global_search_scopes_own_their_word_wrap_state() {
        assert!(!SearchScope::CurrentFile.owns_global_word_wrap());
        assert!(SearchScope::AllOpenFiles.owns_global_word_wrap());
        assert!(SearchScope::Directory.owns_global_word_wrap());
    }

    #[test]
    fn restored_current_result_reapplies_the_domain_selection() {
        let rows = [4, 10, 42].into_iter().collect();
        let document = Arc::new(LogDocument::placeholder("restore-selection.log"));
        let delegate = LogTableDelegate::projected(1, document, rows);

        assert_eq!(delegate.settle_table_selection(1), Some(10));
        assert_eq!(delegate.selected_source_rows(), vec![10]);
    }

    #[test]
    fn restored_documents_wait_for_ready_before_replacing_the_loading_surface() {
        assert!(!should_upgrade_loading_document(
            DocumentLoadState::Opening,
            DocumentLoadState::Preview,
            true,
        ));
        assert!(should_upgrade_loading_document(
            DocumentLoadState::Opening,
            DocumentLoadState::Ready,
            true,
        ));
        assert!(should_upgrade_loading_document(
            DocumentLoadState::Opening,
            DocumentLoadState::Preview,
            false,
        ));
        assert!(should_upgrade_loading_document(
            DocumentLoadState::Preview,
            DocumentLoadState::Ready,
            false,
        ));
    }
}

impl<K> Default for WrappedListState<K> {
    fn default() -> Self {
        Self {
            item_sizes: Rc::new(RefCell::new(Rc::new(Vec::new()))),
            base_height: Rc::new(Cell::new(px(0.))),
            pending_heights: RefCell::new(BTreeMap::new()),
            measured_rows: RefCell::new(VecDeque::new()),
            height_corrections: Rc::new(RefCell::new(Vec::new())),
            scroll_handle: VirtualListScrollHandle::new(),
            text_selections: RefCell::default(),
            measurement_anchor: Rc::new(Cell::new(None)),
            scrollbar_measurement_pending: Rc::new(Cell::new(false)),
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
            item_sizes: self.item_sizes.clone(),
            height_corrections: self.height_corrections.clone(),
            measurement_anchor: self.measurement_anchor.clone(),
            scrollbar_measurement_pending: self.scrollbar_measurement_pending.clone(),
            item_count,
            slot_height,
        }
    }
    fn sizes(&self, count: usize, base_height: Pixels) -> Rc<Vec<Size<Pixels>>> {
        if self.item_sizes.borrow().len() != count || self.base_height.get() != base_height {
            self.base_height.set(base_height);
            *self.item_sizes.borrow_mut() = Rc::new(vec![size(px(0.), base_height); count]);
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
            let mut next = self.item_sizes.borrow().as_ref().clone();
            let mut measured_rows = self.measured_rows.borrow_mut();
            for (row_ix, height) in pending {
                let Some(item) = next.get_mut(row_ix) else {
                    continue;
                };
                item.height = height.max(base_height);
                if let Some(old_ix) = measured_rows.iter().position(|row| *row == row_ix) {
                    measured_rows.remove(old_ix);
                }
                measured_rows.push_back(row_ix);
            }
            while measured_rows.len() > WRAPPED_HEIGHT_CACHE_LIMIT {
                if let Some(evicted) = measured_rows.pop_front()
                    && let Some(item) = next.get_mut(evicted)
                {
                    item.height = base_height;
                }
            }
            let mut corrections = measured_rows
                .iter()
                .filter_map(|row_ix| {
                    next.get(*row_ix)
                        .map(|item| (*row_ix, item.height - base_height))
                })
                .collect::<Vec<_>>();
            corrections.sort_by_key(|(row_ix, _)| *row_ix);
            let mut cumulative = px(0.);
            for (_, correction) in &mut corrections {
                cumulative += *correction;
                *correction = cumulative;
            }
            *self.height_corrections.borrow_mut() = corrections;
            *self.item_sizes.borrow_mut() = Rc::new(next);
            if explicit_anchor.is_none() && was_at_bottom {
                self.scroll_handle.scroll_to_bottom();
            } else {
                self.set_row_viewport_y(anchor.row_ix, anchor.viewport_y);
            }
        }
        self.item_sizes.borrow().clone()
    }

    fn queue_measured_height(&self, row_ix: usize, height: Pixels, base_height: Pixels) -> bool {
        let height = height.max(base_height);
        let current_height = self
            .pending_heights
            .borrow()
            .get(&row_ix)
            .copied()
            .or_else(|| self.item_sizes.borrow().get(row_ix).map(|size| size.height));
        let Some(current_height) = current_height else {
            return false;
        };
        if (current_height - height).abs() < px(0.5) {
            return false;
        }
        self.pending_heights.borrow_mut().insert(row_ix, height);
        true
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
        let item_sizes = self.item_sizes.borrow();
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
                    .or_else(|| item_sizes.get(row_ix).map(|size| size.height))?;
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
        *self.item_sizes.borrow_mut() = Rc::new(Vec::new());
        self.base_height.set(px(0.));
        self.pending_heights.borrow_mut().clear();
        self.measured_rows.borrow_mut().clear();
        self.height_corrections.borrow_mut().clear();
        self.text_selections.borrow_mut().clear();
        self.row_bounds.borrow_mut().clear();
        self.scrollbar_measurement_pending.set(false);
    }

    fn invalidate_for_layout(&self, key: WrappedLayoutKey) -> bool {
        if !self.needs_layout_invalidation(&key) {
            return false;
        }
        self.layout_key.replace(Some(key));
        *self.item_sizes.borrow_mut() = Rc::new(Vec::new());
        self.base_height.set(px(0.));
        self.pending_heights.borrow_mut().clear();
        self.measured_rows.borrow_mut().clear();
        self.height_corrections.borrow_mut().clear();
        self.text_selections.borrow_mut().clear();
        self.row_bounds.borrow_mut().clear();
        self.scrollbar_measurement_pending.set(false);
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
        let count = self.item_sizes.borrow().len();
        if count == 0 {
            return None;
        }
        let top = (-self.scroll_handle.offset().y).max(px(0.));
        let viewport_height = self.scroll_handle.bounds().size.height;
        let first = self.first_visible_row();
        let row_ix = viewport_anchor_row(count, first, preferred_row, |row_ix| {
            let row_top = self.prefix_height(row_ix);
            let row_height = self
                .item_sizes
                .borrow()
                .get(row_ix)
                .map_or(self.base_height.get(), |item| item.height);
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
        self.scroll_handle = VirtualListScrollHandle::new();
        self.clear_measurement_anchor();
        self.scrollbar_measurement_pending.set(false);
    }

    fn take_scrollbar_measurement_request(&self) -> bool {
        self.scrollbar_measurement_pending.replace(false)
    }

    fn clear_measurement_anchor(&self) {
        self.measurement_anchor.set(None);
    }

    fn first_visible_row(&self) -> usize {
        let top = -self.scroll_handle.offset().y;
        let count = self.item_sizes.borrow().len();
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
        let item_sizes = self.item_sizes.borrow();
        let Some(row_height) = item_sizes.get(row_ix).map(|item| item.height) else {
            drop(item_sizes);
            self.scroll_handle
                .scroll_to_item(row_ix, ScrollStrategy::Center);
            return;
        };
        let viewport_height = self.scroll_handle.bounds().size.height;
        if viewport_height <= px(0.) {
            drop(item_sizes);
            self.scroll_handle
                .scroll_to_item(row_ix, ScrollStrategy::Center);
            return;
        }
        let row_top = self.prefix_height(row_ix);
        let content_height = self.prefix_height(item_sizes.len());
        let top = centered_scroll_top(
            row_top,
            row_height,
            viewport_height,
            content_height - viewport_height,
        );
        drop(item_sizes);
        self.restore_viewport(row_ix, row_top - top, false);
    }

    fn apply_wheel_line_scroll(&self, delta_y: Pixels, row_count: usize, line_count: usize) {
        if row_count == 0 || delta_y == px(0.) {
            return;
        }

        let current = self.scroll_handle.offset();
        let current_top = (-current.y).clamp(px(0.), self.scroll_handle.max_offset().y.max(px(0.)));
        let current_row = row_for_absolute_y(
            row_count,
            self.base_height.get(),
            &self.height_corrections.borrow(),
            current_top,
        );
        let target_row = if delta_y < px(0.) {
            current_row.saturating_add(line_count)
        } else {
            current_row.saturating_sub(line_count)
        }
        .min(row_count.saturating_sub(1));
        self.place_row_at_top(target_row);
    }

    fn scale_native_wheel_scroll(&self, delta_y: Pixels, scale: f32) {
        self.clear_measurement_anchor();
        let current = self.scroll_handle.offset();
        let max_y = self.scroll_handle.max_offset().y.max(px(0.));
        let target_y = (current.y + delta_y * scale).clamp(-max_y, px(0.));
        self.scroll_handle.set_offset(point(current.x, target_y));
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

fn apply_uniform_wheel_line_scroll(
    handle: &gpui::UniformListScrollHandle,
    delta_y: Pixels,
    row_count: usize,
    row_height: Pixels,
    line_count: usize,
) {
    if row_count == 0 || delta_y == px(0.) || row_height <= px(0.) {
        return;
    }

    let base_handle = handle.0.borrow().base_handle.clone();
    let current = base_handle.offset();
    let max_y = base_handle.max_offset().y.max(px(0.));
    let current_top = (-current.y).clamp(px(0.), max_y);
    let current_row = row_for_absolute_y(row_count, row_height, &[], current_top);
    let target_row = if delta_y < px(0.) {
        current_row.saturating_add(line_count)
    } else {
        current_row.saturating_sub(line_count)
    }
    .min(row_count.saturating_sub(1));
    let target_top = (row_height * target_row as f32).min(max_y);
    base_handle.set_offset(point(current.x, -target_top));
}

fn scale_uniform_wheel_scroll(handle: &gpui::UniformListScrollHandle, delta_y: Pixels, scale: f32) {
    let base_handle = handle.0.borrow().base_handle.clone();
    let current = base_handle.offset();
    let max_y = base_handle.max_offset().y.max(px(0.));
    let target_y = (current.y + delta_y * scale).clamp(-max_y, px(0.));
    base_handle.set_offset(point(current.x, target_y));
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

    fn select_and_center_log_row(&mut self, row_ix: usize, cx: &mut App) {
        self.log_table.update(cx, |table, cx| {
            table.set_active_log_row(row_ix, cx);
        });
        self.log_viewport.center_row(row_ix);
    }

    fn refresh_result_rows(&mut self, cx: &mut App) {
        let word_wrap = self.result_viewport.is_wrapped();
        let row_height = if word_wrap && self.result_viewport.wrapped_base_height() > px(0.) {
            self.result_viewport.wrapped_base_height()
        } else {
            self.result_table
                .read(cx)
                .vertical_scroll_handle
                .0
                .borrow()
                .last_item_size
                .map(|size| size.item.height)
                .unwrap_or_else(|| px(self.result_table.read(cx).delegate().log_font_size() as f32))
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
        let result_rows = self.compute_result_rows();
        self.restoring_result_selection = true;
        let active_restored = self.result_table.update(cx, |table, cx| {
            if self.auto_follow {
                table.delegate().set_active_log_row(None);
            }
            table
                .delegate_mut()
                .set_matched_rows(self.search_result.line_indices.clone());
            table.delegate_mut().set_row_projection(result_rows);
            let active_restored = table.sync_active_log_row(cx);
            table.refresh_log_rows(cx);
            active_restored
        });
        if !active_restored {
            self.restoring_result_selection = false;
        }
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

    fn selected_source_rows(&self, cx: &App) -> Vec<usize> {
        let table = match self.selection_table {
            SelectionTable::Log => &self.log_table,
            SelectionTable::Results => &self.result_table,
        };
        let state = table.read(cx);
        let mut rows = state.delegate().selected_source_rows();
        if rows.is_empty()
            && let Some(source_row) = state
                .active_log_row()
                .and_then(|row_ix| state.delegate().source_row(row_ix))
        {
            rows.push(source_row);
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

struct CompletedGlobalSearch {
    scope: SearchScope,
    query: SearchQuery,
    results: GlobalSearchResults,
    matcher: Option<SearchMatcher>,
    viewport_anchor: Option<RowViewportAnchor<LogRowKey>>,
}

pub struct Workspace {
    primary_window: bool,
    focus_handle: FocusHandle,
    status_surface: Entity<WorkspaceStatusSurface>,
    query: Entity<InputState>,
    search_history: Vec<String>,
    predefined_filters: Vec<PredefinedFilter>,
    cloud: CloudController,
    updates: UpdateController,
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
    transient_paths: BTreeSet<PathBuf>,
    pending_tab_moves: BTreeSet<u64>,
    documents: Vec<DocumentTab>,
    tabs: Vec<WorkspaceTabId>,
    active_tab_id: WorkspaceTabId,
    active_ix: Option<usize>,
    document_tab_scroll: ScrollHandle,
    pending_document_tab_reveal: Cell<Option<u64>>,
    pending_directory_result_jump: Option<(PathBuf, usize)>,
    next_document_id: u64,
    next_new_tab_id: u64,
    case_sensitive: bool,
    regex: bool,
    activity: Activity,
    selected_source_row: Option<usize>,
    row_drag_bounds: BTreeMap<(u64, WrappedRegion), Bounds<Pixels>>,
    row_drag_selection: Option<RowDragSelection>,
    row_drag_frame_scheduled: bool,
    open_task: Option<Task<()>>,
    pending_external_paths: Vec<PathBuf>,
    searches: SearchController,
    result_export_task: Option<Task<()>>,
    result_export_operation: Option<ResultExportOperation>,
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
    _update_client_bootstrap_task: Task<()>,
    _cloud_client_bootstrap_task: Task<()>,
    _automatic_update_task: Task<()>,
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

impl Workspace {
    pub(crate) fn init_window_registry(cx: &mut App) {
        cx.set_global(WorkspaceWindowRegistry::default());
    }

    pub(crate) fn open_external_paths_in_last_active_window(
        paths: &[PathBuf],
        cx: &mut App,
    ) -> bool {
        if paths.is_empty() {
            return false;
        }
        let candidates = cx
            .global::<WorkspaceWindowRegistry>()
            .windows_by_recent_focus();
        for candidate in candidates {
            let workspace = candidate.workspace.clone();
            let paths = paths.to_vec();
            if candidate
                .window
                .update(cx, move |_, window, cx| {
                    workspace.update(cx, |workspace, cx| {
                        workspace.enqueue_external_paths(paths, window, cx)
                    });
                    cx.activate(true);
                    window.activate_window();
                })
                .is_ok()
            {
                return true;
            }
        }
        false
    }

    pub(crate) fn register_window(workspace: &Entity<Self>, window: &mut Window, cx: &mut App) {
        let window_handle = window.window_handle();
        let registered_workspace = workspace.clone();
        cx.update_global::<WorkspaceWindowRegistry, _>(|registry, _| {
            registry.register(window_handle, registered_workspace)
        });
        workspace.update(cx, |workspace, cx| {
            let activation_subscription =
                cx.observe_window_activation(window, |workspace, window, cx| {
                    if window.is_window_active() {
                        let window_handle = window.window_handle();
                        cx.update_global::<WorkspaceWindowRegistry, _>(|registry, _| {
                            registry.mark_focused(window_handle)
                        });
                        workspace.restore_input_focus(window, cx);
                        workspace.start_file_watch(window, cx);
                        cx.notify();
                    } else {
                        TextSelection::end(window, cx);
                        workspace.end_all_row_drag_selection(window, cx);
                        workspace.release_input_focus(window, cx);
                        workspace.file_watch_task = None;
                    }
                });
            let appearance_subscription =
                cx.observe_window_appearance(window, |workspace, window, cx| {
                    if workspace.persistence.store.is_some()
                        && workspace.app_settings.theme_preference == ThemePreference::System
                    {
                        Self::apply_theme_preference(ThemePreference::System, window, cx);
                    }
                });
            workspace.subscriptions.push(activation_subscription);
            workspace.subscriptions.push(appearance_subscription);
            // 新窗口打开后不会再收到一次激活通知，跟随轮询要在这里起头。
            workspace.start_file_watch(window, cx);
        });
    }

    /// 自动跟随只在窗口激活期间轮询：后台窗口看不到新内容，而定时唤醒会让进程一直占用 CPU。
    fn start_file_watch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.file_watch_task.is_some() {
            return;
        }
        self.file_watch_task = Some(cx.spawn_in(window, async move |this, cx| {
            loop {
                Self::poll_auto_follow(&this, cx).await;
                cx.background_executor().timer(FILE_WATCH_INTERVAL).await;
            }
        }));
    }

    async fn poll_auto_follow(this: &WeakEntity<Self>, cx: &mut AsyncWindowContext) {
        let candidate = this
            .update_in(cx, |this, _, cx| this.auto_follow_candidate(cx))
            .ok()
            .flatten();
        let Some((document_id, path, indexed_size, indexed_modified)) = candidate else {
            return;
        };
        let metadata = cx
            .background_spawn(async move {
                std::fs::metadata(path).map(|metadata| (metadata.len(), metadata.modified().ok()))
            })
            .await;
        let Ok((current_size, current_modified)) = metadata else {
            return;
        };
        if current_size == indexed_size && current_modified == indexed_modified {
            return;
        }

        _ = this.update_in(cx, |this, window, cx| {
            let remains_stale = this.documents.iter().any(|tab| {
                tab.id == document_id
                    && tab.auto_follow
                    && (tab.document.metadata().file_size != current_size
                        || tab.document.metadata().modified != current_modified)
            });
            if remains_stale {
                this.reload_document(document_id, true, ReloadStrategy::ExtendAppend, window, cx);
            }
        });
    }

    /// 窗口失活时把焦点从输入框收回：`gpui-component` 的光标闪烁定时器只在输入框失焦时停下，
    /// 否则后台窗口每 500ms 都会因为光标翻转而重绘一次。
    fn release_input_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let inputs = [
            self.query.focus_handle(cx),
            self.quick_find.query.focus_handle(cx),
        ];
        let Some(focused) = inputs.into_iter().find(|handle| handle.is_focused(window)) else {
            return;
        };
        self.deactivated_input_focus = Some(focused);
        self.focus_handle.focus(window, cx);
    }

    fn restore_input_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(focused) = self.deactivated_input_focus.take() else {
            return;
        };
        focused.focus(window, cx);
    }

    fn apply_theme_preference(
        preference: ThemePreference,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mode = match preference {
            ThemePreference::Light => ThemeMode::Light,
            ThemePreference::Dark => ThemeMode::Dark,
            ThemePreference::System => window.appearance().into(),
        };
        ui_theme::apply_product_theme(mode, cx);

        cx.refresh_windows();
    }

    pub(crate) fn unregister_window(window_id: WindowId, cx: &mut App) {
        let (workspace, target_to_clear) =
            cx.update_global::<WorkspaceWindowRegistry, _>(|registry, _| {
                let mut target_to_clear = None;
                if registry
                    .cross_window_tab_drag
                    .as_ref()
                    .is_some_and(|drag| drag.source_window.window_id() == window_id)
                {
                    target_to_clear = registry
                        .cross_window_tab_drag
                        .take()
                        .and_then(|drag| drag.target);
                } else if let Some(drag) = &mut registry.cross_window_tab_drag
                    && drag
                        .target
                        .as_ref()
                        .is_some_and(|target| target.window.window_id() == window_id)
                {
                    drag.target = None;
                }
                (registry.unregister(window_id), target_to_clear)
            });
        if let Some(target_to_clear) = target_to_clear {
            Self::set_cross_window_drop_visual(&target_to_clear, None, cx);
        }
        let Some(workspace) = workspace else {
            return;
        };
        let snapshot = workspace.update(cx, |workspace, cx| workspace.take_quit_snapshot(cx));
        let background_executor = cx.background_executor().clone();
        let task = cx.spawn(async move |_| {
            for task in snapshot.state_tasks {
                task.await;
            }
            if let Some(task) = snapshot.workspace_order_task {
                task.await;
            }
            let result = background_executor
                .spawn(async move {
                    let store = match snapshot.store {
                        Some(store) => store,
                        None => Arc::new(StateStore::open_default()?),
                    };
                    if let Some(predefined_filters) = snapshot.predefined_filters {
                        save_predefined_filters_if_current(
                            &store,
                            &predefined_filters,
                            snapshot.predefined_filters_revision,
                        )?;
                    }
                    store.save_sessions(&snapshot.sessions)?;
                    if let Some(search_state) = snapshot.search_state {
                        store.save_workspace_search_state(&search_state)?;
                    }
                    Ok::<_, anyhow::Error>(())
                })
                .await;
            if let Err(error) = result {
                log::error!("关闭窗口时文件会话未能保存：{error}");
            }
        });
        cx.update_global::<WorkspaceWindowRegistry, _>(|registry, _| {
            registry.closed_flush_tasks.push(task)
        });
    }

    fn set_cross_window_drop_visual(
        target: &CrossWindowDropTarget,
        mode: Option<TabTransferMode>,
        cx: &mut App,
    ) {
        target.workspace.update(cx, |workspace, cx| {
            let target_ix = mode.map(|_| target.target_ix);
            let visible = mode.is_some();
            if workspace.cross_window_drop_ix != target_ix
                || workspace.file_drop_visible != visible
                || workspace.file_drop_tab_transfer != mode
            {
                workspace.cross_window_drop_ix = target_ix;
                workspace.file_drop_visible = visible;
                workspace.file_drop_tab_transfer = mode;
                cx.notify();
            }
        });
    }

    fn track_cross_window_tab_drag(
        dragged: &DraggedTab,
        event: &DragMoveEvent<DraggedTab>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(document_id) = dragged.tab_id.document_id() else {
            return;
        };
        let source_window = window.window_handle();
        let source_bounds = window.bounds();
        let source_scale = window.scale_factor();
        let screen_x =
            (source_bounds.origin.x.as_f32() + event.event.position.x.as_f32()) * source_scale;
        let screen_y =
            (source_bounds.origin.y.as_f32() + event.event.position.y.as_f32()) * source_scale;
        let mut candidates = cx
            .global::<WorkspaceWindowRegistry>()
            .windows
            .iter()
            .filter(|entry| entry.window != source_window)
            .cloned()
            .collect::<Vec<_>>();
        candidates.sort_by_key(|entry| std::cmp::Reverse(entry.focus_order));

        let mut next_target = None;
        let mut over_workspace_window = false;
        for candidate in candidates {
            let target_window = candidate.window;
            let target_workspace = candidate.workspace.clone();
            let hit = target_window
                .update(cx, |_, target_window, cx| {
                    let bounds = target_window.bounds();
                    let target_scale = target_window.scale_factor();
                    let left = bounds.origin.x.as_f32() * target_scale;
                    let top = bounds.origin.y.as_f32() * target_scale;
                    let right = (bounds.origin.x + bounds.size.width).as_f32() * target_scale;
                    let bottom = (bounds.origin.y + bounds.size.height).as_f32() * target_scale;
                    if screen_x < left || screen_x >= right || screen_y < top || screen_y >= bottom
                    {
                        return (false, None);
                    }
                    let local_position = Point {
                        x: px((screen_x - left) / target_scale),
                        y: px((screen_y - top) / target_scale),
                    };
                    let workspace = target_workspace.read(cx);
                    (
                        true,
                        workspace
                            .tab_drop_layout
                            .borrow()
                            .drop_index(local_position),
                    )
                })
                .unwrap_or((false, None));
            let (inside_window, target_ix) = hit;
            if inside_window {
                over_workspace_window = true;
            }
            if let Some(target_ix) = target_ix {
                next_target = Some(CrossWindowDropTarget {
                    window: target_window,
                    workspace: candidate.workspace,
                    target_ix,
                });
                break;
            }
            if inside_window {
                break;
            }
        }

        let mode = if window.modifiers().control {
            TabTransferMode::Copy
        } else {
            TabTransferMode::Move
        };
        let (previous_target, changed) =
            cx.update_global::<WorkspaceWindowRegistry, _>(|registry, _| {
                let unchanged = registry.cross_window_tab_drag.as_ref().is_some_and(|drag| {
                    drag.source_window == source_window
                        && drag.document_id == document_id
                        && drag.mode == mode
                        && drag.over_workspace_window == over_workspace_window
                        && match (&drag.target, &next_target) {
                            (Some(left), Some(right)) => {
                                left.window == right.window && left.target_ix == right.target_ix
                            }
                            (None, None) => true,
                            _ => false,
                        }
                });
                if unchanged {
                    return (None, false);
                }
                let previous_target = registry
                    .cross_window_tab_drag
                    .take()
                    .and_then(|drag| drag.target);
                registry.cross_window_tab_drag = Some(CrossWindowTabDrag {
                    source_window,
                    source: dragged.source.clone(),
                    document_id,
                    mode,
                    target: next_target.clone(),
                    over_workspace_window,
                });
                (previous_target, true)
            });
        if !changed {
            return;
        }
        if let Some(previous_target) = previous_target {
            Self::set_cross_window_drop_visual(&previous_target, None, cx);
        }
        if let Some(next_target) = next_target {
            Self::set_cross_window_drop_visual(&next_target, Some(mode), cx);
        }
    }

    fn finish_cross_window_tab_drag(event: &MouseUpEvent, window: &mut Window, cx: &mut App) {
        let source_window = window.window_handle();
        let drag = cx.update_global::<WorkspaceWindowRegistry, _>(|registry, _| {
            registry
                .cross_window_tab_drag
                .as_ref()
                .is_some_and(|drag| drag.source_window == source_window)
                .then(|| registry.cross_window_tab_drag.take())
                .flatten()
        });
        let Some(drag) = drag else {
            return;
        };
        let mode = if window.modifiers().control {
            TabTransferMode::Copy
        } else {
            TabTransferMode::Move
        };
        let Some(source) = drag.source.upgrade() else {
            return;
        };
        if let Some(target) = drag.target {
            Self::set_cross_window_drop_visual(&target, None, cx);
            source.update(cx, |source, cx| {
                source.transfer_tab_to_window_target(
                    drag.document_id,
                    mode,
                    TabTransferTarget {
                        window: target.window,
                        workspace: target.workspace,
                        target_ix: Some(target.target_ix),
                    },
                    window,
                    cx,
                );
            });
            return;
        }

        let client_bounds = Bounds::new(Point::default(), window.bounds().size);
        if drag.over_workspace_window || client_bounds.contains(&event.position) {
            return;
        }
        let screen_position = window.bounds().origin + event.position;
        let (bounds, display_id) = Self::detached_window_placement(screen_position, window, cx);
        source.update(cx, |source, cx| {
            source.transfer_tab_to_new_window(
                drag.document_id,
                mode,
                Some((bounds, display_id)),
                window,
                cx,
            );
        });
    }

    fn detached_window_placement(
        screen_position: Point<Pixels>,
        window: &Window,
        cx: &App,
    ) -> (Bounds<Pixels>, Option<DisplayId>) {
        let display = cx
            .displays()
            .into_iter()
            .find(|display| display.bounds().contains(&screen_position))
            .or_else(|| window.display(cx));
        let Some(display) = display else {
            return (
                Bounds::new(
                    screen_position - point(px(180.), px(24.)),
                    size(px(1280.), px(800.)),
                ),
                None,
            );
        };
        let visible = display.visible_bounds();
        let window_size = size(px(1280.), px(800.)).min(&visible.size);
        let minimum_origin = visible.origin;
        let maximum_origin = point(
            visible.origin.x + visible.size.width - window_size.width,
            visible.origin.y + visible.size.height - window_size.height,
        );
        let preferred_origin = screen_position - point(px(180.), px(24.));
        let origin = point(
            preferred_origin.x.clamp(minimum_origin.x, maximum_origin.x),
            preferred_origin.y.clamp(minimum_origin.y, maximum_origin.y),
        );
        (Bounds::new(origin, window_size), Some(display.id()))
    }

    pub(crate) fn flush_all_on_quit(cx: &mut App) -> impl Future<Output = ()> + use<> {
        let (registered, closed_flush_tasks) =
            cx.update_global::<WorkspaceWindowRegistry, _>(|registry, _| {
                registry.cross_window_tab_drag = None;
                (
                    registry
                        .windows
                        .iter()
                        .map(|entry| entry.workspace.clone())
                        .collect::<Vec<_>>(),
                    std::mem::take(&mut registry.closed_flush_tasks),
                )
            });
        let snapshots = registered
            .into_iter()
            .map(|workspace| workspace.update(cx, |workspace, cx| workspace.take_quit_snapshot(cx)))
            .collect::<Vec<_>>();
        let background_executor = cx.background_executor().clone();

        async move {
            for task in closed_flush_tasks {
                task.await;
            }
            let mut store = None;
            let mut sessions = Vec::new();
            let mut session_paths = BTreeSet::new();
            let mut open_paths = Vec::new();
            let mut open_path_set = BTreeSet::new();
            let mut active_path = None;
            let mut search_state = None;
            let mut predefined_filters = None;

            for mut snapshot in snapshots {
                for task in snapshot.state_tasks {
                    task.await;
                }
                if let Some(task) = snapshot.workspace_order_task {
                    task.await;
                }
                store = store.or(snapshot.store.take());
                for (path, state) in snapshot.sessions {
                    if session_paths.insert(path.clone()) {
                        sessions.push((path, state));
                    }
                }
                for path in snapshot.open_paths {
                    if open_path_set.insert(path.clone()) {
                        open_paths.push(path);
                    }
                }
                if active_path.is_none()
                    && snapshot
                        .active_path
                        .as_ref()
                        .is_some_and(|path| open_path_set.contains(path))
                {
                    active_path = snapshot.active_path;
                }
                search_state = search_state.or(snapshot.search_state);
                if let Some(filters) = snapshot.predefined_filters {
                    predefined_filters = Some((snapshot.predefined_filters_revision, filters));
                }
            }

            let result = background_executor
                .spawn(async move {
                    let store = match store {
                        Some(store) => store,
                        None => Arc::new(StateStore::open_default()?),
                    };
                    store.save_workspace(&sessions, &open_paths, active_path.as_deref())?;
                    if let Some((revision, predefined_filters)) = predefined_filters {
                        save_predefined_filters_if_current(&store, &predefined_filters, revision)?;
                    }
                    if let Some(search_state) = search_state {
                        store.save_workspace_search_state(&search_state)?;
                    }
                    Ok::<_, anyhow::Error>(())
                })
                .await;
            if let Err(error) = result {
                log::error!("退出状态未能保存：{error}");
            }
        }
    }

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
                let wrapped_group_anchor = if word_wrap {
                    match row {
                        GlobalSearchRow::Group { document_id } => this
                            .global_viewport
                            .capture_wrapped_viewport_position(Some(*row_ix))
                            .map(|position| RowViewportAnchor {
                                key: LogRowKey::FileGroup { document_id },
                                viewport_y: position.viewport_y,
                                fallback_ix: position.row_ix,
                            }),
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
                match row {
                    GlobalSearchRow::Group { document_id } => {
                        if table.read(cx).delegate().group_has_results(document_id) {
                            table.update(cx, |table, cx| {
                                table.delegate_mut().toggle_group(document_id);
                                table.delegate().clear_row_selection();
                                table.clear_selection(cx);
                                table.refresh(cx);
                            });
                            if word_wrap {
                                let base_height = px((this.app_settings.log_font_size
                                    + this.app_settings.log_line_spacing)
                                    as f32);
                                this.prime_global_wrapped_group_toggle(
                                    wrapped_group_anchor,
                                    base_height,
                                    window,
                                    cx,
                                );
                            }
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
                this.schedule_workspace_search_state_save(window, cx);
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
                this.refresh_global_result_rows(cx);
                this.schedule_workspace_search_state_save(window, cx);
                cx.notify();
            },
        ));
        let focus_handle = cx.focus_handle().tab_stop(true);
        let focus_on_start = focus_handle.clone();
        window.defer(cx, move |window, cx| focus_on_start.focus(window, cx));
        let update_state = if cfg!(debug_assertions) {
            AppUpdateState::Unsupported
        } else {
            AppUpdateState::Idle
        };
        let update_client_bootstrap_task = cx.spawn_in(window, async move |this, cx| {
            if cfg!(debug_assertions) {
                return;
            }
            let client = cx
                .background_spawn(async move { UpdateClient::open_default() })
                .await;
            _ = this.update_in(cx, |this, _, cx| {
                match client {
                    Ok(client) => {
                        this.updates.client = Some(client);
                        this.updates.state = AppUpdateState::Idle;
                    }
                    Err(_) => this.updates.state = AppUpdateState::Error,
                }
                cx.notify();
            });
        });
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
                    .background_spawn(async move { vclogg_core::cleanup_index_cache(directory) })
                    .await
                {
                    log::warn!("索引缓存自动维护失败：{error:#}");
                }
            })
        });
        let automatic_update_task = cx.spawn_in(window, async move |this, cx| {
            if !primary_window || cfg!(debug_assertions) {
                return;
            }
            cx.background_executor()
                .timer(Duration::from_secs(15))
                .await;
            _ = this.update_in(cx, |this, window, cx| {
                if matches!(this.updates.state, AppUpdateState::Idle) {
                    this.check_for_updates(false, window, cx);
                }
            });
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
                        this.global_search.preferences = global_search_preferences;
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
                        this.global_table.update(cx, |table, cx| {
                            table.delegate_mut().set_appearance(&app_settings);
                            table.delegate_mut().set_word_boundary_characters(
                                app_settings.word_boundary_characters.clone(),
                            );
                            table
                                .delegate_mut()
                                .set_highlight_log_levels(app_settings.highlight_log_levels);
                            table.refresh(cx);
                        });
                        this.refresh_global_result_rows(cx);
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
                            .filter(|tab| !this.transient_paths.contains(tab.document.path()))
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
            updates: UpdateController::new(update_state),
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
            open_task: None,
            pending_external_paths: Vec::new(),
            searches: SearchController::default(),
            result_export_task: None,
            result_export_operation: None,
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
            _update_client_bootstrap_task: update_client_bootstrap_task,
            _cloud_client_bootstrap_task: cloud_client_bootstrap_task,
            _automatic_update_task: automatic_update_task,
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

    fn result_export_snapshot(&self, cx: &App) -> Option<ResultExport> {
        match self.global_search.scope {
            SearchScope::CurrentFile => {
                let tab = self.active_document()?;
                let rows = tab.result_rows(cx);
                (tab.load_state == DocumentLoadState::Ready && !rows.is_empty()).then(|| {
                    ResultExport::Single {
                        document: tab.document.clone(),
                        rows,
                    }
                })
            }
            SearchScope::AllOpenFiles => {
                let groups = self
                    .documents
                    .iter()
                    .filter(|tab| self.global_search.selected_documents.contains(&tab.id))
                    .filter_map(|tab| {
                        if tab.load_state != DocumentLoadState::Ready {
                            return None;
                        }
                        let result = self.global_search.results.get(&tab.id);
                        let rows = compute_result_rows(
                            self.global_search.result_mode,
                            result.map(|result| &result.search_result),
                            &tab.marked_rows,
                        );
                        (!rows.is_empty()).then(|| ExportGroup {
                            path: result
                                .map(|result| result.path.clone())
                                .unwrap_or_else(|| tab.document.path().to_path_buf()),
                            document: result
                                .map(|result| result.document.clone())
                                .unwrap_or_else(|| tab.document.clone()),
                            rows,
                        })
                    })
                    .collect::<Vec<_>>();
                (!groups.is_empty()).then(|| ResultExport::Global {
                    groups: groups.into(),
                })
            }
            SearchScope::Directory => {
                if self.global_search.result_scope != Some(SearchScope::Directory) {
                    return None;
                }
                let groups = self
                    .global_search
                    .results
                    .values()
                    .filter_map(|result| {
                        let rows = compute_result_rows(
                            self.global_search.result_mode,
                            Some(&result.search_result),
                            &BTreeSet::new(),
                        );
                        (!rows.is_empty()).then(|| ExportGroup {
                            path: result.path.clone(),
                            document: result.document.clone(),
                            rows,
                        })
                    })
                    .collect::<Vec<_>>();
                (!groups.is_empty()).then(|| ResultExport::Global {
                    groups: groups.into(),
                })
            }
        }
    }

    fn result_export_suggested_name(&self) -> String {
        match self.global_search.scope {
            SearchScope::CurrentFile => self
                .active_document()
                .and_then(|tab| tab.document.path().file_stem())
                .map(|stem| format!("{}-results.log", stem.to_string_lossy()))
                .unwrap_or_else(|| "search-results.log".to_string()),
            SearchScope::AllOpenFiles => "global-search-results.log".to_string(),
            SearchScope::Directory => "directory-search-results.log".to_string(),
        }
    }

    fn open_search_results_in_new_tab_action(
        &mut self,
        _: &OpenSearchResultsInNewTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_results_in_new_tab(window, cx);
    }

    fn merge_search_results_in_new_tab_action(
        &mut self,
        _: &MergeSearchResultsInNewTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_timestamp_merged_results(window, cx);
    }

    fn save_search_results_to_file_action(
        &mut self,
        _: &SaveSearchResultsToFile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.save_results_as(window, cx);
    }

    fn open_results_in_new_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.result_export_task.is_some() || self.open_task.is_some() {
            return;
        }
        let Some(export) = self.result_export_snapshot(cx) else {
            return;
        };
        self.result_export_operation = Some(ResultExportOperation::OpenInNewTab);
        cx.notify();
        self.result_export_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let row_count = export.row_count();
                    let path = result_export::save_to_unique_temp(&export)?;
                    Ok::<_, anyhow::Error>((path, row_count))
                })
                .await;
            _ = this.update_in(cx, |this, window, cx| {
                this.result_export_task = None;
                this.result_export_operation = None;
                match result {
                    Ok((path, row_count)) if this.open_task.is_none() => {
                        this.transient_paths.insert(path.clone());
                        this.begin_open_paths(vec![path], window, cx);
                        window.push_notification(
                            crate::tr_args!(
                                "已将 {row_count} 行结果写入新标签",
                                "Wrote {row_count} result lines to a new tab",
                            ),
                            cx,
                        );
                    }
                    Ok((path, _)) => window.push_notification(
                        crate::tr_args!(
                            "结果已写入 {}，但当前正在打开其他文件，请稍后重试",
                            "Results were written to {}, but another file is being opened. Try again shortly.",
                            path.display(),
                        ),
                        cx,
                    ),
                    Err(error) => {
                        window.push_notification(
                            crate::tr_args!(
                                "结果未能写入新标签：{error}",
                                "Couldn’t write results to a new tab: {error}",
                            ),
                            cx,
                        )
                    }
                }
                cx.notify();
            });
        }));
    }

    fn open_timestamp_merged_results(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.global_search.scope, SearchScope::CurrentFile)
            || self.result_export_task.is_some()
            || self.open_task.is_some()
        {
            return;
        }
        let Some(export) = self.result_export_snapshot(cx) else {
            return;
        };
        self.result_export_operation = Some(ResultExportOperation::MergeByTimestamp);
        cx.notify();
        self.result_export_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let row_count = export.row_count();
                    let path = result_export::save_timestamp_merged_to_unique_temp(&export)?;
                    Ok::<_, anyhow::Error>((path, row_count))
                })
                .await;
            _ = this.update_in(cx, |this, window, cx| {
                this.result_export_task = None;
                this.result_export_operation = None;
                match result {
                    Ok((path, row_count)) if this.open_task.is_none() => {
                        this.transient_paths.insert(path.clone());
                        this.begin_open_paths(vec![path], window, cx);
                        window.push_notification(
                            crate::tr_args!(
                                "已按时间戳合并 {row_count} 行结果到新标签",
                                "Merged {row_count} result lines by timestamp into a new tab",
                            ),
                            cx,
                        );
                    }
                    Ok((path, _)) => window.push_notification(
                        crate::tr_args!(
                            "结果已生成到 {}，但当前正在打开其他文件",
                            "Results were generated at {}, but another file is being opened",
                            path.display(),
                        ),
                        cx,
                    ),
                    Err(error) => window.push_notification(
                        crate::tr_args!(
                            "按时间戳合并失败：{error}",
                            "Couldn’t merge by timestamp: {error}",
                        ),
                        cx,
                    ),
                }
                cx.notify();
            });
        }));
    }

    fn save_results_as(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.result_export_task.is_some() {
            return;
        }
        let Some(export) = self.result_export_snapshot(cx) else {
            return;
        };
        let suggested_name = self.result_export_suggested_name();
        let directory = self
            .active_document()
            .and_then(|tab| tab.document.path().parent())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let prompt = cx.prompt_for_new_path(&directory, Some(&suggested_name));
        self.result_export_operation = Some(ResultExportOperation::SaveAs);
        cx.notify();
        self.result_export_task = Some(cx.spawn_in(window, async move |this, cx| {
            let selected_path = prompt.await;
            let result = match selected_path {
                Ok(Ok(Some(target))) => {
                    let saved = cx
                        .background_spawn(async move {
                            let row_count = result_export::save(&export, &target)?;
                            Ok::<_, anyhow::Error>((target, row_count))
                        })
                        .await;
                    Some(saved)
                }
                Ok(Ok(None)) => None,
                Ok(Err(error)) => Some(Err(error)),
                Err(error) => Some(Err(anyhow::anyhow!(error))),
            };
            _ = this.update_in(cx, |this, window, cx| {
                this.result_export_task = None;
                this.result_export_operation = None;
                match result {
                    Some(Ok((path, row_count))) => window.push_notification(
                        crate::tr_args!(
                            "已保存 {row_count} 行结果到 {}",
                            "Saved {row_count} result lines to {}",
                            path.display(),
                        ),
                        cx,
                    ),
                    Some(Err(error)) => window.push_notification(
                        crate::tr_args!("结果未能保存：{error}", "Couldn’t save results: {error}",),
                        cx,
                    ),
                    None => {}
                }
                cx.notify();
            });
        }));
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
            if !path.as_os_str().is_empty() && !self.pending_external_paths.contains(&path) {
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
                self.transient_paths.insert(initial.path.clone());
            }
            if let Some(completion) = initial.move_completion {
                overrides
                    .move_completions
                    .insert(initial.path.clone(), completion);
            }
            if let Some(target_ix) = initial.target_ix {
                overrides
                    .target_indices
                    .insert(initial.path.clone(), target_ix);
            }
            if let Some(session) = initial.session {
                overrides.sessions.insert(initial.path.clone(), session);
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
        if paths.is_empty() || self.open_task.is_some() {
            return;
        }
        for path in &paths {
            if !overrides.sessions.contains_key(path)
                && let Some(session) = self.persistence.pending_session_overrides.get(path)
            {
                overrides.sessions.insert(path.clone(), session.clone());
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
                        overrides.sessions.get(path).cloned(),
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
        let opening_ids = paths
            .iter()
            .filter_map(|path| {
                self.documents
                    .iter()
                    .find(|tab| {
                        tab.document.path() == path && tab.load_state == DocumentLoadState::Opening
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
                                restored.extend(sessions);
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
            let previews = cx
                .background_spawn(async move {
                    prepare_paths_bounded(preview_paths, |path| {
                        prepare_document_preview(
                            path,
                            preview_store.as_deref(),
                            preview_sessions.get(path).cloned(),
                            effective_search_result_limit,
                        )
                    })
                })
                .await;

            _ = this.update_in(cx, |this, window, cx| {
                let previews = previews
                    .into_iter()
                    .filter(|(path, _)| {
                        opening_ids.get(path).is_some_and(|expected_id| {
                            this.documents.iter().any(|tab| {
                                tab.id == *expected_id
                                    && tab.document.path() == path
                                    && tab.load_state == DocumentLoadState::Opening
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
                            sessions.get(path).cloned(),
                            effective_search_result_limit,
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
                        tab.document.path() == path && tab.load_state == DocumentLoadState::Ready
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
                .any(|file| file.path == tab.document.path())
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
        self.color_labels = labels;
        if self
            .last_color_label_id
            .as_ref()
            .is_some_and(|id| self.color_labels.iter().all(|label| &label.id != id))
        {
            self.last_color_label_id = None;
        }
        for tab in &mut self.documents {
            let resolved = resolve_color_rules(&tab.keyword_color_rules, &self.color_labels);
            tab.resolved_color_rules = resolved.clone();
            for table in [tab.log_table.clone(), tab.result_table.clone()] {
                table.update(cx, |table, cx| {
                    table.delegate_mut().set_color_rules(resolved.clone());
                    table.refresh(cx);
                });
            }
        }
        self.refresh_global_result_rows(cx);
        cx.notify();
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

    fn check_for_updates(&mut self, manual: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.updates.task.is_some() {
            return;
        }
        if cfg!(debug_assertions) {
            if manual {
                window.push_notification(crate::tr!("开发构建不执行应用更新，请使用发行版验证更新流程", "Development builds don’t install updates. Use a release build to verify the update flow."), cx);
            }
            return;
        }
        let Some(client) = self.updates.client.clone() else {
            self.updates.state = AppUpdateState::Error;
            cx.notify();
            return;
        };
        let static_server_url = (!self.cloud.settings.server_url.trim().is_empty())
            .then(|| self.cloud.settings.server_url.trim().to_string());
        self.updates.state = AppUpdateState::Checking;
        self.updates.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    client.check_latest(env!("CARGO_PKG_VERSION"), static_server_url.as_deref())
                })
                .await;
            _ = this.update_in(cx, |this, window, cx| {
                this.updates.task = None;
                match result {
                    Ok(Some(update)) => {
                        this.updates.state = AppUpdateState::Available(Box::new(update.clone()));
                        this.confirm_update_download(update, window, cx);
                    }
                    Ok(None) => {
                        this.updates.state = AppUpdateState::Current;
                        if manual {
                            window.push_notification(
                                crate::tr_args!(
                                    "VCLogg2 {} 已是最新版本",
                                    "VCLogg2 {} is up to date",
                                    env!("CARGO_PKG_VERSION")
                                ),
                                cx,
                            );
                        }
                    }
                    Err(error) => {
                        this.updates.state = AppUpdateState::Error;
                        if manual {
                            window.push_notification(
                                crate::tr_args!(
                                    "检查更新失败：{error}",
                                    "Couldn’t check for updates: {error}"
                                ),
                                cx,
                            );
                        }
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn confirm_update_download(
        &mut self,
        update: AvailableUpdate,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let version = update.manifest.version.clone();
        let size = format_bytes(update.manifest.size);
        let workspace = cx.entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let workspace = workspace.clone();
            let update = update.clone();
            alert
                .title(crate::tr_args!("发现 VCLogg2 {version}", "VCLogg2 {version} is available"))
                .description(crate::tr_args!(
                    "更新包大小 {size}。下载后将验证整包和每个分块的 SHA-256。",
                    "The update is {size}. After downloading, SHA-256 will verify the package and every chunk.",
                ))
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(crate::tr!("下载更新", "Download update"))
                        .cancel_text(crate::tr!("稍后", "Later"))
                        .show_cancel(true),
                )
                .on_ok(move |_, window, cx| {
                    workspace.update(cx, |this, cx| {
                        this.download_update(update.clone(), window, cx)
                    });
                    true
                })
        });
    }

    fn download_update(
        &mut self,
        update: AvailableUpdate,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.updates.task.is_some() {
            return;
        }
        let Some(client) = self.updates.client.clone() else {
            return;
        };
        let progress = UpdateDownloadProgress::default();
        let progress_for_download = progress.clone();
        let version = update.manifest.version.clone();
        self.updates.transferred = 0;
        self.updates.total = update.manifest.size;
        self.updates.state = AppUpdateState::Downloading {
            version: version.clone(),
        };
        self.updates.progress_task = Some(cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                let snapshot = progress.snapshot();
                let keep_polling = this
                    .update_in(cx, |this, _, cx| {
                        let keep_polling =
                            matches!(this.updates.state, AppUpdateState::Downloading { .. });
                        if keep_polling
                            && (this.updates.transferred, this.updates.total) != snapshot
                        {
                            this.updates.transferred = snapshot.0;
                            this.updates.total = snapshot.1;
                            cx.notify();
                        }
                        keep_polling
                    })
                    .unwrap_or(false);
                if !keep_polling {
                    break;
                }
            }
        }));
        self.updates.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move { client.download(&update, &progress_for_download) })
                .await;
            _ = this.update_in(cx, |this, window, cx| {
                this.updates.task = None;
                this.updates.progress_task.take();
                match result {
                    Ok(downloaded) => {
                        this.updates.transferred = this.updates.total;
                        this.updates.state = AppUpdateState::Downloaded(downloaded.clone());
                        this.confirm_update_install(downloaded, window, cx);
                    }
                    Err(error) => {
                        this.updates.state = AppUpdateState::Error;
                        window.push_notification(
                            crate::tr_args!(
                                "下载更新失败：{error}",
                                "Couldn’t download the update: {error}"
                            ),
                            cx,
                        );
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn confirm_update_install(
        &mut self,
        update: DownloadedUpdate,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let version = update.version.clone();
        let workspace = cx.entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let workspace = workspace.clone();
            let update = update.clone();
            alert
                .title(crate::tr_args!("安装 VCLogg2 {version}？", "Install VCLogg2 {version}?"))
                .description(crate::tr!("应用会先按正常退出流程保存所有窗口状态，退出后由独立助手完成当前用户安装并重新启动。", "The application will save all window state and exit normally. A separate helper will install the update for the current user and restart VCLogg2."))
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(crate::tr!("退出并安装", "Quit and install"))
                        .cancel_text(crate::tr!("稍后", "Later"))
                        .show_cancel(true),
                )
                .on_ok(move |_, window, cx| {
                    workspace.update(cx, |this, cx| {
                        this.start_update_install(update.clone(), window, cx)
                    });
                    true
                })
        });
    }

    fn start_update_install(
        &mut self,
        update: DownloadedUpdate,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.updates.task.is_some() {
            return;
        }
        self.updates.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move { launch_installer(&update) })
                .await;
            _ = this.update_in(cx, |this, window, cx| match result {
                Ok(()) => cx.quit(),
                Err(error) => {
                    this.updates.task = None;
                    this.updates.state = AppUpdateState::Error;
                    window.push_notification(
                        crate::tr_args!(
                            "无法启动更新安装助手：{error}",
                            "Couldn’t start the update installer: {error}"
                        ),
                        cx,
                    );
                    cx.notify();
                }
            });
        }));
    }

    fn handle_update_button(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.updates.state.clone() {
            AppUpdateState::Unsupported => {
                window.push_notification(crate::tr!("开发构建不执行应用更新，请使用发行版验证更新流程", "Development builds don’t install updates. Use a release build to verify the update flow."), cx)
            }
            AppUpdateState::Available(update) => self.confirm_update_download(*update, window, cx),
            AppUpdateState::Downloaded(update) => self.confirm_update_install(update, window, cx),
            AppUpdateState::Downloading { .. } | AppUpdateState::Checking => {}
            AppUpdateState::Idle | AppUpdateState::Current | AppUpdateState::Error => {
                self.check_for_updates(true, window, cx)
            }
        }
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
        self.global_table.update(cx, |table, cx| {
            table.delegate_mut().set_appearance(&settings);
            table
                .delegate_mut()
                .set_word_boundary_characters(settings.word_boundary_characters.clone());
            table
                .delegate_mut()
                .set_highlight_log_levels(settings.highlight_log_levels);
            table.refresh(cx);
        });
        self.refresh_global_result_rows(cx);
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
                workspace.global_table.update(cx, |table, cx| {
                    table.delegate_mut().set_appearance(&shared_settings);
                    table.delegate_mut().set_word_boundary_characters(
                        shared_settings.word_boundary_characters.clone(),
                    );
                    table
                        .delegate_mut()
                        .set_highlight_log_levels(shared_settings.highlight_log_levels);
                    table.refresh(cx);
                });
                workspace.refresh_global_result_rows(cx);
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

        let word_wrap = if region == WrappedRegion::GlobalResults {
            self.global_viewport.is_wrapped()
        } else {
            self.documents[document_ix].log_viewport.is_wrapped()
        };
        let delta_y = if word_wrap {
            event.delta.pixel_delta(px(20.)).y
        } else {
            axis_delta.y
        };
        if delta_y == px(0.) {
            return;
        }

        let auto_follow_changed = region == WrappedRegion::Log
            && std::mem::replace(&mut self.documents[document_ix].auto_follow, false);
        let line_scroll = self.app_settings.scroll_by_line
            && (!word_wrap || self.app_settings.scroll_by_line_when_word_wrap);
        let scroll_scale = self.app_settings.mouse_wheel_scroll_percent as f32 / 100.;
        if !line_scroll && (scroll_scale - 1.).abs() < f32::EPSILON {
            if auto_follow_changed {
                cx.notify();
            }
            return;
        }

        let row_height = self.log_row_height();
        let line_count = usize::from(self.app_settings.mouse_wheel_scroll_lines.max(1));
        match region {
            WrappedRegion::Log => self.documents[document_ix].log_viewport.apply_wheel_scroll(
                delta_y,
                self.documents[document_ix].document.line_count(),
                row_height,
                line_count,
                line_scroll,
                scroll_scale,
            ),
            WrappedRegion::Results => self.documents[document_ix]
                .result_viewport
                .apply_wheel_scroll(
                    delta_y,
                    self.documents[document_ix].result_row_count(cx),
                    row_height,
                    line_count,
                    line_scroll,
                    scroll_scale,
                ),
            WrappedRegion::GlobalResults => self.global_viewport.apply_wheel_scroll(
                delta_y,
                self.global_table.read(cx).delegate().rows_len(),
                row_height,
                line_count,
                line_scroll,
                scroll_scale,
            ),
        }

        cx.stop_propagation();
        cx.notify();
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
        if self.transient_paths.contains(&path) {
            return;
        }
        self.persistence
            .pending_session_overrides
            .insert(path.clone(), state.clone());
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
                    this.persistence
                        .last_saved_sessions
                        .get(&saved_path)
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
                    if this
                        .persistence
                        .pending_session_overrides
                        .get(&saved_path)
                        .is_some_and(|latest| Self::session_contents_equal(latest, &desired_state))
                    {
                        this.persistence
                            .pending_session_overrides
                            .remove(&saved_path);
                    }
                    this.persistence
                        .last_saved_sessions
                        .insert(saved_path.clone(), result.state.clone());
                    if let Some(tab) = this
                        .documents
                        .iter_mut()
                        .find(|tab| tab.document.path() == saved_path)
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
        let marked_rows = tab
            .marked_rows
            .iter()
            .chain(tab.pending_restore_marked_rows.iter())
            .copied()
            .collect::<BTreeSet<_>>();
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
            marked_rows: marked_rows.into_iter().collect(),
            show_line_numbers: tab.show_line_numbers,
            show_row_separators: tab.show_row_separators,
            word_wrap: tab.log_viewport.is_wrapped(),
            keyword_color_rules: tab.keyword_color_rules.clone(),
            resume,
        }
    }

    fn take_quit_snapshot(&mut self, cx: &mut Context<Self>) -> QuitWorkspaceSnapshot {
        self.persistence.checkpoint_task.take();
        self.capture_retained_global_context(self.global_search.scope, cx);
        let search_state = self.primary_window.then(|| self.workspace_search_state());
        let store = self.persistence.store.clone();
        let predefined_filters = cx
            .global::<WorkspaceWindowRegistry>()
            .predefined_filters
            .clone();
        let mut sessions = std::mem::take(&mut self.persistence.pending_sessions)
            .into_iter()
            .filter(|(path, _, _)| !self.transient_paths.contains(path))
            .map(|(path, _, state)| (path, state))
            .collect::<BTreeMap<_, _>>();
        sessions.extend(
            self.persistence
                .pending_session_overrides
                .iter()
                .filter(|(path, _)| !self.transient_paths.contains(*path))
                .map(|(path, state)| (path.clone(), state.clone())),
        );
        sessions.extend(
            self.documents
                .iter()
                .filter(|tab| !self.transient_paths.contains(tab.document.path()))
                .map(|tab| {
                    (
                        tab.document.path().to_path_buf(),
                        self.file_session_state(tab, cx),
                    )
                }),
        );
        let open_paths = self
            .documents
            .iter()
            .filter(|tab| !self.transient_paths.contains(tab.document.path()))
            .map(|tab| tab.document.path().to_path_buf())
            .collect::<Vec<_>>();
        let active_path = self
            .active_document()
            .filter(|tab| !self.transient_paths.contains(tab.document.path()))
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
        self.persistence.checkpoint_task = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(1_500))
                .await;
            _ = this.update_in(cx, |this, window, cx| {
                let Some(tab) = this.documents.iter().find(|tab| tab.id == document_id) else {
                    this.persistence.checkpoint_task = None;
                    return;
                };
                let path = tab.document.path().to_path_buf();
                let base = tab.session_base.clone();
                let state = this.file_session_state(tab, cx);
                this.persistence.checkpoint_task = None;
                this.save_file_session(path, base, state, window, cx);
            });
        }));
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
        let previous_active_id = self.active_document().map(|tab| tab.id);
        let mut errors = Vec::new();
        let mut warnings = Vec::new();
        let mut recorded_paths = Vec::new();
        let mut cache_writes = Vec::new();
        let mut installed_document_ids = BTreeSet::new();

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
                .position(|tab| tab.document.path() == path)
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
                    if ready && !self.transient_paths.contains(&path) {
                        recorded_paths.push(path.clone());
                    }
                }
                if self.active_ix != Some(existing_ix) {
                    self.activate_tab(existing_ix, window, cx);
                }
                continue;
            }

            if prepared.load_state == DocumentLoadState::Ready
                && !self.transient_paths.contains(&path)
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
            let restored_marked_rows = session.marked_rows.iter().copied().collect::<BTreeSet<_>>();
            let marked_rows = restored_marked_rows
                .iter()
                .copied()
                .filter(|row| document.contains_source_row(*row))
                .collect::<BTreeSet<_>>();
            let result_rows =
                compute_result_rows(result_mode, Some(&prepared.search_result), &marked_rows);
            let marked_rows = Arc::new(marked_rows);
            let marked_rows_snapshot = marked_rows.clone();
            let keyword_color_rules = session.keyword_color_rules.clone();
            let resolved_color_rules =
                resolve_color_rules(&keyword_color_rules, &self.color_labels);
            let document_id = self.next_document_id;
            self.next_document_id += 1;
            if self
                .global_search
                .preferences
                .get(&path)
                .copied()
                .unwrap_or(true)
            {
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
                            return;
                        }
                        _ => return,
                    };
                    let Some(tab) = this.documents.iter_mut().find(|tab| tab.id == document_id)
                    else {
                        return;
                    };
                    if tab.restoring_result_selection {
                        tab.restoring_result_selection = false;
                        return;
                    }
                    let Some(source_row) =
                        table.read(cx).delegate().settle_table_selection(result_ix)
                    else {
                        return;
                    };
                    tab.auto_follow = false;
                    tab.select_and_center_log_row(source_row, cx);
                    if !keep_quick_find_focus {
                        tab.result_focus_handle.focus(window, cx);
                    }
                    tab.selection_table = SelectionTable::Results;
                    this.active_log_region = LogRegion::CurrentResults;
                    this.selected_source_row = Some(source_row);
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
                    let Some(tab) = this.documents.iter_mut().find(|tab| tab.id == document_id)
                    else {
                        return;
                    };
                    if tab.result_mode == *mode {
                        return;
                    }
                    tab.result_mode = *mode;
                    tab.refresh_result_rows(cx);
                    if mode.includes_marks() && !tab.marked_rows.is_empty() {
                        tab.results_visible = true;
                    }
                    this.schedule_checkpoint(document_id, window, cx);
                    cx.notify();
                },
            ));

            let results_visible = session.resume.current_search.results_visible
                || (result_mode.includes_marks() && !marked_rows.is_empty());
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
                results_visible,
                restoring_result_selection: false,
                marked_rows,
                pending_restore_marked_rows: if prepared.load_state != DocumentLoadState::Ready {
                    restored_marked_rows
                } else {
                    BTreeSet::new()
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
                selection_table: match session.resume.active_region {
                    PersistedLogRegion::Body => SelectionTable::Log,
                    PersistedLogRegion::CurrentResults => SelectionTable::Results,
                },
                uses_default_view_options,
                load_state: prepared.load_state,
                pending_restore_row: (prepared.load_state != DocumentLoadState::Ready)
                    .then_some(pending_restore_row)
                    .flatten(),
                pending_resume,
            });
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
            } else if let Some(target_ix) = target_indices.get(&path).copied() {
                let target_ix = target_ix.min(self.tabs.len());
                self.tabs.insert(target_ix, workspace_tab_id);
            } else {
                self.tabs.push(workspace_tab_id);
            }
            self.active_tab_id = workspace_tab_id;
        }
        self.reorder_documents_to_match_tabs();
        if let Some(active_path) = active_path
            && let Some(document_id) = self
                .documents
                .iter()
                .find(|tab| tab.document.path() == active_path)
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
            if let Some(expected_id) = opening_ids.get(&path).copied() {
                let still_open = self.documents.iter().any(|tab| {
                    tab.id == expected_id
                        && tab.document.path() == path
                        && matches!(
                            tab.load_state,
                            DocumentLoadState::Opening | DocumentLoadState::Preview
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
            tab.results_visible = resume.current_search.results_visible;
            tab.selection_table = match resume.active_region {
                PersistedLogRegion::Body => SelectionTable::Log,
                PersistedLogRegion::CurrentResults => SelectionTable::Results,
            };

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
            self.active_log_region = match resume.active_region {
                PersistedLogRegion::Body => LogRegion::Body,
                PersistedLogRegion::CurrentResults => LogRegion::CurrentResults,
            };
        }
    }

    fn complete_pending_directory_result_jump(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((path, source_row)) = self.pending_directory_result_jump.clone() else {
            return;
        };
        let Some(document_ix) = self.documents.iter().position(|tab| {
            tab.document.path() == path && tab.load_state == DocumentLoadState::Ready
        }) else {
            self.pending_directory_result_jump = None;
            return;
        };
        self.pending_directory_result_jump = None;
        self.activate_tab(document_ix, window, cx);
        let Some(tab) = self.documents.get_mut(document_ix) else {
            return;
        };
        tab.auto_follow = false;
        tab.selection_table = SelectionTable::Log;
        tab.select_and_center_log_row(source_row, cx);
        self.selected_source_row = Some(source_row);
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
            tab.pending_restore_marked_rows = session.marked_rows.iter().copied().collect();
            tab.pending_restore_row = session.selected_row;
            tab.pending_resume = Some(session.resume.clone());
            tab.keyword_color_rules = session.keyword_color_rules.clone();
            tab.resolved_color_rules =
                resolve_color_rules(&tab.keyword_color_rules, &self.color_labels);
            tab.show_line_numbers = session.show_line_numbers;
            tab.show_row_separators = session.show_row_separators;
            tab.log_viewport.set_word_wrap(session.word_wrap);
            tab.result_viewport.set_word_wrap(session.word_wrap);
            tab.uses_default_view_options = false;
            tab.results_visible = session.resume.current_search.results_visible
                || (tab.result_mode.includes_marks()
                    && !tab.pending_restore_marked_rows.is_empty());
            tab.selection_table = match session.resume.active_region {
                PersistedLogRegion::Body => SelectionTable::Log,
                PersistedLogRegion::CurrentResults => SelectionTable::Results,
            };
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
            let marked_rows = Arc::make_mut(&mut tab.marked_rows);
            marked_rows.extend(std::mem::take(&mut tab.pending_restore_marked_rows));
            marked_rows.retain(|row| *row < tab.document.source_line_count());
        } else {
            tab.marked_rows = Arc::new(
                tab.pending_restore_marked_rows
                    .iter()
                    .copied()
                    .filter(|row| tab.document.contains_source_row(*row))
                    .collect(),
            );
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

                match result {
                    Ok((document, search_result, query, search_matcher)) => {
                        tab.document = document;
                        tab.search_query.text = query.text;
                        tab.search_query.max_results = search_result_limit;
                        tab.search_result = search_result;
                        tab.search_matcher = search_matcher;
                        let marked_rows = Arc::make_mut(&mut tab.marked_rows);
                        marked_rows.extend(std::mem::take(&mut tab.pending_restore_marked_rows));
                        marked_rows.retain(|row| *row < tab.document.line_count());
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
                    }
                    Err(error) => {
                        if follow_end {
                            tab.auto_follow = false;
                        }
                        let message: SharedString = error.to_string().into();
                        window.push_notification(message.clone(), cx);
                        this.activity = Activity::Error;
                    }
                }
                this.open_task = None;
                this.open_queued_external_paths_if_idle(window, cx);
                cx.notify();
            });
        }));
    }

    fn activate_workspace_tab(
        &mut self,
        tab_id: WorkspaceTabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.tabs.contains(&tab_id) {
            return;
        }
        if self.active_tab_id != tab_id
            && let Some(active) = self.active_document()
        {
            let path = active.document.path().to_path_buf();
            let base = active.session_base.clone();
            let state = self.file_session_state(active, cx);
            self.save_file_session(path, base, state, window, cx);
        }
        self.active_tab_id = tab_id;
        self.sync_active_document_ix();
        self.pending_document_tab_reveal.set(None);
        self.sync_active_document(window, cx);
        cx.notify();
    }

    fn activate_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.documents.len() {
            return;
        }
        self.activate_workspace_tab(WorkspaceTabId::Document(self.documents[ix].id), window, cx);
    }

    fn reveal_pending_document_tab(&self) {
        let Some(document_id) = self.pending_document_tab_reveal.get() else {
            return;
        };
        let Some(ix) = self
            .tabs
            .iter()
            .position(|tab_id| *tab_id == WorkspaceTabId::Document(document_id))
        else {
            self.pending_document_tab_reveal.set(None);
            return;
        };

        // Segmented TabBar inserts its absolute selection indicator before the tab children.
        // ScrollHandle indices address those direct children, so the document index is shifted by
        // one. Before the first frame supplies child bounds, leave the reveal unacknowledged so
        // the next frame retries against the indicator-inclusive children.
        self.document_tab_scroll.scroll_to_item(ix + 1);
        if self.document_tab_scroll.children_count() > 0 {
            self.pending_document_tab_reveal.set(None);
        }
    }

    fn scroll_document_tabs_from_wheel(
        &self,
        event: &ScrollWheelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let delta = event.delta.pixel_delta(window.line_height());
        if delta.y == px(0.) || delta.x.abs() > delta.y.abs() {
            return;
        }

        let max_x = self.document_tab_scroll.max_offset().x.max(px(0.));
        if max_x == px(0.) {
            return;
        }

        let current = self.document_tab_scroll.offset();
        let next_x = (current.x + delta.y).clamp(-max_x, px(0.));
        if next_x != current.x {
            self.document_tab_scroll
                .set_offset(point(next_x, current.y));
            cx.notify();
        }
        cx.stop_propagation();
    }

    fn sync_active_document(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (title, query, selected_row) = self
            .active_document()
            .map(|tab| {
                (
                    format!("{} — VCLogg2", tab.title),
                    tab.search_query.text.clone(),
                    {
                        let table = tab.log_table.read(cx);
                        table
                            .active_log_row()
                            .and_then(|row_ix| table.delegate().source_row(row_ix))
                    },
                )
            })
            .unwrap_or_else(|| {
                (
                    crate::tr!("新标签页 — VCLogg2", "New tab — VCLogg2").to_string(),
                    String::new(),
                    None,
                )
            });

        window.set_window_title(&title);
        if self.global_search.scope == SearchScope::CurrentFile {
            self.reset_search_history_navigation();
            self.query
                .update(cx, |state, cx| state.set_value(query, window, cx));
        }
        self.selected_source_row = selected_row;
    }

    fn close_active_tab(
        &mut self,
        _: &CloseActiveTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_close_workspace_tabs(BTreeSet::from([self.active_tab_id]), window, cx);
    }

    fn close_tab_by_id(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        if !self.tabs.contains(&WorkspaceTabId::Document(id)) {
            return;
        }
        self.close_workspace_tabs(BTreeSet::from([WorkspaceTabId::Document(id)]), window, cx);
    }

    fn close_tab_group(
        &mut self,
        tab_id: WorkspaceTabId,
        group: TabCloseGroup,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(target_ix) = self.tabs.iter().position(|candidate| *candidate == tab_id) else {
            return;
        };
        let ids = self
            .tabs
            .iter()
            .enumerate()
            .filter(|(ix, candidate)| match group {
                TabCloseGroup::Current => **candidate == tab_id,
                TabCloseGroup::Others => **candidate != tab_id,
                TabCloseGroup::Left => *ix < target_ix,
                TabCloseGroup::Right => *ix > target_ix,
                TabCloseGroup::All => true,
            })
            .map(|(_, tab_id)| *tab_id)
            .collect::<BTreeSet<_>>();
        self.request_close_workspace_tabs(ids, window, cx);
    }

    fn request_close_workspace_tabs(
        &mut self,
        ids: BTreeSet<WorkspaceTabId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ids = ids
            .into_iter()
            .filter(|tab_id| self.tabs.contains(tab_id))
            .collect::<BTreeSet<_>>();
        if ids.is_empty() {
            return;
        }
        let document_ids = ids
            .iter()
            .filter_map(|tab_id| tab_id.document_id())
            .collect::<BTreeSet<_>>();
        if document_ids.is_empty() || !self.app_settings.confirm_close_tab {
            self.close_workspace_tabs(ids, window, cx);
            return;
        }

        let count = document_ids.len();
        let title = if count == 1 {
            crate::tr!("关闭日志标签？", "Close log tab?").to_string()
        } else {
            crate::tr!("关闭多个日志标签？", "Close log tabs?").to_string()
        };
        let description = if count == 1 {
            let label = self
                .documents
                .iter()
                .find(|tab| document_ids.contains(&tab.id))
                .map(|tab| tab.title.to_string())
                .unwrap_or_else(|| crate::tr!("当前日志", "Current log").to_string());
            crate::tr_args!(
                "确定关闭“{label}”吗？日志文件不会被删除，当前会话会在后台保存。",
                "Close “{label}”? The log file won’t be deleted and the current session will be saved in the background."
            )
        } else {
            crate::tr_args!(
                "确定关闭这 {count} 个日志标签吗？日志文件不会被删除，当前会话会在后台保存。",
                "Close these {count} log tabs? Log files won’t be deleted and the current sessions will be saved in the background."
            )
        };
        let workspace = cx.entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let workspace = workspace.clone();
            let ids = ids.clone();
            alert
                .icon(Icon::new(IconName::Info))
                .title(title.clone())
                .description(description.clone())
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(crate::tr!("关闭标签", "Close tabs"))
                        .cancel_text(crate::tr!("取消", "Cancel"))
                        .show_cancel(true),
                )
                .on_ok(move |_, window, cx| {
                    workspace.update(cx, |this, cx| {
                        this.close_workspace_tabs(ids.clone(), window, cx)
                    });
                    true
                })
        });
    }

    fn close_workspace_tabs(
        &mut self,
        ids: BTreeSet<WorkspaceTabId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if ids.is_empty() {
            return;
        }
        let previous_active_id = self.active_tab_id;
        let previous_active_ix = self.active_workspace_tab_ix().unwrap_or_default();
        let document_ids = ids
            .iter()
            .filter_map(|tab_id| tab_id.document_id())
            .collect::<BTreeSet<_>>();
        let sessions = self
            .documents
            .iter()
            .filter(|tab| document_ids.contains(&tab.id))
            .map(|tab| {
                (
                    tab.document.path().to_path_buf(),
                    tab.session_base.clone(),
                    self.file_session_state(tab, cx),
                )
            })
            .collect::<Vec<_>>();

        if self.searches.targets_any_document(&document_ids) {
            self.cancel_search();
        }
        for (path, base, session) in sessions {
            self.save_file_session(path, base, session, window, cx);
        }

        self.tabs.retain(|tab_id| !ids.contains(tab_id));
        self.documents.retain(|tab| !document_ids.contains(&tab.id));
        self.row_drag_bounds
            .retain(|(tab_id, _), _| !document_ids.contains(tab_id));
        if self.tabs.is_empty() {
            self.document_tab_scroll = ScrollHandle::new();
            self.pending_document_tab_reveal.set(None);
            let tab_id = WorkspaceTabId::New(self.next_new_tab_id);
            self.next_new_tab_id = self.next_new_tab_id.saturating_add(1);
            self.tabs.push(tab_id);
            self.active_tab_id = tab_id;
        } else if ids.contains(&previous_active_id) {
            self.active_tab_id = self.tabs[previous_active_ix.min(self.tabs.len() - 1)];
        } else {
            self.active_tab_id = previous_active_id;
        }
        self.global_search
            .selected_documents
            .retain(|document_id| !document_ids.contains(document_id));
        self.global_search
            .results
            .retain(|document_id, _| !document_ids.contains(document_id));
        self.reorder_documents_to_match_tabs();
        if !document_ids.is_empty() {
            self.global_search.revision = self.global_search.revision.saturating_add(1);
            self.refresh_global_result_rows(cx);
        }
        self.sync_active_document(window, cx);
        cx.notify();
    }

    fn reorder_tab(
        &mut self,
        tab_id: WorkspaceTabId,
        target_ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(source_ix) = self.tabs.iter().position(|candidate| *candidate == tab_id) else {
            return;
        };
        let target_ix = target_ix.min(self.tabs.len());
        let insert_ix = if source_ix < target_ix {
            target_ix.saturating_sub(1)
        } else {
            target_ix
        };
        if insert_ix == source_ix {
            return;
        }

        let tab_id = self.tabs.remove(source_ix);
        self.tabs.insert(insert_ix, tab_id);
        self.reorder_documents_to_match_tabs();
        self.refresh_global_result_rows(cx);
        self.persist_workspace_order(window, cx);
        cx.notify();
    }

    fn persist_workspace_order(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let store = self.persistence.store.clone();
        let sessions = self
            .documents
            .iter()
            .filter(|tab| !self.transient_paths.contains(tab.document.path()))
            .map(|tab| {
                (
                    tab.document.path().to_path_buf(),
                    self.file_session_state(tab, cx),
                )
            })
            .collect::<Vec<_>>();
        let open_paths = sessions
            .iter()
            .map(|(path, _)| path.clone())
            .collect::<Vec<_>>();
        let active_path = self
            .active_document()
            .filter(|tab| !self.transient_paths.contains(tab.document.path()))
            .map(|tab| tab.document.path().to_path_buf());
        let primary_window = self.primary_window;
        let previous_task = self.persistence.workspace_order_task.take();

        self.persistence.workspace_order_task = Some(cx.spawn_in(window, async move |this, cx| {
            if let Some(task) = previous_task {
                task.await;
            }
            let result = cx
                .background_spawn(async move {
                    if let Some(store) = store {
                        if primary_window {
                            store.save_workspace(&sessions, &open_paths, active_path.as_deref())
                        } else {
                            store.save_sessions(&sessions)
                        }
                    } else {
                        let store = StateStore::open_default()?;
                        if primary_window {
                            store.save_workspace(&sessions, &open_paths, active_path.as_deref())
                        } else {
                            store.save_sessions(&sessions)
                        }
                    }
                })
                .await;
            if let Err(error) = result {
                _ = this.update_in(cx, |_, window, cx| {
                    window.push_notification(
                        crate::tr_args!(
                            "标签顺序未能保存：{error}",
                            "Couldn’t save tab order: {error}"
                        ),
                        cx,
                    )
                });
            }
        }));
    }

    fn copy_tab_file_path(
        &mut self,
        document_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = self
            .documents
            .iter()
            .find(|tab| tab.id == document_id)
            .map(|tab| tab.document.path().display().to_string())
        else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(path));
        window.push_notification(crate::tr!("已复制文件路径", "File path copied"), cx);
    }

    fn reveal_tab_file(&mut self, document_id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self
            .documents
            .iter()
            .find(|tab| tab.id == document_id)
            .map(|tab| tab.document.path().to_path_buf())
        else {
            return;
        };
        match crate::open_directory::launch_custom(&self.app_settings.open_directory_command, &path)
        {
            Ok(true) => {}
            Ok(false) => {
                let Some(directory) = path.parent() else {
                    window.push_notification(
                        crate::tr!(
                            "无法确定文件所在目录",
                            "Couldn’t determine the file’s folder"
                        ),
                        cx,
                    );
                    return;
                };
                cx.open_url(&directory.to_string_lossy());
            }
            Err(error) => window.push_notification(error.to_string(), cx),
        }
    }

    fn copy_tab_to_new_window(
        &mut self,
        document_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.transfer_tab_to_new_window(document_id, TabTransferMode::Copy, None, window, cx);
    }

    fn move_tab_to_new_window(
        &mut self,
        document_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.transfer_tab_to_new_window(document_id, TabTransferMode::Move, None, window, cx);
    }

    fn transfer_tab_to_new_window(
        &mut self,
        document_id: u64,
        mode: TabTransferMode,
        placement: Option<(Bounds<Pixels>, Option<DisplayId>)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if mode == TabTransferMode::Move && !self.pending_tab_moves.insert(document_id) {
            window.push_notification(
                crate::tr!(
                    "此标签正在移动到新窗口",
                    "This tab is being moved to a new window"
                ),
                cx,
            );
            return;
        }
        let Some(tab) = self.documents.iter().find(|tab| tab.id == document_id) else {
            self.pending_tab_moves.remove(&document_id);
            return;
        };
        let path = tab.document.path().to_path_buf();
        let session = self.file_session_state(tab, cx);
        let transient = self.transient_paths.contains(&path);
        let initial = match mode {
            TabTransferMode::Copy => InitialDocument::new(path, session, transient),
            TabTransferMode::Move => InitialDocument::moving(
                path,
                session,
                transient,
                cx.weak_entity(),
                window.window_handle(),
                document_id,
            ),
        };
        let result = if let Some((bounds, display_id)) = placement {
            crate::open_workspace_window_at(cx, false, vec![initial], bounds, display_id)
        } else {
            crate::open_workspace_window(cx, false, vec![initial])
        };
        if let Err(error) = result {
            self.pending_tab_moves.remove(&document_id);
            let operation = match mode {
                TabTransferMode::Copy => crate::tr!("复制标签", "copy the tab"),
                TabTransferMode::Move => crate::tr!("移动标签", "move the tab"),
            };
            window.push_notification(
                crate::tr_args!(
                    "无法在新窗口{operation}：{error}",
                    "Couldn’t {operation} in a new window: {error}"
                ),
                cx,
            );
        }
    }

    fn receive_transferred_tab(
        &mut self,
        initial: InitialDocument,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> TabTransferReception {
        if let Some(existing_ix) = self
            .documents
            .iter()
            .position(|tab| tab.document.path() == initial.path)
        {
            self.activate_tab(existing_ix, window, cx);
            if let Some(completion) = initial.move_completion {
                cx.defer(move |cx| completion.finish(false, cx));
            }
            window.push_notification(
                crate::tr!(
                    "此窗口已经打开同一文件",
                    "This window already has the same file open"
                ),
                cx,
            );
            return TabTransferReception::AlreadyOpen;
        }
        if self.open_task.is_some() {
            if let Some(completion) = initial.move_completion {
                cx.defer(move |cx| completion.finish(false, cx));
            }
            window.push_notification(
                crate::tr!(
                    "此窗口正在打开其他文件，请稍后重试",
                    "This window is opening another file. Try again shortly."
                ),
                cx,
            );
            return TabTransferReception::Busy;
        }
        let file_name = initial
            .path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| initial.path.display().to_string());
        self.begin_open_initial_documents(vec![initial], window, cx);
        window.push_notification(
            crate::tr_args!("正在接收标签：{file_name}", "Receiving tab: {file_name}"),
            cx,
        );
        TabTransferReception::Accepted
    }

    fn transfer_tab_to_previous_window(
        &mut self,
        document_id: u64,
        mode: TabTransferMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let source_window = window.window_handle();
        let target = cx
            .global::<WorkspaceWindowRegistry>()
            .previous_window(source_window);
        let Some(target) = target else {
            window.push_notification(
                crate::tr!(
                    "没有可接收标签的另一窗口",
                    "No other window can receive the tab"
                ),
                cx,
            );
            return;
        };
        self.transfer_tab_to_window_target(
            document_id,
            mode,
            TabTransferTarget {
                window: target.window,
                workspace: target.workspace,
                target_ix: None,
            },
            window,
            cx,
        );
    }

    fn transfer_tab_to_window_target(
        &mut self,
        document_id: u64,
        mode: TabTransferMode,
        target: TabTransferTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let source_window = window.window_handle();
        if mode == TabTransferMode::Move && !self.pending_tab_moves.insert(document_id) {
            window.push_notification(
                crate::tr!(
                    "此标签正在移动到另一窗口",
                    "This tab is being moved to another window"
                ),
                cx,
            );
            return;
        }
        let Some(tab) = self.documents.iter().find(|tab| tab.id == document_id) else {
            self.pending_tab_moves.remove(&document_id);
            return;
        };
        let path = tab.document.path().to_path_buf();
        let file_name = tab.title.clone();
        let session = self.file_session_state(tab, cx);
        let transient = self.transient_paths.contains(&path);
        let mut initial = match mode {
            TabTransferMode::Copy => InitialDocument::new(path, session, transient),
            TabTransferMode::Move => InitialDocument::moving(
                path,
                session,
                transient,
                cx.weak_entity(),
                source_window,
                document_id,
            ),
        };
        if let Some(target_ix) = target.target_ix {
            initial = initial.at_index(target_ix);
        }
        let result = target.window.update(cx, move |_, target_window, cx| {
            let target_workspace = target.workspace;
            target_workspace.update(cx, |target_workspace, cx| {
                target_workspace.receive_transferred_tab(initial, target_window, cx)
            })
        });
        let reception = result.unwrap_or(TabTransferReception::Closed);
        match (mode, reception) {
            (TabTransferMode::Copy, TabTransferReception::Accepted) => window.push_notification(
                crate::tr_args!(
                    "已把 {file_name} 复制到另一窗口",
                    "Copied {file_name} to another window"
                ),
                cx,
            ),
            (TabTransferMode::Move, TabTransferReception::Accepted) => window.push_notification(
                crate::tr_args!(
                    "正在把 {file_name} 移动到另一窗口",
                    "Moving {file_name} to another window"
                ),
                cx,
            ),
            (TabTransferMode::Copy, TabTransferReception::AlreadyOpen) => window.push_notification(
                crate::tr!(
                    "另一窗口已经打开同一文件",
                    "Another window already has the same file open"
                ),
                cx,
            ),
            (TabTransferMode::Copy, TabTransferReception::Busy) => window.push_notification(
                crate::tr!(
                    "另一窗口正忙，标签未复制",
                    "The other window is busy; the tab wasn’t copied"
                ),
                cx,
            ),
            (_, TabTransferReception::Closed) => {
                self.pending_tab_moves.remove(&document_id);
                window.push_notification(
                    crate::tr!(
                        "另一窗口已关闭，标签仍保留在当前窗口",
                        "The other window closed; the tab remains in this window"
                    ),
                    cx,
                );
            }
            (
                TabTransferMode::Move,
                TabTransferReception::AlreadyOpen | TabTransferReception::Busy,
            ) => {}
        }
    }

    fn open_rename_tab_dialog(
        &mut self,
        document_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(current_title) = self
            .documents
            .iter()
            .find(|tab| tab.id == document_id)
            .map(|tab| tab.title.to_string())
        else {
            return;
        };
        let rename = cx.new(|cx| RenameTabDialog::new(&current_title, window, cx));
        let input = rename.read(cx).input();
        let focus_input = input.clone();
        window.defer(cx, move |window, cx| {
            focus_input.focus_handle(cx).focus(window, cx);
            focus_input.update(cx, |input, cx| input.select_all(window, cx));
        });
        let workspace = cx.entity();
        window.open_dialog(cx, move |dialog, _, _| {
            let rename_for_submit = rename.clone();
            let input_for_submit = input.clone();
            let workspace = workspace.clone();
            dialog
                .title(crate::tr!("重命名标签", "Rename tab"))
                .child(rename.clone())
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(crate::tr!("保存", "Save"))
                        .cancel_text(crate::tr!("取消", "Cancel"))
                        .show_cancel(true),
                )
                .on_ok(move |_, window, cx| {
                    let Some(title) = rename_for_submit.read(cx).title(cx) else {
                        rename_for_submit.update(cx, |rename, cx| rename.show_validation_error(cx));
                        input_for_submit.focus_handle(cx).focus(window, cx);
                        return false;
                    };
                    workspace.update(cx, |workspace, cx| {
                        workspace.rename_tab(document_id, title, window, cx)
                    });
                    true
                })
        });
    }

    fn rename_tab(
        &mut self,
        document_id: u64,
        title: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.documents.iter_mut().find(|tab| tab.id == document_id) else {
            return;
        };
        if tab.title.as_ref() == title {
            return;
        }
        let display_title: SharedString = title.clone().into();
        tab.title = display_title.clone();
        tab.custom_title = Some(title.clone());
        if let Some(result) = self.global_search.results.get_mut(&document_id) {
            result.title = display_title;
        }
        self.refresh_global_result_rows(cx);
        if self
            .active_document()
            .is_some_and(|tab| tab.id == document_id)
        {
            self.sync_active_document(window, cx);
        }
        self.schedule_checkpoint(document_id, window, cx);
        window.push_notification(
            crate::tr_args!("标签已重命名为 {title}", "Tab renamed to {title}"),
            cx,
        );
        cx.notify();
    }

    fn restore_tab_title(&mut self, document_id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.documents.iter_mut().find(|tab| tab.id == document_id) else {
            return;
        };
        if tab.custom_title.is_none() {
            return;
        }
        let original_title = tab.document.file_name();
        let display_title: SharedString = original_title.clone().into();
        tab.title = display_title.clone();
        tab.custom_title = None;
        if let Some(result) = self.global_search.results.get_mut(&document_id) {
            result.title = display_title;
        }
        self.refresh_global_result_rows(cx);
        if self
            .active_document()
            .is_some_and(|tab| tab.id == document_id)
        {
            self.sync_active_document(window, cx);
        }
        self.schedule_checkpoint(document_id, window, cx);
        window.push_notification(
            crate::tr_args!(
                "已恢复标签名称：{original_title}",
                "Tab name restored: {original_title}"
            ),
            cx,
        );
        cx.notify();
    }

    fn confirm_close_and_delete_file(
        &mut self,
        document_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.documents.iter().find(|tab| tab.id == document_id) else {
            return;
        };
        let name = tab.document.file_name();
        let path = tab.document.path().to_path_buf();
        let description = crate::tr_args!(
            "文件将被移入系统回收站，对应标签页也会关闭。\n\n{name}\n{}\n\n如果需要，可以稍后从系统回收站恢复。",
            "The file will be moved to the system trash and its tab will close.\n\n{name}\n{}\n\nYou can restore it later from the trash if needed.",
            path.display(),
        );
        let workspace = cx.entity();
        window.open_alert_dialog(cx, move |alert, _, cx| {
            let workspace = workspace.clone();
            let delete_path = path.clone();
            let delete_name = name.clone();
            alert
                .icon(Icon::new(IconName::Info).text_color(cx.theme().danger))
                .title(crate::tr!(
                    "关闭并删除此文件？",
                    "Close and delete this file?"
                ))
                .description(description.clone())
                .button_props(
                    DialogButtonProps::default()
                        .ok_variant(ButtonVariant::Danger)
                        .ok_text(crate::tr!("删除文件", "Delete file"))
                        .cancel_text(crate::tr!("取消", "Cancel"))
                        .show_cancel(true),
                )
                .on_ok(move |_, window, cx| {
                    workspace.update(cx, |this, cx| {
                        this.close_tab_by_id(document_id, window, cx);
                        let workspace = cx.weak_entity();
                        let path = delete_path.clone();
                        let name = delete_name.clone();
                        window.defer(cx, move |window, cx| {
                            _ = workspace.update(cx, |this, cx| {
                                this.start_move_file_to_trash(path, name, window, cx)
                            });
                        });
                    });
                    true
                })
        });
    }

    fn start_move_file_to_trash(
        &mut self,
        path: PathBuf,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let store = self.persistence.store.clone();
        self.persistence
            .state_tasks
            .push(cx.spawn_in(window, async move |this, cx| {
                let result = cx
                    .background_spawn(async move {
                        let moved = crate::trash::move_file_to_trash(&path)?;
                        let collections = if let Some(store) = store {
                            store.delete_session_for_path(&path)?;
                            Some((
                                store.recent_files(8)?,
                                store.pinned_files()?,
                                store.last_workspace()?,
                            ))
                        } else {
                            None
                        };
                        Ok::<_, anyhow::Error>((moved, collections))
                    })
                    .await;
                _ = this.update_in(cx, |this, window, cx| {
                    match result {
                    Ok((moved, collections)) => {
                        if let Some((recent, pinned, last_workspace)) = collections {
                            this.recent_files = recent;
                            this.pinned_files = pinned;
                            this.last_workspace_files = last_workspace;
                        }
                        window.push_notification(
                            if moved {
                                crate::tr_args!(
                                    "已关闭并移入回收站：{name}",
                                    "Closed and moved to the trash: {name}"
                                )
                            } else {
                                crate::tr_args!(
                                    "已关闭标签；文件原本已不存在：{name}",
                                    "Tab closed; the file no longer existed: {name}"
                                )
                            },
                            cx,
                        );
                    }
                    Err(error) => window.push_notification(
                        crate::tr_args!(
                            "标签已关闭，但文件未能移入回收站：{error}",
                            "The tab closed, but the file couldn’t be moved to the trash: {error}"
                        ),
                        cx,
                    ),
                }
                    cx.notify();
                });
            }));
    }

    fn build_tab_menu(
        menu: PopupMenu,
        document_id: u64,
        state: TabMenuState,
        workspace: Entity<Self>,
        window: &mut Window,
    ) -> PopupMenu {
        let close = {
            let workspace = workspace.clone();
            window.listener_for(&workspace, move |this, _, window, cx| {
                this.close_tab_group(
                    WorkspaceTabId::Document(document_id),
                    TabCloseGroup::Current,
                    window,
                    cx,
                )
            })
        };
        let close_others = {
            let workspace = workspace.clone();
            window.listener_for(&workspace, move |this, _, window, cx| {
                this.close_tab_group(
                    WorkspaceTabId::Document(document_id),
                    TabCloseGroup::Others,
                    window,
                    cx,
                )
            })
        };
        let close_left = {
            let workspace = workspace.clone();
            window.listener_for(&workspace, move |this, _, window, cx| {
                this.close_tab_group(
                    WorkspaceTabId::Document(document_id),
                    TabCloseGroup::Left,
                    window,
                    cx,
                )
            })
        };
        let close_right = {
            let workspace = workspace.clone();
            window.listener_for(&workspace, move |this, _, window, cx| {
                this.close_tab_group(
                    WorkspaceTabId::Document(document_id),
                    TabCloseGroup::Right,
                    window,
                    cx,
                )
            })
        };
        let close_all = {
            let workspace = workspace.clone();
            window.listener_for(&workspace, move |this, _, window, cx| {
                this.close_tab_group(
                    WorkspaceTabId::Document(document_id),
                    TabCloseGroup::All,
                    window,
                    cx,
                )
            })
        };
        let close_and_delete = {
            let workspace = workspace.clone();
            window.listener_for(&workspace, move |this, _, window, cx| {
                this.confirm_close_and_delete_file(document_id, window, cx)
            })
        };
        let copy_path = {
            let workspace = workspace.clone();
            window.listener_for(&workspace, move |this, _, window, cx| {
                this.copy_tab_file_path(document_id, window, cx)
            })
        };
        let reveal = window.listener_for(&workspace, move |this, _, window, cx| {
            this.reveal_tab_file(document_id, window, cx)
        });
        let copy_to_new_window = {
            let workspace = workspace.clone();
            window.listener_for(&workspace, move |this, _, window, cx| {
                this.copy_tab_to_new_window(document_id, window, cx)
            })
        };
        let move_to_new_window = {
            let workspace = workspace.clone();
            window.listener_for(&workspace, move |this, _, window, cx| {
                this.move_tab_to_new_window(document_id, window, cx)
            })
        };
        let move_to_other_window = {
            let workspace = workspace.clone();
            window.listener_for(&workspace, move |this, _, window, cx| {
                this.transfer_tab_to_previous_window(document_id, TabTransferMode::Move, window, cx)
            })
        };
        let copy_to_other_window = {
            let workspace = workspace.clone();
            window.listener_for(&workspace, move |this, _, window, cx| {
                this.transfer_tab_to_previous_window(document_id, TabTransferMode::Copy, window, cx)
            })
        };
        let rename = {
            let workspace = workspace.clone();
            window.listener_for(&workspace, move |this, _, window, cx| {
                this.open_rename_tab_dialog(document_id, window, cx)
            })
        };
        let restore_title = window.listener_for(&workspace, move |this, _, window, cx| {
            this.restore_tab_title(document_id, window, cx)
        });

        menu.item(PopupMenuItem::new(crate::tr!("关闭标签", "Close tab")).on_click(close))
            .item(
                PopupMenuItem::new(crate::tr!("关闭并删除文件…", "Close and delete file…"))
                    .on_click(close_and_delete),
            )
            .item(
                PopupMenuItem::new(crate::tr!("关闭其他标签", "Close other tabs"))
                    .disabled(state.tab_count <= 1)
                    .on_click(close_others),
            )
            .item(
                PopupMenuItem::new(crate::tr!("关闭左侧标签", "Close tabs to the left"))
                    .disabled(state.tab_ix == 0)
                    .on_click(close_left),
            )
            .item(
                PopupMenuItem::new(crate::tr!("关闭右侧标签", "Close tabs to the right"))
                    .disabled(state.tab_ix + 1 >= state.tab_count)
                    .on_click(close_right),
            )
            .item(
                PopupMenuItem::new(crate::tr!("关闭所有标签", "Close all tabs"))
                    .on_click(close_all),
            )
            .separator()
            .item(
                PopupMenuItem::new(crate::tr!("复制完整路径", "Copy full path"))
                    .on_click(copy_path),
            )
            .item(
                PopupMenuItem::new(crate::tr!("打开所在目录", "Open containing folder"))
                    .on_click(reveal),
            )
            .item(
                PopupMenuItem::new(crate::tr!("复制到新窗口", "Copy to new window"))
                    .on_click(copy_to_new_window),
            )
            .item(
                PopupMenuItem::new(crate::tr!("移动到新窗口", "Move to new window"))
                    .on_click(move_to_new_window),
            )
            .item(
                PopupMenuItem::new(crate::tr!("移动到另一窗口", "Move to another window"))
                    .disabled(!state.has_other_window)
                    .on_click(move_to_other_window),
            )
            .item(
                PopupMenuItem::new(crate::tr!("复制到另一窗口", "Copy to another window"))
                    .disabled(!state.has_other_window)
                    .on_click(copy_to_other_window),
            )
            .separator()
            .item(PopupMenuItem::new(crate::tr!("重命名标签…", "Rename tab…")).on_click(rename))
            .item(
                PopupMenuItem::new(crate::tr!("恢复标签名称", "Restore tab name"))
                    .disabled(!state.can_restore_title)
                    .on_click(restore_title),
            )
    }

    fn build_new_tab_menu(
        menu: PopupMenu,
        tab_id: WorkspaceTabId,
        state: TabMenuState,
        workspace: Entity<Self>,
        window: &mut Window,
    ) -> PopupMenu {
        let close = {
            let workspace = workspace.clone();
            window.listener_for(&workspace, move |this, _, window, cx| {
                this.close_tab_group(tab_id, TabCloseGroup::Current, window, cx)
            })
        };
        let close_others = {
            let workspace = workspace.clone();
            window.listener_for(&workspace, move |this, _, window, cx| {
                this.close_tab_group(tab_id, TabCloseGroup::Others, window, cx)
            })
        };
        let close_left = {
            let workspace = workspace.clone();
            window.listener_for(&workspace, move |this, _, window, cx| {
                this.close_tab_group(tab_id, TabCloseGroup::Left, window, cx)
            })
        };
        let close_right = {
            let workspace = workspace.clone();
            window.listener_for(&workspace, move |this, _, window, cx| {
                this.close_tab_group(tab_id, TabCloseGroup::Right, window, cx)
            })
        };
        let close_all = window.listener_for(&workspace, move |this, _, window, cx| {
            this.close_tab_group(tab_id, TabCloseGroup::All, window, cx)
        });

        menu.item(PopupMenuItem::new(crate::tr!("关闭标签", "Close tab")).on_click(close))
            .item(
                PopupMenuItem::new(crate::tr!("关闭其他标签", "Close other tabs"))
                    .disabled(state.tab_count <= 1)
                    .on_click(close_others),
            )
            .item(
                PopupMenuItem::new(crate::tr!("关闭左侧标签", "Close tabs to the left"))
                    .disabled(state.tab_ix == 0)
                    .on_click(close_left),
            )
            .item(
                PopupMenuItem::new(crate::tr!("关闭右侧标签", "Close tabs to the right"))
                    .disabled(state.tab_ix + 1 >= state.tab_count)
                    .on_click(close_right),
            )
            .item(
                PopupMenuItem::new(crate::tr!("关闭所有标签", "Close all tabs"))
                    .on_click(close_all),
            )
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
            let selected_rows = self.global_table.read(cx).delegate().selected_matches();
            if selected_rows.is_empty() {
                window.push_notification(
                    crate::tr!(
                        "请先选择要复制的全局结果行",
                        "Select global result lines to copy first"
                    ),
                    cx,
                );
                return;
            }
            let mut text = String::new();
            let mut copied = 0_usize;
            for (document_id, source_row) in selected_rows {
                let document = self
                    .documents
                    .iter()
                    .find(|tab| tab.id == document_id)
                    .map(|tab| tab.document.clone())
                    .or_else(|| {
                        self.global_search
                            .results
                            .get(&document_id)
                            .map(|result| result.document.clone())
                    });
                let Some(document) = document else {
                    continue;
                };
                let Some(line) = document.line(source_row) else {
                    continue;
                };
                if copied > 0 {
                    text.push('\n');
                }
                if include_line_number {
                    text.push_str(&(source_row + 1).to_string());
                    text.push('\t');
                }
                text.push_str(&line);
                copied += 1;
            }
            if text.is_empty() {
                window.push_notification(
                    crate::tr!(
                        "所选全局结果已不可用，请重新选择",
                        "The selected global results are no longer available. Select them again."
                    ),
                    cx,
                );
                return;
            }
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            window.push_notification(
                crate::tr_args!(
                    "已复制 {copied} 条全局结果",
                    "Copied {copied} global results"
                ),
                cx,
            );
            return;
        }
        let Some(tab) = self.active_document() else {
            return;
        };
        let selected_rows = tab.selected_source_rows(cx);
        if selected_rows.is_empty() {
            window.push_notification(
                crate::tr!("请先选择要复制的日志行", "Select log lines to copy first"),
                cx,
            );
            return;
        }
        let mut text = String::new();
        let mut copied = 0_usize;
        for source_row in selected_rows.iter().copied() {
            let Some(line) = tab.document.line(source_row) else {
                continue;
            };
            if copied > 0 {
                text.push('\n');
            }
            if include_line_number {
                text.push_str(&(source_row + 1).to_string());
                text.push('\t');
            }
            text.push_str(&line);
            copied = copied.saturating_add(1);
        }
        if text.is_empty() {
            window.push_notification(
                crate::tr!(
                    "所选日志行已不可用，请重新选择",
                    "The selected log lines are no longer available. Select them again."
                ),
                cx,
            );
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        if selected_rows.len() == 1 {
            window.push_notification(
                crate::tr_args!("已复制第 {} 行", "Copied line {}", selected_rows[0] + 1),
                cx,
            );
        } else {
            window.push_notification(
                crate::tr_args!("已复制 {} 行", "Copied {} lines", selected_rows.len()),
                cx,
            );
        }
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
        let (active_ix, keywords) =
            match self.context_color_target(Some(selected_text.as_str()), cx) {
                Ok(target) => target,
                Err(message) => {
                    window.push_notification(message, cx);
                    return;
                }
            };
        if keywords.is_empty() {
            window.push_notification(
                crate::tr!(
                    "请先选择包含文字的日志行",
                    "Select log lines containing text first"
                ),
                cx,
            );
            return;
        }

        let remove = keywords.iter().all(|keyword| {
            self.documents[active_ix]
                .keyword_color_rules
                .iter()
                .any(|rule| {
                    rule.enabled && rule.case_sensitive && rule.keyword.as_str() == keyword.as_str()
                })
        });
        let feedback = if remove {
            self.documents[active_ix]
                .keyword_color_rules
                .retain(|rule| {
                    !(rule.enabled
                        && rule.case_sensitive
                        && keywords.contains(rule.keyword.as_str()))
                });
            crate::tr_args!(
                "已移除 {} 行文字的颜色标签",
                "Removed color labels from {} lines of text",
                keywords.len()
            )
        } else {
            if self.color_labels.is_empty() {
                window.push_notification(
                    crate::tr!(
                        "请先在“颜色标签…”中添加标签",
                        "Add a label in Color labels… first"
                    ),
                    cx,
                );
                return;
            }
            let next_ix = self
                .last_color_label_id
                .as_deref()
                .and_then(|id| self.color_labels.iter().position(|label| label.id == id))
                .map_or(0, |ix| (ix + 1) % self.color_labels.len());
            let label = self.color_labels[next_ix].clone();
            self.last_color_label_id = Some(label.id.clone());
            for keyword in &keywords {
                if let Some(rule) = self.documents[active_ix]
                    .keyword_color_rules
                    .iter_mut()
                    .find(|rule| rule.case_sensitive && rule.keyword == *keyword)
                {
                    rule.label_id = Some(label.id.clone());
                    rule.color = label.color;
                    rule.alpha = label.alpha;
                    rule.enabled = true;
                } else {
                    self.documents[active_ix]
                        .keyword_color_rules
                        .push(KeywordColorRule {
                            label_id: Some(label.id.clone()),
                            keyword: keyword.clone(),
                            color: label.color,
                            alpha: label.alpha,
                            case_sensitive: true,
                            enabled: true,
                        });
                }
            }
            crate::tr_args!(
                "已用“{}”高亮 {} 行文字",
                "Highlighted “{}” in {} lines of text",
                label.localized_name(),
                keywords.len()
            )
        };

        let resolved = resolve_color_rules(
            &self.documents[active_ix].keyword_color_rules,
            &self.color_labels,
        );
        self.documents[active_ix].resolved_color_rules = resolved.clone();
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
        self.refresh_global_result_rows(cx);
        self.schedule_checkpoint(document_id, window, cx);
        window.push_notification(feedback, cx);
        cx.notify();
    }

    fn toggle_marked_row(
        &mut self,
        _: &ToggleMarkedRow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active_log_region == LogRegion::GlobalResults && self.global_search.results_visible
        {
            if self.global_search.scope == SearchScope::Directory {
                window.push_notification(
                    crate::tr!(
                        "请先打开目录结果所属文件，再标记日志行",
                        "Open the file containing the directory result before marking log lines"
                    ),
                    cx,
                );
                return;
            }
            let selected_matches = self.global_table.read(cx).delegate().selected_matches();
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
            let mut selected_by_document = BTreeMap::<u64, Vec<usize>>::new();
            for (document_id, source_row) in selected_matches {
                selected_by_document
                    .entry(document_id)
                    .or_default()
                    .push(source_row);
            }
            let is_marking = selected_by_document.iter().any(|(document_id, rows)| {
                self.documents
                    .iter()
                    .find(|tab| tab.id == *document_id)
                    .is_some_and(|tab| rows.iter().any(|row| !tab.marked_rows.contains(row)))
            });
            let mut changed_documents = Vec::new();
            let mut changed_rows = 0_usize;
            for (document_id, rows) in selected_by_document {
                let Some(tab_ix) = self.documents.iter().position(|tab| tab.id == document_id)
                else {
                    continue;
                };
                let tab = &mut self.documents[tab_ix];
                let rows = rows
                    .into_iter()
                    .filter(|row| tab.document.contains_source_row(*row))
                    .collect::<Vec<_>>();
                if rows.is_empty() {
                    continue;
                }
                if is_marking {
                    Arc::make_mut(&mut tab.marked_rows).extend(rows.iter().copied());
                    tab.pending_restore_marked_rows.extend(rows.iter().copied());
                } else {
                    let marked_rows = Arc::make_mut(&mut tab.marked_rows);
                    for source_row in &rows {
                        marked_rows.remove(source_row);
                        tab.pending_restore_marked_rows.remove(source_row);
                    }
                }
                changed_rows = changed_rows.saturating_add(rows.len());
                let marked_rows = tab.marked_rows.clone();
                tab.log_table.update(cx, |table, cx| {
                    table.delegate_mut().set_marked_rows(marked_rows.clone());
                    table.refresh(cx);
                });
                tab.refresh_result_rows(cx);
                if is_marking && tab.result_mode.includes_marks() {
                    tab.results_visible = true;
                }
                tab.result_table.update(cx, |table, cx| {
                    table.delegate_mut().set_marked_rows(marked_rows);
                    table.refresh(cx);
                });
                changed_documents.push(document_id);
            }
            self.refresh_global_result_rows(cx);
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
        let selected_rows = self.documents[active_ix].selected_source_rows(cx);
        if selected_rows.is_empty() {
            window.push_notification(
                crate::tr!("请先选择要标记的日志行", "Select log lines to mark first"),
                cx,
            );
            return;
        }
        let (document_id, is_marking) = {
            let tab = &mut self.documents[active_ix];
            if selected_rows
                .iter()
                .any(|source_row| !tab.document.contains_source_row(*source_row))
            {
                window.push_notification(
                    crate::tr!(
                        "所选日志行已不可用，请重新选择",
                        "The selected log lines are no longer available. Select them again."
                    ),
                    cx,
                );
                return;
            }

            let is_marking = !selected_rows
                .iter()
                .all(|source_row| tab.marked_rows.contains(source_row));
            if is_marking {
                Arc::make_mut(&mut tab.marked_rows).extend(selected_rows.iter().copied());
                tab.pending_restore_marked_rows
                    .extend(selected_rows.iter().copied());
            } else {
                let marked_rows = Arc::make_mut(&mut tab.marked_rows);
                for source_row in &selected_rows {
                    marked_rows.remove(source_row);
                    tab.pending_restore_marked_rows.remove(source_row);
                }
            }
            let marked_rows = tab.marked_rows.clone();
            tab.log_table.update(cx, |table, cx| {
                table.delegate_mut().set_marked_rows(marked_rows.clone());
                table.refresh(cx);
            });
            tab.refresh_result_rows(cx);
            if is_marking && tab.result_mode.includes_marks() {
                tab.results_visible = true;
            }
            tab.result_table.update(cx, |table, cx| {
                table.delegate_mut().set_marked_rows(marked_rows);
                table.refresh(cx);
            });
            (tab.id, is_marking)
        };
        if is_marking
            && self.global_search.result_mode.includes_marks()
            && self.global_search.selected_documents.contains(&document_id)
        {
            self.global_search.results_visible = true;
        }
        self.refresh_global_result_rows(cx);
        let action = if is_marking {
            crate::tr!("已标记", "Marked")
        } else {
            crate::tr!("已取消标记", "Unmarked")
        };
        if selected_rows.len() == 1 {
            window.push_notification(
                crate::tr_args!("{action}第 {} 行", "{action} line {}", selected_rows[0] + 1),
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

    fn open_quick_find(&mut self, _: &OpenQuickFind, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.active_document() else {
            return;
        };
        if tab.load_state != DocumentLoadState::Ready {
            window.push_notification(
                crate::tr!(
                    "完整索引建立后即可查找",
                    "Find will be available after the full index is built"
                ),
                cx,
            );
            return;
        }
        let (target, anchor) = match self.last_user_log_region {
            LogRegion::GlobalResults
                if self.global_search.scope != SearchScope::CurrentFile
                    && self.global_search.results_visible =>
            {
                (
                    QuickFindTarget::GlobalResults,
                    self.global_table
                        .read(cx)
                        .active_log_row()
                        .unwrap_or_default(),
                )
            }
            LogRegion::CurrentResults
                if self.global_search.scope == SearchScope::CurrentFile && tab.results_visible =>
            {
                (
                    QuickFindTarget::Results(tab.id),
                    tab.result_table
                        .read(cx)
                        .active_log_row()
                        .unwrap_or_default(),
                )
            }
            _ => (
                QuickFindTarget::Log(tab.id),
                tab.log_table.read(cx).active_log_row().unwrap_or_default(),
            ),
        };

        self.quick_find.open(target, anchor);
        self.update_quick_find_matcher(window, cx);
        let focus = self.quick_find.query.focus_handle(cx);
        self.quick_find
            .query
            .update(cx, |state, cx| state.select_all(window, cx));
        window.defer(cx, move |window, cx| focus.focus(window, cx));
        if !self.quick_find.query.read(cx).value().is_empty() {
            self.start_quick_find(QuickFindDirection::Forward, true, window, cx);
        }
        cx.notify();
    }

    fn quick_find_input_has_focus(&self, window: &Window, cx: &App) -> bool {
        self.quick_find.open && self.quick_find.query.focus_handle(cx).is_focused(window)
    }

    fn close_quick_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let target = self.quick_find.close();
        self.refresh_quick_find_highlights(cx);
        match target {
            Some(QuickFindTarget::Log(document_id)) => self
                .documents
                .iter()
                .find(|tab| tab.id == document_id)
                .map(|tab| tab.log_focus_handle.clone())
                .unwrap_or_else(|| self.focus_handle.clone())
                .focus(window, cx),
            Some(QuickFindTarget::Results(document_id)) => self
                .documents
                .iter()
                .find(|tab| tab.id == document_id)
                .map(|tab| tab.result_focus_handle.clone())
                .unwrap_or_else(|| self.focus_handle.clone())
                .focus(window, cx),
            Some(QuickFindTarget::GlobalResults) => {
                self.global_results_focus_handle.focus(window, cx)
            }
            None => self.focus_handle.focus(window, cx),
        }
        cx.notify();
    }

    fn cancel_quick_find_work(&mut self) {
        self.quick_find.cancel_work();
    }

    fn update_quick_find_matcher(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let query = self.quick_find.query.read(cx).value().to_string();
        match SearchMatcher::quick_find(
            &query,
            self.quick_find.case_sensitive,
            self.quick_find.whole_word,
            self.quick_find.regex,
        ) {
            Ok(matcher) => {
                self.quick_find.matcher = matcher;
                self.quick_find.error = None;
            }
            Err(error) => {
                self.quick_find.matcher = None;
                self.quick_find.error = Some(error.to_string().into());
            }
        }
        self.refresh_quick_find_highlights(cx);
    }

    fn focus_quick_find_input(&self, window: &mut Window, cx: &mut Context<Self>) {
        let focus = self.quick_find.query.focus_handle(cx);
        window.defer(cx, move |window, cx| focus.focus(window, cx));
    }

    fn toggle_quick_find_case_sensitive(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.quick_find.case_sensitive = !self.quick_find.case_sensitive;
        self.schedule_incremental_quick_find(window, cx);
        self.focus_quick_find_input(window, cx);
    }

    fn toggle_quick_find_whole_word(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.quick_find.whole_word = !self.quick_find.whole_word;
        self.schedule_incremental_quick_find(window, cx);
        self.focus_quick_find_input(window, cx);
    }

    fn toggle_quick_find_regex(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.quick_find.regex = !self.quick_find.regex;
        self.schedule_incremental_quick_find(window, cx);
        self.focus_quick_find_input(window, cx);
    }

    fn refresh_quick_find_highlights(&mut self, cx: &mut Context<Self>) {
        let matcher = self.quick_find.matcher.clone();
        for tab in &self.documents {
            tab.log_table.update(cx, |table, cx| {
                table.delegate_mut().set_quick_find_matcher(matcher.clone());
                table.refresh(cx);
            });
            tab.result_table.update(cx, |table, cx| {
                table.delegate_mut().set_quick_find_matcher(matcher.clone());
                table.refresh(cx);
            });
        }
        self.global_table.update(cx, |table, cx| {
            table.delegate_mut().set_quick_find_matcher(matcher);
            table.refresh(cx);
        });
    }

    fn schedule_incremental_quick_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.quick_find.open {
            return;
        }
        self.cancel_quick_find_work();
        self.quick_find.matched = None;
        self.quick_find.no_match = false;
        self.quick_find.boundary = None;
        self.update_quick_find_matcher(window, cx);
        if self.quick_find.query.read(cx).value().is_empty() || self.quick_find.matcher.is_none() {
            cx.notify();
            return;
        }
        let revision = self.quick_find.revision;
        self.quick_find.task = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(50))
                .await;
            _ = this.update_in(cx, |this, window, cx| {
                if this.quick_find.open && this.quick_find.revision == revision {
                    this.start_quick_find(QuickFindDirection::Forward, true, window, cx);
                }
            });
        }));
        cx.notify();
    }

    fn quick_find_source(
        &self,
        target: QuickFindTarget,
        cx: &App,
    ) -> Option<(QuickFindSource, usize)> {
        match target {
            QuickFindTarget::Log(document_id) => {
                let tab = self.documents.iter().find(|tab| tab.id == document_id)?;
                let row_count = tab.document.line_count();
                Some((
                    QuickFindSource::Document {
                        document: tab.document.clone(),
                        rows: None,
                        row_count,
                    },
                    row_count,
                ))
            }
            QuickFindTarget::Results(document_id) => {
                let tab = self.documents.iter().find(|tab| tab.id == document_id)?;
                let rows = tab.result_rows(cx);
                let row_count = rows.len();
                Some((
                    QuickFindSource::Document {
                        document: tab.document.clone(),
                        rows: Some(rows),
                        row_count,
                    },
                    row_count,
                ))
            }
            QuickFindTarget::GlobalResults => {
                let table = self.global_table.read(cx);
                let row_count = table.delegate().rows_len();
                Some((
                    QuickFindSource::Global(table.delegate().quick_find_groups()),
                    row_count,
                ))
            }
        }
    }

    fn start_quick_find(
        &mut self,
        direction: QuickFindDirection,
        incremental: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.quick_find.open {
            return;
        }
        let query = self.quick_find.query.read(cx).value().to_string();
        if query.is_empty() {
            self.cancel_quick_find_work();
            self.quick_find.matched = None;
            self.quick_find.no_match = false;
            self.quick_find.boundary = None;
            cx.notify();
            return;
        }
        let Some(target) = self.quick_find.target else {
            return;
        };
        let Some(matcher) = self.quick_find.matcher.clone() else {
            self.update_quick_find_matcher(window, cx);
            let Some(matcher) = self.quick_find.matcher.clone() else {
                return;
            };
            return self.start_quick_find_with_matcher(
                target,
                matcher,
                direction,
                incremental,
                window,
                cx,
            );
        };
        self.start_quick_find_with_matcher(target, matcher, direction, incremental, window, cx);
    }

    fn start_quick_find_with_matcher(
        &mut self,
        target: QuickFindTarget,
        matcher: SearchMatcher,
        direction: QuickFindDirection,
        incremental: bool,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((source, row_count)) = self.quick_find_source(target, cx) else {
            return;
        };
        if row_count == 0 {
            self.quick_find.busy = false;
            self.quick_find.direction = None;
            self.quick_find.no_match |= incremental;
            self.quick_find.boundary = Some(match direction {
                QuickFindDirection::Forward => QuickFindBoundary::End,
                QuickFindDirection::Backward => QuickFindBoundary::Start,
            });
            cx.notify();
            return;
        }

        let requested_boundary = match direction {
            QuickFindDirection::Forward => QuickFindBoundary::End,
            QuickFindDirection::Backward => QuickFindBoundary::Start,
        };
        if !incremental && self.quick_find.boundary == Some(requested_boundary) {
            self.quick_find.busy = false;
            self.quick_find.direction = None;
            return;
        }

        let current_match = (!incremental)
            .then_some(self.quick_find.matched)
            .flatten()
            .filter(|matched| matched.target == target);
        let start = match (direction, current_match) {
            (QuickFindDirection::Forward, Some(matched)) => {
                (matched.view_row + 1 < row_count).then_some(matched.view_row + 1)
            }
            (QuickFindDirection::Backward, Some(matched)) => matched.view_row.checked_sub(1),
            (QuickFindDirection::Forward, None) => Some(self.quick_find.anchor.min(row_count - 1)),
            (QuickFindDirection::Backward, None) => {
                self.quick_find.anchor.min(row_count - 1).checked_sub(1)
            }
        };
        let Some(start) = start else {
            self.quick_find.busy = false;
            self.quick_find.direction = None;
            self.quick_find.no_match |= incremental;
            self.quick_find.boundary = Some(match direction {
                QuickFindDirection::Forward => QuickFindBoundary::End,
                QuickFindDirection::Backward => QuickFindBoundary::Start,
            });
            cx.notify();
            return;
        };

        self.cancel_quick_find_work();
        let revision = self.quick_find.revision;
        let cancellation = SearchCancellation::default();
        self.quick_find.cancellation = Some(cancellation.clone());
        self.quick_find.busy = true;
        self.quick_find.direction = Some(direction);
        self.quick_find.boundary = None;
        cx.notify();
        self.quick_find.task = Some(cx.spawn(async move |this, cx| {
            let matched = cx
                .background_spawn(async move {
                    Self::find_quick_match(source, target, matcher, direction, start, cancellation)
                })
                .await;
            _ = this.update(cx, |this, cx| {
                if !this.quick_find.open
                    || this.quick_find.revision != revision
                    || this.quick_find.target != Some(target)
                {
                    return;
                }
                this.quick_find.busy = false;
                this.quick_find.direction = None;
                this.quick_find.cancellation = None;
                this.quick_find.task = None;
                match matched {
                    Some(matched) => {
                        this.quick_find.matched = Some(matched);
                        this.quick_find.no_match = false;
                        this.quick_find.boundary = None;
                        this.apply_quick_find_match(matched, cx);
                    }
                    None => {
                        this.quick_find.no_match |= incremental;
                        this.quick_find.boundary = Some(match direction {
                            QuickFindDirection::Forward => QuickFindBoundary::End,
                            QuickFindDirection::Backward => QuickFindBoundary::Start,
                        });
                    }
                }
                cx.notify();
            });
        }));
    }

    fn find_quick_match(
        source: QuickFindSource,
        target: QuickFindTarget,
        matcher: SearchMatcher,
        direction: QuickFindDirection,
        start: usize,
        cancellation: SearchCancellation,
    ) -> Option<QuickFindMatch> {
        let mut inspected = 0_usize;
        let mut inspect = |view_row: usize, source_row: usize, document: &Arc<LogDocument>| {
            inspected = inspected.saturating_add(1);
            if inspected & 0x3ff == 0 && cancellation.is_cancelled() {
                return None;
            }
            let line = document.line(source_row)?;
            (!matcher.matching_ranges(&line).is_empty()).then_some(QuickFindMatch {
                target,
                view_row,
                source_row,
            })
        };

        match source {
            QuickFindSource::Document {
                document,
                rows,
                row_count,
            } => match direction {
                QuickFindDirection::Forward => {
                    for view_row in start..row_count {
                        if cancellation.is_cancelled() {
                            return None;
                        }
                        let source_row = rows
                            .as_ref()
                            .and_then(|rows| rows.get(view_row))
                            .unwrap_or(view_row);
                        if let Some(matched) = inspect(view_row, source_row, &document) {
                            return Some(matched);
                        }
                    }
                    None
                }
                QuickFindDirection::Backward => {
                    if row_count == 0 {
                        return None;
                    }
                    for view_row in (0..=start.min(row_count - 1)).rev() {
                        if cancellation.is_cancelled() {
                            return None;
                        }
                        let source_row = rows
                            .as_ref()
                            .and_then(|rows| rows.get(view_row))
                            .unwrap_or(view_row);
                        if let Some(matched) = inspect(view_row, source_row, &document) {
                            return Some(matched);
                        }
                    }
                    None
                }
            },
            QuickFindSource::Global(groups) => match direction {
                QuickFindDirection::Forward => {
                    for group in &groups {
                        let first = start.saturating_sub(group.view_start).min(group.rows.len());
                        for result_ix in first..group.rows.len() {
                            let source_row = group.rows.get(result_ix)?;
                            let view_row = group.view_start.saturating_add(result_ix);
                            if let Some(matched) = inspect(view_row, source_row, &group.document) {
                                return Some(matched);
                            }
                        }
                    }
                    None
                }
                QuickFindDirection::Backward => {
                    for group in groups.iter().rev() {
                        if group.rows.is_empty() || start < group.view_start {
                            continue;
                        }
                        let last = start
                            .saturating_sub(group.view_start)
                            .min(group.rows.len().saturating_sub(1));
                        for result_ix in (0..=last).rev() {
                            let source_row = group.rows.get(result_ix)?;
                            let view_row = group.view_start.saturating_add(result_ix);
                            if let Some(matched) = inspect(view_row, source_row, &group.document) {
                                return Some(matched);
                            }
                        }
                    }
                    None
                }
            },
        }
    }

    fn apply_quick_find_match(&mut self, matched: QuickFindMatch, cx: &mut Context<Self>) {
        match matched.target {
            QuickFindTarget::Log(document_id) => {
                let Some(tab) = self.documents.iter_mut().find(|tab| tab.id == document_id) else {
                    return;
                };
                tab.auto_follow = false;
                tab.selection_table = SelectionTable::Log;
                tab.log_table.update(cx, |table, cx| {
                    table.set_active_log_row(matched.view_row, cx);
                });
                tab.log_viewport.center_row(matched.view_row);
                self.active_log_region = LogRegion::Body;
                self.selected_source_row = Some(matched.source_row);
            }
            QuickFindTarget::Results(document_id) => {
                let Some(tab) = self.documents.iter_mut().find(|tab| tab.id == document_id) else {
                    return;
                };
                tab.auto_follow = false;
                tab.selection_table = SelectionTable::Results;
                tab.result_table.update(cx, |table, cx| {
                    table.set_active_log_row(matched.view_row, cx);
                });
                tab.result_viewport.center_row(matched.view_row);
                self.active_log_region = LogRegion::CurrentResults;
                self.selected_source_row = Some(matched.source_row);
            }
            QuickFindTarget::GlobalResults => {
                self.global_table.update(cx, |table, cx| {
                    table.set_active_log_row(matched.view_row, cx);
                });
                self.global_viewport.center_row(matched.view_row);
                self.active_log_region = LogRegion::GlobalResults;
            }
        }
    }

    fn open_global_search_files_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.documents.is_empty() {
            return;
        }
        let files = self
            .documents
            .iter()
            .map(|tab| GlobalSearchFileOption {
                document_id: tab.id,
                title: tab.title.clone(),
                path: tab.document.path().to_path_buf(),
                opened_at: tab.opened_at,
                selected: self.global_search.selected_documents.contains(&tab.id),
            })
            .collect::<Vec<_>>();
        let picker = cx.new(|_| GlobalSearchFilesDialog::new(files));
        let workspace = cx.entity();
        let dialog_width = large_dialog_size(window).width;
        window.open_dialog(cx, move |dialog, _, _| {
            let picker = picker.clone();
            let workspace = workspace.clone();
            dialog
                .w(dialog_width)
                .title(crate::tr!(
                    "参与多标签搜索的文件",
                    "Files in multi-tab search"
                ))
                .child(picker.clone())
                .footer(
                    DialogFooter::new()
                        .child(
                            DialogClose::new().child(
                                Button::new("global-search-files-dialog-cancel")
                                    .label(crate::tr!("取消", "Cancel")),
                            ),
                        )
                        .child(
                            DialogAction::new().child(
                                Button::new("global-search-files-dialog-save")
                                    .primary()
                                    .label(crate::tr!("保存", "Save")),
                            ),
                        ),
                )
                .on_ok(move |_, window, cx| {
                    let selected = picker.read(cx).selected_document_ids();
                    workspace.update(cx, |this, cx| {
                        this.apply_global_selected_documents(selected, window, cx)
                    });
                    true
                })
        });
    }

    fn open_directory_search_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picker = cx.new(|cx| {
            DirectorySearchDialog::new(self.global_search.directory_options.clone(), window, cx)
        });
        let workspace = cx.entity();
        window.open_dialog(cx, move |dialog, _, _| {
            let picker_for_submit = picker.clone();
            let workspace = workspace.clone();
            dialog
                .title(crate::tr!("目录搜索设置", "Directory search settings"))
                .child(picker.clone())
                .footer(
                    DialogFooter::new()
                        .child(
                            DialogClose::new().child(
                                Button::new("directory-search-dialog-cancel")
                                    .label(crate::tr!("取消", "Cancel")),
                            ),
                        )
                        .child(
                            DialogAction::new().child(
                                Button::new("directory-search-dialog-save")
                                    .primary()
                                    .label(crate::tr!("保存", "Save")),
                            ),
                        ),
                )
                .on_ok(move |_, window, cx| {
                    let Some(options) = picker_for_submit.read(cx).options(cx) else {
                        picker_for_submit
                            .update(cx, |picker, cx| picker.show_validation_errors(cx));
                        return false;
                    };
                    workspace.update(cx, |this, cx| {
                        this.apply_directory_search_options(options, window, cx)
                    });
                    true
                })
        });
    }

    fn apply_directory_search_options(
        &mut self,
        options: DirectorySearchOptions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.global_search.directory_options == options {
            return;
        }
        if self.global_search.scope == SearchScope::Directory
            && self.searches.has_target(SearchTarget::Directory)
        {
            self.cancel_search();
        }
        self.global_search.directory_options = options;
        self.global_search.pending_directory_restore = None;
        self.global_search.directory_context = RetainedGlobalSearchContext::default();
        self.global_search.clear_directory_document_ids();
        if self.global_search.result_scope == Some(SearchScope::Directory) {
            self.global_search.revision = self.global_search.revision.saturating_add(1);
            self.global_search.results_visible = false;
            self.global_search.results.clear();
            self.global_search.matcher = None;
            self.global_search.result_scope = None;
            self.refresh_global_result_rows(cx);
        }
        self.schedule_workspace_search_state_save(window, cx);
        cx.notify();
    }

    fn apply_global_selected_documents(
        &mut self,
        selected: BTreeSet<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let available = self
            .documents
            .iter()
            .map(|tab| tab.id)
            .collect::<BTreeSet<_>>();
        let selected = selected
            .intersection(&available)
            .copied()
            .collect::<BTreeSet<_>>();
        if self.global_search.selected_documents == selected {
            return;
        }
        if self.searches.has_target(SearchTarget::AllOpenFiles) {
            self.cancel_search();
        }
        self.global_search.revision = self.global_search.revision.saturating_add(1);
        self.global_search.selected_documents = selected;
        let preferences = self
            .documents
            .iter()
            .map(|tab| {
                let selected = self.global_search.selected_documents.contains(&tab.id);
                let path = tab.document.path().to_path_buf();
                self.global_search
                    .preferences
                    .insert(path.clone(), selected);
                (path, selected)
            })
            .collect::<Vec<_>>();
        if let Some(store) = self.persistence.store.clone() {
            self.persistence
                .state_tasks
                .push(cx.spawn(async move |this, cx| {
                    let result = cx
                        .background_spawn(async move {
                            store.save_global_search_preferences(&preferences)
                        })
                        .await;
                    if let Err(error) = result {
                        _ = this.update(cx, |_, cx| {
                            cx.notify();
                            log::error!("全局搜索参与偏好未能保存：{error}");
                        });
                    }
                }));
        }
        self.refresh_global_result_rows(cx);
        self.maybe_restore_persisted_search(window, cx);
        self.schedule_workspace_search_state_save(window, cx);
        cx.notify();
    }

    fn refresh_global_result_rows(&mut self, cx: &mut Context<Self>) {
        let word_wrap = self.global_viewport.is_wrapped();
        let row_height = self.log_row_height();
        let viewport_anchor = self.capture_global_viewport_anchor(row_height, cx);
        let measured_heights = {
            let table = self.global_table.read(cx);
            if word_wrap {
                self.global_viewport
                    .wrapped_measured_heights_by_key(|row_ix| table.delegate().row_key(row_ix))
            } else {
                BTreeMap::new()
            }
        };

        let groups = match self.global_search.scope {
            SearchScope::AllOpenFiles => self
                .documents
                .iter()
                .filter(|tab| self.global_search.selected_documents.contains(&tab.id))
                .map(|tab| {
                    let result = self.global_search.results.get(&tab.id);
                    let search_result = result.map(|result| &result.search_result);
                    GlobalSearchGroup {
                        source: crate::global_search_table::GlobalSearchGroupSource {
                            document_id: tab.id,
                            title: result
                                .map(|result| result.title.clone())
                                .unwrap_or_else(|| tab.title.clone()),
                            path: result
                                .map(|result| result.path.clone())
                                .unwrap_or_else(|| tab.document.path().to_path_buf()),
                            document: result
                                .map(|result| result.document.clone())
                                .unwrap_or_else(|| tab.document.clone()),
                        },
                        projection: crate::global_search_table::GlobalSearchGroupProjection {
                            rows: compute_result_rows(
                                self.global_search.result_mode,
                                search_result,
                                &tab.marked_rows,
                            ),
                        },
                        presentation: crate::global_search_table::GlobalSearchGroupPresentation {
                            matched_rows: search_result
                                .map(|result| result.line_indices.clone())
                                .unwrap_or_default(),
                            marked_rows: tab.marked_rows.clone(),
                            truncated: search_result.is_some_and(|result| result.truncated)
                                && self.global_search.result_mode.includes_matches(),
                            failure: result.and_then(|result| result.failure.clone()),
                            color_rules: tab.resolved_color_rules.clone(),
                        },
                    }
                })
                .collect::<Vec<_>>(),
            SearchScope::Directory
                if self.global_search.result_scope == Some(SearchScope::Directory) =>
            {
                let open_documents_by_path = self
                    .documents
                    .iter()
                    .map(|tab| (path_match_key(tab.document.path()), tab))
                    .collect::<BTreeMap<_, _>>();
                self.global_search
                    .results
                    .iter()
                    .filter_map(|(document_id, result)| {
                        let open_tab =
                            path_match_map_get(&open_documents_by_path, &result.path).copied();
                        let marked_rows = open_tab
                            .map(|tab| tab.marked_rows.clone())
                            .unwrap_or_default();
                        let rows = compute_result_rows(
                            self.global_search.result_mode,
                            Some(&result.search_result),
                            &marked_rows,
                        );
                        (!rows.is_empty() || result.failure.is_some()).then(|| GlobalSearchGroup {
                            source: crate::global_search_table::GlobalSearchGroupSource {
                                document_id: *document_id,
                                title: result.title.clone(),
                                path: result.path.clone(),
                                document: result.document.clone(),
                            },
                            projection: crate::global_search_table::GlobalSearchGroupProjection {
                                rows,
                            },
                            presentation:
                                crate::global_search_table::GlobalSearchGroupPresentation {
                                    matched_rows: result.search_result.line_indices.clone(),
                                    marked_rows,
                                    truncated: result.search_result.truncated
                                        && self.global_search.result_mode.includes_matches(),
                                    failure: result.failure.clone(),
                                    color_rules: open_tab
                                        .map(|tab| tab.resolved_color_rules.clone())
                                        .unwrap_or_else(|| {
                                            Arc::from(Vec::<ResolvedColorRule>::new())
                                        }),
                                },
                        })
                    })
                    .collect::<Vec<_>>()
            }
            SearchScope::CurrentFile | SearchScope::Directory => Vec::new(),
        };

        let matcher = (self.global_search.result_mode.includes_matches()
            && self.app_settings.highlight_matches)
            .then(|| self.global_search.matcher.clone())
            .flatten();
        self.global_search.restoring_selection = true;
        let active_restored = self.global_table.update(cx, |table, cx| {
            table.delegate_mut().set_groups(groups, matcher);
            let active_restored = table.sync_active_log_row(cx);
            table.refresh_log_rows(cx);
            active_restored
        });
        if word_wrap {
            let table = self.global_table.read(cx);
            self.global_viewport.reset_wrapped_with_remapped_heights(
                table.delegate().rows_len(),
                row_height,
                measured_heights,
                |key| table.delegate().row_ix_for_key(*key),
            );
        } else {
            self.global_viewport.invalidate_wrapped();
        }
        self.restore_global_viewport_anchor(viewport_anchor, row_height, cx);
        if !active_restored {
            self.global_search.restoring_selection = false;
        }
    }

    fn reset_search_history_navigation(&mut self) {
        self.search_history_ix = None;
        self.search_history_draft = None;
    }

    fn close_search_autocomplete(&mut self) {
        self.search_autocomplete_mode = SearchAutocompleteMode::Closed;
        self.search_suggestion_ix = None;
    }

    fn reset_search_suggestion_scroll(&self) {
        let base_handle = {
            let mut state = self.search_suggestion_scroll.0.borrow_mut();
            state.deferred_scroll_to_item = None;
            state.base_handle.clone()
        };
        base_handle.set_offset(point(px(0.), px(0.)));
    }

    fn search_autocomplete_suggestions(&self, cx: &App) -> Vec<SearchSuggestion> {
        match self.search_autocomplete_mode {
            SearchAutocompleteMode::Closed => Vec::new(),
            SearchAutocompleteMode::Matches => {
                let query = self.query.read(cx).value().to_string();
                if search_autocomplete_needle(&query).is_empty() {
                    Vec::new()
                } else {
                    search_autocomplete_suggestions(
                        &self.search_history,
                        &self.predefined_filters,
                        &query,
                        100,
                    )
                }
            }
            SearchAutocompleteMode::History => {
                search_autocomplete_suggestions(&self.search_history, &[], "", usize::MAX)
            }
        }
    }

    fn accept_active_search_suggestion(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let suggestions = self.search_autocomplete_suggestions(cx);
        let Some(suggestion) = self
            .search_suggestion_ix
            .and_then(|ix| suggestions.get(ix))
            .cloned()
        else {
            return false;
        };
        self.accept_search_suggestion(suggestion, window, cx);
        true
    }

    fn refresh_search_autocomplete(&mut self, cx: &mut Context<Self>) {
        let query = self.query.read(cx).value().to_string();
        let has_input = !search_autocomplete_needle(&query).is_empty();
        let has_suggestions = has_input
            && !search_autocomplete_suggestions(
                &self.search_history,
                &self.predefined_filters,
                &query,
                1,
            )
            .is_empty();
        self.search_autocomplete_mode = if has_suggestions {
            SearchAutocompleteMode::Matches
        } else {
            SearchAutocompleteMode::Closed
        };
        self.search_suggestion_ix = None;
        self.reset_search_suggestion_scroll();
        cx.notify();
    }

    fn accept_search_suggestion(
        &mut self,
        suggestion: SearchSuggestion,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let current = self.query.read(cx).value().to_string();
        let select_all = self.search_autocomplete_mode == SearchAutocompleteMode::History;
        let next = match self.search_autocomplete_mode {
            SearchAutocompleteMode::History => suggestion.value,
            SearchAutocompleteMode::Matches => apply_search_suggestion(&current, &suggestion.value),
            SearchAutocompleteMode::Closed => return,
        };
        self.reset_search_history_navigation();
        self.query.update(cx, |state, cx| {
            state.set_value(next, window, cx);
            if select_all {
                state.select_all(window, cx);
            }
        });
        self.close_search_autocomplete();
        self.query.focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    fn toggle_search_history_popup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.search_history.is_empty() {
            return;
        }
        self.reset_search_history_navigation();
        if self.search_autocomplete_mode == SearchAutocompleteMode::History {
            self.close_search_autocomplete();
        } else {
            self.search_autocomplete_mode = SearchAutocompleteMode::History;
            self.search_suggestion_ix = None;
            self.reset_search_suggestion_scroll();
        }
        self.query.focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    fn navigate_search_autocomplete_by_key(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        match key {
            "escape" if self.search_autocomplete_mode != SearchAutocompleteMode::Closed => {
                self.close_search_autocomplete();
                cx.notify();
                true
            }
            "up" | "down" => {
                if self.search_autocomplete_mode == SearchAutocompleteMode::Closed {
                    if self.search_history.is_empty() {
                        return false;
                    }
                    self.search_autocomplete_mode = SearchAutocompleteMode::History;
                    self.search_suggestion_ix = Some(if key == "down" {
                        0
                    } else {
                        self.search_history.len() - 1
                    });
                } else {
                    let suggestion_count = self.search_autocomplete_suggestions(cx).len();
                    if suggestion_count == 0 {
                        self.close_search_autocomplete();
                        cx.notify();
                        return false;
                    }
                    self.search_suggestion_ix = Some(match (key, self.search_suggestion_ix) {
                        ("down", Some(ix)) if ix + 1 < suggestion_count => ix + 1,
                        ("down", _) => 0,
                        ("up", Some(ix)) if ix > 0 && ix < suggestion_count => ix - 1,
                        ("up", _) => suggestion_count - 1,
                        _ => unreachable!(),
                    });
                }
                if let Some(ix) = self.search_suggestion_ix {
                    self.search_suggestion_scroll
                        .scroll_to_item(ix, ScrollStrategy::Nearest);
                }
                cx.notify();
                true
            }
            _ => false,
        }
    }

    fn set_query_from_search_history(
        &mut self,
        query: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.query
            .update(cx, |state, cx| state.set_value(query, window, cx));
        self.close_search_autocomplete();
        self.query.focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    fn navigate_search_history_by_wheel(
        &mut self,
        wheel_up: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.search_history.is_empty() {
            return false;
        }
        if self.search_history_ix.is_none() {
            self.search_history_draft = Some(self.query.read(cx).value().to_string());
        }
        let next_ix = match self.search_history_ix {
            None => 0,
            Some(ix) if wheel_up => ix.saturating_sub(1),
            Some(ix) => (ix + 1).min(self.search_history.len() - 1),
        };
        self.search_history_ix = Some(next_ix);
        self.set_query_from_search_history(self.search_history[next_ix].clone(), window, cx);
        true
    }

    fn choose_predefined_filter(
        &mut self,
        filter: PredefinedFilter,
        checked: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let current = self.query.read(cx).value().to_string();
        let next = toggle_filter_in_query(&current, &filter, checked);
        self.reset_search_history_navigation();
        self.query.update(cx, |state, cx| {
            state.set_value(next.clone(), window, cx);
        });
        self.close_search_autocomplete();
        if checked && filter.use_regex {
            self.set_search_defaults(self.case_sensitive, true, window, cx);
        }
        let checkpoint = match self.global_search.scope {
            SearchScope::CurrentFile => self.active_ix.map(|active_ix| {
                let tab = &mut self.documents[active_ix];
                tab.search_query.text = next;
                tab.search_query.regex = self.regex;
                tab.id
            }),
            SearchScope::AllOpenFiles => {
                self.global_search.query.text = next;
                self.global_search.query.regex = self.regex;
                None
            }
            SearchScope::Directory => {
                self.global_search.directory_query.text = next;
                self.global_search.directory_query.regex = self.regex;
                None
            }
        };
        if let Some(document_id) = checkpoint {
            self.schedule_checkpoint(document_id, window, cx);
        }
        cx.notify();
    }

    fn apply_search_history(&mut self, history: Vec<String>, cx: &mut Context<Self>) {
        self.search_history = normalize_search_history(history);
        self.reset_search_history_navigation();
        self.search_suggestion_ix = None;
        self.reset_search_suggestion_scroll();
        if self.search_autocomplete_mode == SearchAutocompleteMode::Matches {
            self.refresh_search_autocomplete(cx);
        } else {
            if self.search_history.is_empty()
                && self.search_autocomplete_mode == SearchAutocompleteMode::History
            {
                self.close_search_autocomplete();
            }
            cx.notify();
        }
    }

    fn replace_search_history(
        &mut self,
        history: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let history = normalize_search_history(history);
        if history == self.search_history {
            return;
        }

        self.apply_search_history(history.clone(), cx);
        let source_window = window.window_handle();
        let other_workspaces = cx
            .global::<WorkspaceWindowRegistry>()
            .windows
            .iter()
            .filter(|entry| entry.window != source_window)
            .map(|entry| entry.workspace.clone())
            .collect::<Vec<_>>();
        for workspace in other_workspaces {
            let shared_history = history.clone();
            workspace.update(cx, |workspace, cx| {
                workspace.apply_search_history(shared_history, cx);
            });
        }

        let Some(store) = self.persistence.store.clone() else {
            return;
        };
        let previous_save = self.persistence.search_history_save_task.take();
        self.persistence.search_history_save_task =
            Some(cx.spawn_in(window, async move |this, cx| {
                if let Some(previous_save) = previous_save {
                    previous_save.await;
                }
                let result = cx
                    .background_spawn(async move { store.save_search_history(&history) })
                    .await;
                if let Err(error) = result {
                    _ = this.update_in(cx, |_, window, cx| {
                        window.push_notification(
                            crate::tr_args!(
                                "搜索历史未能保存：{error}",
                                "Couldn’t save search history: {error}"
                            ),
                            cx,
                        );
                    });
                }
            }));
    }

    fn remove_search_history_entries(
        &mut self,
        removed: &[String],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if removed.is_empty() {
            return;
        }
        let removed = removed.iter().map(String::as_str).collect::<HashSet<_>>();
        let history = self
            .search_history
            .iter()
            .filter(|query| !removed.contains(query.as_str()))
            .cloned()
            .collect();
        self.replace_search_history(history, window, cx);
    }

    fn record_search_history(&mut self, query: &str, window: &mut Window, cx: &mut Context<Self>) {
        if query.is_empty() {
            return;
        }
        let history = std::iter::once(query.to_string())
            .chain(self.search_history.iter().cloned())
            .collect();
        self.replace_search_history(history, window, cx);
    }

    fn persisted_search_query(query: &SearchQuery) -> PersistedSearchQuery {
        PersistedSearchQuery {
            text: query.text.clone(),
            case_sensitive: query.case_sensitive,
            regex: query.regex,
        }
    }

    fn restored_search_query(
        query: &PersistedSearchQuery,
        fallback_limit: Option<usize>,
    ) -> SearchQuery {
        SearchQuery {
            text: query.text.clone(),
            case_sensitive: query.case_sensitive,
            regex: query.regex,
            max_results: fallback_limit,
        }
    }

    fn global_context_path<'a>(
        &'a self,
        context: &'a RetainedGlobalSearchContext,
        document_id: u64,
    ) -> Option<&'a std::path::Path> {
        context
            .results
            .get(&document_id)
            .map(|result| result.path.as_path())
            .or_else(|| {
                self.documents
                    .iter()
                    .find(|tab| tab.id == document_id)
                    .map(|tab| tab.document.path())
            })
    }

    fn persisted_global_context(
        &self,
        scope: SearchScope,
        context: &RetainedGlobalSearchContext,
        pending: Option<&PersistedGlobalSearchContext>,
    ) -> PersistedGlobalSearchContext {
        let query = match scope {
            SearchScope::AllOpenFiles => &self.global_search.query,
            SearchScope::Directory => &self.global_search.directory_query,
            SearchScope::CurrentFile => return PersistedGlobalSearchContext::default(),
        };
        if !context.initialized
            && context.results.is_empty()
            && let Some(pending) = pending
        {
            let mut persisted = pending.clone();
            persisted.query = Self::persisted_search_query(query);
            persisted.result_mode = context.result_mode.database_value();
            persisted.results_visible = context.results_visible;
            persisted.word_wrap = context.word_wrap;
            persisted.active = context.active;
            return persisted;
        }

        let collapsed_paths = context
            .collapsed_document_ids
            .iter()
            .filter_map(|document_id| self.global_context_path(context, *document_id))
            .map(|path| path.to_string_lossy().into_owned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let selection = context
            .selection
            .iter()
            .filter_map(|(document_id, rows)| {
                self.global_context_path(context, *document_id)
                    .map(|path| PersistedPathSelection {
                        path: path.to_string_lossy().into_owned(),
                        rows: compress_rows(rows.iter()),
                    })
            })
            .collect();
        let row_key = |key: LogRowKey| {
            let (document_id, source_row) = match key {
                LogRowKey::Row {
                    document_id,
                    source_row,
                } => (document_id, Some(source_row)),
                LogRowKey::FileGroup { document_id } => (document_id, None),
            };
            self.global_context_path(context, document_id)
                .map(|path| PersistedSearchRowKey {
                    path: path.to_string_lossy().into_owned(),
                    source_row,
                })
        };
        let selected_row = context.selected_row.and_then(|row| match row {
            GlobalSearchRow::Group { document_id } => row_key(LogRowKey::FileGroup { document_id }),
            GlobalSearchRow::Match {
                document_id,
                source_row,
            } => row_key(LogRowKey::Row {
                document_id,
                source_row,
            }),
        });
        let viewport = context.viewport.and_then(|viewport| {
            row_key(viewport.key).map(|key| {
                PersistedSearchViewport::new(
                    key,
                    viewport.viewport_y.as_f32(),
                    context.horizontal_offset,
                    viewport.at_end,
                    viewport.fallback_ix,
                )
            })
        });
        PersistedGlobalSearchContext {
            query: Self::persisted_search_query(query),
            result_mode: context.result_mode.database_value(),
            results_visible: context.results_visible,
            word_wrap: context.word_wrap,
            collapsed_paths,
            selection,
            selected_row,
            viewport,
            active: context.active,
            ..PersistedGlobalSearchContext::default()
        }
    }

    fn workspace_search_state(&self) -> WorkspaceSearchState {
        WorkspaceSearchState {
            active_scope: match self.global_search.scope {
                SearchScope::CurrentFile => PersistedSearchScope::CurrentFile,
                SearchScope::AllOpenFiles => PersistedSearchScope::AllOpenFiles,
                SearchScope::Directory => PersistedSearchScope::Directory,
            },
            all_open: self.persisted_global_context(
                SearchScope::AllOpenFiles,
                &self.global_search.all_open_context,
                self.global_search.pending_all_open_restore.as_ref(),
            ),
            directory: self.persisted_global_context(
                SearchScope::Directory,
                &self.global_search.directory_context,
                self.global_search.pending_directory_restore.as_ref(),
            ),
            directory_options: PersistedDirectorySearchOptions {
                directory: self
                    .global_search
                    .directory_options
                    .directory
                    .as_deref()
                    .map(|path| path.to_string_lossy().into_owned()),
                file_type: 0,
                file_type_filter_enabled: Some(
                    self.global_search
                        .directory_options
                        .file_type_filter_enabled,
                ),
                file_type_patterns: Some(
                    self.global_search
                        .directory_options
                        .file_type_patterns
                        .clone(),
                ),
                include_subdirectories: self.global_search.directory_options.include_subdirectories,
                include_hidden_directories: self
                    .global_search
                    .directory_options
                    .include_hidden_directories,
            },
            ..WorkspaceSearchState::default()
        }
    }

    fn queue_workspace_search_state_save(
        &mut self,
        state: WorkspaceSearchState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.primary_window {
            return;
        }
        let Some(store) = self.persistence.store.clone() else {
            self.persistence.pending_workspace_search_save = Some(state);
            return;
        };
        let previous_save = self.persistence.search_context_save_task.take();
        self.persistence.search_context_save_task =
            Some(cx.spawn_in(window, async move |this, cx| {
                if let Some(previous_save) = previous_save {
                    previous_save.await;
                }
                let result = cx
                    .background_spawn(async move { store.save_workspace_search_state(&state) })
                    .await;
                if let Err(error) = result {
                    _ = this.update_in(cx, |_, window, cx| {
                        window.push_notification(
                            crate::tr_args!(
                                "搜索状态未能保存：{error}",
                                "Couldn’t save search state: {error}"
                            ),
                            cx,
                        );
                    });
                }
            }));
    }

    fn schedule_workspace_search_state_save(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.capture_retained_global_context(self.global_search.scope, cx);
        let state = self.workspace_search_state();
        if self.global_search.pending_all_open_restore.is_some() {
            self.global_search.pending_all_open_restore = Some(state.all_open.clone());
        }
        if self.global_search.pending_directory_restore.is_some() {
            self.global_search.pending_directory_restore = Some(state.directory.clone());
        }
        self.queue_workspace_search_state_save(state, window, cx);
    }

    fn apply_persisted_workspace_search_state(
        &mut self,
        state: WorkspaceSearchState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let search_result_limit = self.app_settings.search_result_limit();
        self.global_search.query =
            Self::restored_search_query(&state.all_open.query, search_result_limit);
        self.global_search.directory_query =
            Self::restored_search_query(&state.directory.query, search_result_limit);
        let (legacy_file_type_filter_enabled, legacy_file_type_patterns) =
            DirectorySearchOptions::from_legacy_file_type(state.directory_options.file_type);
        self.global_search.directory_options = DirectorySearchOptions {
            directory: state
                .directory_options
                .directory
                .as_deref()
                .map(PathBuf::from),
            file_type_filter_enabled: state
                .directory_options
                .file_type_filter_enabled
                .unwrap_or(legacy_file_type_filter_enabled),
            file_type_patterns: state
                .directory_options
                .file_type_patterns
                .unwrap_or(legacy_file_type_patterns),
            include_subdirectories: state.directory_options.include_subdirectories,
            include_hidden_directories: state.directory_options.include_hidden_directories,
        };
        self.global_search.all_open_context = RetainedGlobalSearchContext {
            result_mode: ResultMode::from_database(state.all_open.result_mode),
            results_visible: state.all_open.results_visible,
            word_wrap: state.all_open.word_wrap,
            active: state.all_open.active,
            ..RetainedGlobalSearchContext::default()
        };
        self.global_search.directory_context = RetainedGlobalSearchContext {
            result_mode: ResultMode::from_database(state.directory.result_mode),
            results_visible: state.directory.results_visible,
            word_wrap: state.directory.word_wrap,
            active: state.directory.active,
            ..RetainedGlobalSearchContext::default()
        };
        self.global_search.pending_all_open_restore =
            state.all_open.results_visible.then_some(state.all_open);
        self.global_search.pending_directory_restore =
            state.directory.results_visible.then_some(state.directory);
        self.global_search.scope = match state.active_scope {
            PersistedSearchScope::CurrentFile => SearchScope::CurrentFile,
            PersistedSearchScope::AllOpenFiles => SearchScope::AllOpenFiles,
            PersistedSearchScope::Directory => SearchScope::Directory,
        };
        if matches!(
            self.global_search.scope,
            SearchScope::AllOpenFiles | SearchScope::Directory
        ) {
            self.restore_retained_global_context(self.global_search.scope, window, cx);
        }
        let text = match self.global_search.scope {
            SearchScope::CurrentFile => self
                .active_document()
                .map(|tab| tab.search_query.text.clone())
                .unwrap_or_default(),
            SearchScope::AllOpenFiles => self.global_search.query.text.clone(),
            SearchScope::Directory => self.global_search.directory_query.text.clone(),
        };
        self.query
            .update(cx, |query, cx| query.set_value(text, window, cx));
    }

    fn global_document_id_for_path(&self, path: &str) -> Option<u64> {
        self.global_search
            .results
            .iter()
            .find_map(|(document_id, result)| {
                Self::persisted_path_matches(&result.path, path).then_some(*document_id)
            })
            .or_else(|| {
                self.documents
                    .iter()
                    .find(|tab| Self::persisted_path_matches(tab.document.path(), path))
                    .map(|tab| tab.id)
            })
    }

    fn persisted_path_matches(actual: &std::path::Path, persisted: &str) -> bool {
        if actual == std::path::Path::new(persisted) {
            return true;
        }
        cfg!(windows) && actual.to_string_lossy().eq_ignore_ascii_case(persisted)
    }

    fn restore_persisted_global_presentation(
        &mut self,
        scope: SearchScope,
        persisted: PersistedGlobalSearchContext,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let collapsed_document_ids = persisted
            .collapsed_paths
            .iter()
            .filter_map(|path| self.global_document_id_for_path(path))
            .collect();
        let selection = persisted
            .selection
            .iter()
            .filter_map(|selection| {
                let document_id = self.global_document_id_for_path(&selection.path)?;
                let rows = CompressedRows::from_inclusive_ranges(selection.rows.iter().filter_map(
                    |range| {
                        Some((
                            usize::try_from(range.start).ok()?,
                            usize::try_from(range.end).ok()?,
                        ))
                    },
                ));
                Some((document_id, rows))
            })
            .collect();
        let restore_key = |key: &PersistedSearchRowKey| {
            let document_id = self.global_document_id_for_path(&key.path)?;
            Some(match key.source_row {
                Some(source_row) => LogRowKey::Row {
                    document_id,
                    source_row,
                },
                None => LogRowKey::FileGroup { document_id },
            })
        };
        let selected_row = persisted.selected_row.as_ref().and_then(|key| {
            restore_key(key).map(|key| match key {
                LogRowKey::Row {
                    document_id,
                    source_row,
                } => GlobalSearchRow::Match {
                    document_id,
                    source_row,
                },
                LogRowKey::FileGroup { document_id } => GlobalSearchRow::Group { document_id },
            })
        });
        let fallback_viewport_key = persisted.viewport.as_ref().and_then(|viewport| {
            let table = self.global_table.read(cx);
            let row_count = table.delegate().rows_len();
            let row_ix = viewport.fallback_ix.min(row_count.checked_sub(1)?);
            table.delegate().row(row_ix).map(|row| match row {
                GlobalSearchRow::Group { document_id } => LogRowKey::FileGroup { document_id },
                GlobalSearchRow::Match {
                    document_id,
                    source_row,
                } => LogRowKey::Row {
                    document_id,
                    source_row,
                },
            })
        });
        let viewport = persisted.viewport.as_ref().and_then(|viewport| {
            restore_key(&viewport.key)
                .or(fallback_viewport_key)
                .map(|key| ViewportAnchor {
                    key,
                    viewport_y: px(viewport.viewport_y()),
                    at_end: viewport.at_end,
                    fallback_ix: viewport.fallback_ix,
                })
        });
        let context = RetainedGlobalSearchContext {
            initialized: true,
            results: self.global_search.results.clone(),
            matcher: self.global_search.matcher.clone(),
            result_mode: ResultMode::from_database(persisted.result_mode),
            results_visible: persisted.results_visible,
            collapsed_document_ids,
            selection,
            selected_row,
            viewport,
            horizontal_offset: persisted
                .viewport
                .as_ref()
                .map_or(0., PersistedSearchViewport::horizontal_offset),
            word_wrap: persisted.word_wrap,
            active: persisted.active,
        };
        match scope {
            SearchScope::AllOpenFiles => {
                self.global_search.all_open_context = context;
                self.global_search.pending_all_open_restore = None;
            }
            SearchScope::Directory => {
                self.global_search.directory_context = context;
                self.global_search.pending_directory_restore = None;
            }
            SearchScope::CurrentFile => return,
        }
        self.restore_retained_global_context(scope, window, cx);
    }

    fn maybe_restore_persisted_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.searches.is_active() || self.open_task.is_some() {
            return;
        }
        match self.global_search.scope {
            SearchScope::AllOpenFiles => {
                if self.global_search.pending_all_open_restore.is_none()
                    || self.documents.is_empty()
                    || self.documents.iter().any(|tab| {
                        self.global_search.selected_documents.contains(&tab.id)
                            && tab.load_state != DocumentLoadState::Ready
                    })
                {
                    return;
                }
                let query_text = self.global_search.query.text.clone();
                self.query
                    .update(cx, |query, cx| query.set_value(query_text, window, cx));
                self.start_global_search(window, cx);
            }
            SearchScope::Directory => {
                if self.global_search.pending_directory_restore.is_none()
                    || self.global_search.directory_options.directory.is_none()
                {
                    return;
                }
                let query_text = self.global_search.directory_query.text.clone();
                self.query
                    .update(cx, |query, cx| query.set_value(query_text, window, cx));
                self.start_directory_search(window, cx);
            }
            SearchScope::CurrentFile => {}
        }
    }

    fn capture_retained_global_context(&mut self, scope: SearchScope, cx: &App) {
        if !matches!(scope, SearchScope::AllOpenFiles | SearchScope::Directory) {
            return;
        }
        let row_height = self.log_row_height();
        let (collapsed_document_ids, selection, selected_row) = {
            let table = self.global_table.read(cx);
            let selected_row = table
                .active_log_row()
                .and_then(|row_ix| table.delegate().row(row_ix));
            (
                table.delegate().collapsed_document_ids(),
                table.delegate().selection_snapshot(),
                selected_row,
            )
        };
        let context = RetainedGlobalSearchContext {
            initialized: self.global_search.result_scope == Some(scope),
            results: self.global_search.results.clone(),
            matcher: self.global_search.matcher.clone(),
            result_mode: self.global_search.result_mode,
            results_visible: self.global_search.results_visible,
            collapsed_document_ids,
            selection,
            selected_row,
            viewport: self.capture_global_viewport_anchor(row_height, cx),
            horizontal_offset: self.global_viewport.horizontal_offset().as_f32(),
            word_wrap: self.global_viewport.is_wrapped(),
            active: self.active_log_region == LogRegion::GlobalResults,
        };
        match scope {
            SearchScope::AllOpenFiles => self.global_search.all_open_context = context,
            SearchScope::Directory => self.global_search.directory_context = context,
            SearchScope::CurrentFile => {}
        }
    }

    fn restore_retained_global_context(
        &mut self,
        scope: SearchScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let context = match scope {
            SearchScope::AllOpenFiles => self.global_search.all_open_context.clone(),
            SearchScope::Directory => self.global_search.directory_context.clone(),
            SearchScope::CurrentFile => return,
        };
        self.global_search.results = context.results.clone();
        self.global_search.matcher = context.matcher.clone();
        self.global_search.result_mode = context.result_mode;
        self.global_search.results_visible = context.results_visible;
        self.global_viewport.set_word_wrap(context.word_wrap);
        self.global_search.result_scope = context.initialized.then_some(scope);
        self.global_search
            .result_mode_select
            .update(cx, |select, cx| {
                select.set_selected_index(
                    Some(IndexPath::new(context.result_mode.select_index())),
                    window,
                    cx,
                );
            });
        self.refresh_global_result_rows(cx);

        self.global_search.restoring_selection = context.selected_row.is_some();
        let selected_restored = self.global_table.update(cx, |table, cx| {
            table
                .delegate_mut()
                .restore_collapsed_document_ids(&context.collapsed_document_ids);
            table.delegate().restore_selection(&context.selection);
            let selected_ix = context
                .selected_row
                .and_then(|row| table.delegate().row_ix(row));
            if let Some(selected_ix) = selected_ix {
                table.set_active_log_row(selected_ix, cx);
            } else {
                table.delegate().set_active_log_row(None);
                table.clear_selection(cx);
            }
            table.refresh(cx);
            selected_ix.is_some()
        });
        if !selected_restored {
            self.global_search.restoring_selection = false;
        }
        self.global_viewport.invalidate_wrapped();
        let word_wrap = self.global_viewport.is_wrapped();
        self.restore_global_viewport_anchor(context.viewport, self.log_row_height(), cx);
        if !word_wrap {
            let table = self.global_table.read(cx);
            let base = table.vertical_scroll_handle.0.borrow().base_handle.clone();
            let offset = base.offset();
            base.set_offset(point(-px(context.horizontal_offset), offset.y));
        }
        if context.active && context.results_visible {
            self.active_log_region = LogRegion::GlobalResults;
        } else if self.active_log_region == LogRegion::GlobalResults {
            self.active_log_region = LogRegion::Body;
        }
    }

    fn set_search_scope(
        &mut self,
        next_scope: SearchScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.global_search.scope == next_scope {
            return;
        }
        if self.searches.has_target(SearchTarget::AllOpenFiles)
            || self.searches.has_target(SearchTarget::Directory)
        {
            self.cancel_search();
        }
        let draft = self.query.read(cx).value().to_string();
        self.reset_search_history_navigation();
        match self.global_search.scope {
            SearchScope::CurrentFile => {
                if let Some(active_ix) = self.active_ix {
                    self.documents[active_ix].search_query.text = draft;
                }
            }
            SearchScope::AllOpenFiles => {
                self.global_search.query.text = draft;
            }
            SearchScope::Directory => self.global_search.directory_query.text = draft,
        }
        self.capture_retained_global_context(self.global_search.scope, cx);
        self.global_search.scope = next_scope;
        if matches!(
            next_scope,
            SearchScope::AllOpenFiles | SearchScope::Directory
        ) {
            self.restore_retained_global_context(next_scope, window, cx);
        } else if self.active_log_region == LogRegion::GlobalResults {
            self.active_log_region = self
                .active_document()
                .filter(|tab| tab.results_visible && tab.selection_table == SelectionTable::Results)
                .map(|_| LogRegion::CurrentResults)
                .unwrap_or(LogRegion::Body);
        }
        let text = match next_scope {
            SearchScope::CurrentFile => self
                .active_document()
                .map(|tab| tab.search_query.text.clone())
                .unwrap_or_default(),
            SearchScope::AllOpenFiles => self.global_search.query.text.clone(),
            SearchScope::Directory => self.global_search.directory_query.text.clone(),
        };
        self.query
            .update(cx, |state, cx| state.set_value(text, window, cx));
        if next_scope == SearchScope::CurrentFile {
            self.refresh_global_result_rows(cx);
        }
        self.maybe_restore_persisted_search(window, cx);
        self.schedule_workspace_search_state_save(window, cx);
        self.close_search_autocomplete();
        self.query.focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    fn jump_to_global_result(
        &mut self,
        document_id: u64,
        source_row: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let result_path = self
            .global_search
            .results
            .get(&document_id)
            .map(|result| result.path.clone());
        let document_ix = self.documents.iter().position(|tab| {
            tab.id == document_id
                || result_path
                    .as_deref()
                    .is_some_and(|path| tab.document.path() == path)
        });
        let Some(document_ix) = document_ix else {
            let Some(path) = result_path else {
                return;
            };
            if self.open_task.is_some() {
                window.push_notification(
                    crate::tr!(
                        "当前正在打开其他文件，请稍后重试",
                        "Another file is being opened. Try again shortly."
                    ),
                    cx,
                );
                return;
            }
            self.pending_directory_result_jump = Some((path.clone(), source_row));
            self.begin_open_paths(vec![path], window, cx);
            return;
        };
        self.activate_tab(document_ix, window, cx);
        let Some(tab) = self.documents.get_mut(document_ix) else {
            return;
        };
        tab.auto_follow = false;
        tab.selection_table = SelectionTable::Log;
        tab.select_and_center_log_row(source_row, cx);
        self.selected_source_row = Some(source_row);
        cx.notify();
    }

    fn activate_global_group(
        &mut self,
        document_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let result_path = self
            .global_search
            .results
            .get(&document_id)
            .map(|result| result.path.clone());
        let document_ix = self.documents.iter().position(|tab| {
            tab.id == document_id
                || result_path
                    .as_deref()
                    .is_some_and(|path| tab.document.path() == path)
        });
        let Some(document_ix) = document_ix else {
            let Some(path) = result_path else {
                return;
            };
            if self.open_task.is_some() {
                window.push_notification(
                    crate::tr!(
                        "当前正在打开其他文件，请稍后重试",
                        "Another file is being opened. Try again shortly."
                    ),
                    cx,
                );
                return;
            }
            self.begin_open_paths(vec![path], window, cx);
            return;
        };
        self.activate_tab(document_ix, window, cx);
    }

    fn start_search_action(
        &mut self,
        _: &StartSearch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.start_search(window, cx);
    }

    fn start_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_search_autocomplete();
        match self.global_search.scope {
            SearchScope::CurrentFile => self.start_current_search(window, cx),
            SearchScope::AllOpenFiles => self.start_global_search(window, cx),
            SearchScope::Directory => self.start_directory_search(window, cx),
        }
    }

    fn start_current_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(active_ix) = self.active_ix else {
            return;
        };
        if self.documents[active_ix].load_state != DocumentLoadState::Ready {
            window.push_notification(
                crate::tr!(
                    "完整索引建立后即可搜索",
                    "Search will be available after the full index is built"
                ),
                cx,
            );
            return;
        }
        let text = self.query.read(cx).value().to_string();

        let query = SearchQuery {
            text,
            case_sensitive: self.case_sensitive,
            regex: self.regex,
            max_results: self.app_settings.search_result_limit(),
        };
        let row_height = self.log_row_height();
        let viewport_anchor = {
            let tab = &self.documents[active_ix];
            tab.results_visible
                .then(|| {
                    Self::capture_local_row_viewport_anchor(
                        tab,
                        WrappedRegion::Results,
                        row_height,
                        cx,
                    )
                })
                .flatten()
        };
        self.cancel_search();
        let cancellation = SearchCancellation::default();
        let tab = &mut self.documents[active_ix];
        tab.search_revision += 1;
        let revision = tab.search_revision;
        let document_id = tab.id;
        let document = tab.document.clone();
        let progress = SearchProgress::new(document.line_count());
        let target = SearchTarget::Document(document_id);
        self.searches.begin(target, revision, cancellation.clone());
        self.activity = Activity::Searching;
        cx.notify();

        let query_for_search = query.clone();
        let task = cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let matcher = SearchMatcher::new(&query_for_search)?;
                    let run = search_with_compiled_matcher(
                        &document,
                        matcher.as_ref(),
                        query_for_search.max_results,
                        &cancellation,
                        &progress,
                    );
                    Ok::<_, anyhow::Error>((run, matcher))
                })
                .await;

            _ = this.update_in(cx, |this, window, cx| {
                if !this.searches.is_current(target, revision) {
                    return;
                }
                if matches!(&result, Ok((SearchRun::Completed(_), _))) {
                    this.record_search_history(&query.text, window, cx);
                }
                let highlight_matches = this.app_settings.highlight_matches;
                let row_height = this.log_row_height();
                let Some(tab_ix) = this.documents.iter().position(|tab| tab.id == document_id)
                else {
                    return;
                };
                let tab = &mut this.documents[tab_ix];
                if tab.search_revision != revision {
                    return;
                }

                let prime_wrapped_results = match result {
                    Ok((SearchRun::Completed(result), search_matcher)) => {
                        tab.search_query = query;
                        tab.search_result = result;
                        tab.search_matcher = search_matcher;
                        tab.refresh_result_rows(cx);
                        Self::position_local_row_viewport_anchor(
                            tab,
                            WrappedRegion::Results,
                            viewport_anchor,
                            row_height,
                            cx,
                        );
                        tab.refresh_search_matcher(highlight_matches, cx);
                        tab.results_visible = true;
                        let word_wrap = tab.result_viewport.is_wrapped();
                        this.activity = Activity::Ready;
                        this.schedule_checkpoint(document_id, window, cx);
                        word_wrap
                    }
                    Ok((SearchRun::Cancelled, _)) => {
                        this.activity = Activity::Ready;
                        false
                    }
                    Err(error) => {
                        let message: SharedString = error.to_string().into();
                        window.push_notification(message.clone(), cx);
                        this.activity = Activity::Error;
                        false
                    }
                };
                if prime_wrapped_results {
                    this.prime_local_wrapped_frame(
                        tab_ix,
                        WrappedRegion::Results,
                        row_height,
                        false,
                        window,
                        cx,
                    );
                    Self::position_local_row_viewport_anchor(
                        &this.documents[tab_ix],
                        WrappedRegion::Results,
                        viewport_anchor,
                        row_height,
                        cx,
                    );
                }
                this.searches.finish(target, revision);
                cx.notify();
            });
        });
        self.searches.set_task(task);
    }

    fn install_completed_global_search(
        &mut self,
        completed: CompletedGlobalSearch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.record_search_history(&completed.query.text, window, cx);
        match completed.scope {
            SearchScope::AllOpenFiles => self.global_search.query = completed.query,
            SearchScope::Directory => {
                self.global_search.directory_query = completed.query;
                let paths = completed
                    .results
                    .values()
                    .map(|result| result.path.clone())
                    .collect::<BTreeSet<_>>();
                self.global_search.retain_directory_document_paths(&paths);
            }
            SearchScope::CurrentFile => {
                debug_assert!(
                    false,
                    "current-file results have a document-owned installer"
                );
                return;
            }
        }
        self.global_search.results = completed.results;
        self.global_search.matcher = completed.matcher;
        self.global_search.result_scope = Some(completed.scope);

        let word_wrap = self.global_viewport.is_wrapped();
        let row_height = self.log_row_height();
        self.refresh_global_result_rows(cx);
        self.position_global_row_viewport_anchor(completed.viewport_anchor, row_height, cx);
        if word_wrap {
            self.prime_global_wrapped_frame(row_height, false, window, cx);
            self.position_global_row_viewport_anchor(completed.viewport_anchor, row_height, cx);
        }
        self.activity = Activity::Ready;

        let pending_restore = match completed.scope {
            SearchScope::AllOpenFiles => self.global_search.pending_all_open_restore.clone(),
            SearchScope::Directory => self.global_search.pending_directory_restore.clone(),
            SearchScope::CurrentFile => None,
        };
        if let Some(persisted) = pending_restore {
            self.restore_persisted_global_presentation(completed.scope, persisted, window, cx);
        }
        self.schedule_workspace_search_state_save(window, cx);
    }

    fn start_global_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.documents.is_empty() {
            return;
        }
        let text = self.query.read(cx).value().to_string();
        if self.global_search.selected_documents.is_empty() {
            window.push_notification(
                crate::tr!(
                    "尚未选择参与全局搜索的文件",
                    "No files are selected for global search"
                ),
                cx,
            );
            return;
        }
        if self.documents.iter().any(|tab| {
            self.global_search.selected_documents.contains(&tab.id)
                && tab.load_state != DocumentLoadState::Ready
        }) {
            window.push_notification(
                crate::tr!(
                    "所选文件的完整索引建立后即可全局搜索",
                    "Global search will be available after the selected files are fully indexed"
                ),
                cx,
            );
            return;
        }

        let query = SearchQuery {
            text,
            case_sensitive: self.case_sensitive,
            regex: self.regex,
            max_results: self.app_settings.search_result_limit(),
        };
        if self
            .global_search
            .pending_all_open_restore
            .as_ref()
            .is_some_and(|persisted| persisted.query.text != query.text)
        {
            self.global_search.pending_all_open_restore = None;
        }
        let row_height = self.log_row_height();
        let viewport_anchor = self
            .global_search
            .results_visible
            .then(|| self.capture_global_row_viewport_anchor(row_height, cx))
            .flatten();
        self.cancel_search();
        self.global_search.revision = self.global_search.revision.saturating_add(1);
        let revision = self.global_search.revision;
        let cancellation = SearchCancellation::default();
        let targets = self
            .documents
            .iter()
            .filter(|tab| self.global_search.selected_documents.contains(&tab.id))
            .map(|tab| {
                (
                    tab.id,
                    tab.title.clone(),
                    tab.document.path().to_path_buf(),
                    tab.document.clone(),
                    SearchProgress::new(tab.document.line_count()),
                )
            })
            .collect::<Vec<_>>();
        let target = SearchTarget::AllOpenFiles;
        self.searches.begin(target, revision, cancellation.clone());
        self.global_search.results_visible = true;
        self.activity = Activity::Searching;
        cx.notify();
        let query_for_search = query.clone();
        let task = cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let matcher = SearchMatcher::new(&query_for_search)?;
                    let max_results = query_for_search.max_results;
                    let matcher_for_search = matcher.as_ref();
                    let outcomes = targets
                        .into_par_iter()
                        .map(|target| {
                            let progress = target.4.clone();
                            let run = search_with_compiled_matcher(
                                &target.3,
                                matcher_for_search,
                                max_results,
                                &cancellation,
                                &progress,
                            );
                            (target, Ok::<_, anyhow::Error>(run))
                        })
                        .collect::<Vec<_>>();
                    Ok::<_, anyhow::Error>((outcomes, matcher))
                })
                .await;

            _ = this.update_in(cx, |this, window, cx| {
                if !this.searches.is_current(target, revision)
                    || this.global_search.revision != revision
                {
                    return;
                }

                match result {
                    Ok((outcomes, matcher)) => {
                        if outcomes
                            .iter()
                            .any(|(_, run)| matches!(run, Ok(SearchRun::Cancelled)))
                        {
                            this.activity = Activity::Ready;
                        } else {
                            let results = outcomes
                                .into_iter()
                                .filter_map(|(target, run)| {
                                    let (document_id, title, path, document, _) = target;
                                    if !this.documents.iter().any(|tab| tab.id == document_id) {
                                        return None;
                                    }
                                    let (search_result, failure) = match run {
                                        Ok(SearchRun::Completed(result)) => (result, None),
                                        Ok(SearchRun::Cancelled) => return None,
                                        Err(error) => (
                                            SearchResult::default(),
                                            Some(error.to_string().into()),
                                        ),
                                    };
                                    Some((
                                        document_id,
                                        GlobalSearchDocumentResult {
                                            title,
                                            path,
                                            document,
                                            search_result,
                                            failure,
                                        },
                                    ))
                                })
                                .collect::<GlobalSearchResults>();
                            this.install_completed_global_search(
                                CompletedGlobalSearch {
                                    scope: SearchScope::AllOpenFiles,
                                    query,
                                    results,
                                    matcher,
                                    viewport_anchor,
                                },
                                window,
                                cx,
                            );
                        }
                    }
                    Err(error) => {
                        let message: SharedString = error.to_string().into();
                        window.push_notification(message.clone(), cx);
                        this.activity = Activity::Error;
                    }
                }
                this.searches.finish(target, revision);
                cx.notify();
            });
        });
        self.searches.set_task(task);
    }

    fn start_directory_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(directory) = self.global_search.directory_options.directory.clone() else {
            window.push_notification(
                crate::tr!(
                    "请先设置目录搜索范围",
                    "Set the directory search scope first"
                ),
                cx,
            );
            self.open_directory_search_dialog(window, cx);
            return;
        };
        let text = self.query.read(cx).value().to_string();

        let query = SearchQuery {
            text,
            case_sensitive: self.case_sensitive,
            regex: self.regex,
            max_results: self.app_settings.search_result_limit(),
        };
        if self
            .global_search
            .pending_directory_restore
            .as_ref()
            .is_some_and(|persisted| persisted.query.text != query.text)
        {
            self.global_search.pending_directory_restore = None;
        }
        let options = self.global_search.directory_options.clone();
        let open_document_paths = self
            .documents
            .iter()
            .map(|tab| path_match_key(tab.document.path()))
            .collect::<BTreeSet<_>>();
        let row_height = self.log_row_height();
        let viewport_anchor = self
            .global_search
            .results_visible
            .then(|| self.capture_global_row_viewport_anchor(row_height, cx))
            .flatten();
        self.cancel_search();
        self.global_search.revision = self.global_search.revision.saturating_add(1);
        let revision = self.global_search.revision;
        let cancellation = SearchCancellation::default();
        let target = SearchTarget::Directory;
        self.searches.begin(target, revision, cancellation.clone());
        self.global_search.results_visible = true;
        self.activity = Activity::Searching;
        cx.notify();
        let query_for_search = query.clone();
        let cancellation_for_search = cancellation.clone();
        let task = cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let matcher = SearchMatcher::new(&query_for_search)?;
                    let enumeration = enumerate_directory_search_paths(&options)?;
                    let file_count = enumeration.paths.len();
                    let unreadable_directory_count = enumeration.unreadable_directory_count;
                    let max_results = query_for_search.max_results;
                    let outcomes = prepare_paths_bounded(
                        enumeration.paths,
                        |path| -> Result<Option<(Arc<LogDocument>, SearchResult)>> {
                        if cancellation_for_search.is_cancelled() {
                            return Ok(None);
                        }
                        let (document, pending_index_cache) =
                            if let Some(cache_root) = crate::app_paths::cache_dir() {
                                LogDocument::open_with_index_cache(
                                    path,
                                    cache_root.join("VCLogg2").join("index"),
                                )?
                            } else {
                                (LogDocument::open(path)?, None)
                            };
                        let document = Arc::new(document);
                        let progress = SearchProgress::new(document.line_count());
                        let run = search_with_compiled_matcher(
                            &document,
                            matcher.as_ref(),
                            max_results,
                            &cancellation_for_search,
                            &progress,
                        );
                        if let Some(cache_write) = pending_index_cache {
                            _ = cache_write.persist();
                        }
                        match run {
                            SearchRun::Completed(search_result)
                                if !search_result.line_indices.is_empty()
                                    || path_match_set_contains(&open_document_paths, path) =>
                            {
                                Ok(Some((document, search_result)))
                            }
                            SearchRun::Completed(_) | SearchRun::Cancelled => Ok(None),
                        }
                        },
                    );
                    if cancellation_for_search.is_cancelled() {
                        return Ok::<_, anyhow::Error>((
                            true,
                            Vec::new(),
                            matcher,
                            file_count,
                            0,
                            unreadable_directory_count,
                        ));
                    }
                    let mut open_error_count = 0;
                    let mut results = Vec::new();
                    for (path, outcome) in outcomes {
                        match outcome {
                            Ok(Some((document, search_result))) => {
                                let title: SharedString = path
                                    .file_name()
                                    .map(|name| name.to_string_lossy().into_owned())
                                    .unwrap_or_else(|| path.display().to_string())
                                    .into();
                                results.push(DirectorySearchResult {
                                    title,
                                    path,
                                    document,
                                    search_result,
                                });
                            }
                            Ok(None) => {}
                            Err(_) => open_error_count += 1,
                        }
                    }
                    Ok::<_, anyhow::Error>((
                        false,
                        results,
                        matcher,
                        file_count,
                        open_error_count,
                        unreadable_directory_count,
                    ))
                })
                .await;

            _ = this.update_in(cx, |this, window, cx| {
                if !this.searches.is_current(target, revision)
                    || this.global_search.revision != revision
                {
                    return;
                }

                match result {
                    Ok((true, _, _, _, _, _)) => this.activity = Activity::Ready,
                    Ok((
                        false,
                        results,
                        matcher,
                        file_count,
                        open_error_count,
                        unreadable_directory_count,
                    )) => {
                        let results = results
                            .into_iter()
                            .map(|result| {
                                let document_id = this
                                    .global_search
                                    .directory_document_id(&result.path);
                                (
                                    document_id,
                                    GlobalSearchDocumentResult {
                                        title: result.title,
                                        path: result.path,
                                        document: result.document,
                                        search_result: result.search_result,
                                        failure: None,
                                    },
                                )
                            })
                            .collect();
                        this.install_completed_global_search(
                            CompletedGlobalSearch {
                                scope: SearchScope::Directory,
                                query,
                                results,
                                matcher,
                                viewport_anchor,
                            },
                            window,
                            cx,
                        );
                        if file_count == 0 {
                            window.push_notification(
                                crate::tr_args!("目录中没有符合文件类型的文件：{}", "No matching file types were found in the directory: {}", directory.display()),
                                cx,
                            );
                        } else if open_error_count > 0 || unreadable_directory_count > 0 {
                            window.push_notification(
                                crate::tr_args!(
                                    "目录搜索已完成；{open_error_count} 个文件和 {unreadable_directory_count} 个子目录无法读取",
                                    "Directory search completed; {open_error_count} files and {unreadable_directory_count} subdirectories couldn’t be read",
                                ),
                                cx,
                            );
                        }
                    }
                    Err(error) => {
                        window.push_notification(crate::tr_args!("目录搜索失败：{error}", "Directory search failed: {error}"), cx);
                        this.activity = Activity::Error;
                    }
                }
                this.searches.finish(target, revision);
                cx.notify();
            });
        });
        self.searches.set_task(task);
    }

    fn clear_search_action(
        &mut self,
        _: &ClearSearch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.clear_search(window, cx);
    }

    fn cancel_search_action(
        &mut self,
        _: &CancelSearch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.cancel_search() {
            window.push_notification(crate::tr!("已取消当前搜索", "Current search canceled"), cx);
            cx.notify();
        }
    }

    fn clear_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.global_search.scope {
            SearchScope::CurrentFile => self.clear_current_search(window, cx),
            SearchScope::AllOpenFiles => self.clear_global_search(window, cx),
            SearchScope::Directory => self.clear_directory_search(window, cx),
        }
    }

    fn clear_current_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(active_ix) = self.active_ix else {
            return;
        };
        let document_id = self.documents[active_ix].id;
        self.cancel_search_for(document_id);
        let highlight_matches = self.app_settings.highlight_matches;
        let case_sensitive = self.app_settings.default_case_sensitive;
        let regex = self.app_settings.default_use_regex;
        let max_results = self.app_settings.search_result_limit();
        let tab = &mut self.documents[active_ix];
        tab.search_revision += 1;
        tab.search_query = SearchQuery {
            text: String::new(),
            case_sensitive,
            regex,
            max_results,
        };
        tab.search_result = SearchResult::default();
        tab.search_matcher = None;
        tab.results_visible = false;
        tab.refresh_result_rows(cx);
        tab.refresh_search_matcher(highlight_matches, cx);
        self.reset_search_history_navigation();
        self.close_search_autocomplete();
        self.query
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.activity = Activity::Ready;
        self.schedule_checkpoint(document_id, window, cx);
        cx.notify();
    }

    fn clear_global_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_search();
        self.global_search.revision = self.global_search.revision.saturating_add(1);
        self.global_search.query = SearchQuery {
            text: String::new(),
            case_sensitive: self.app_settings.default_case_sensitive,
            regex: self.app_settings.default_use_regex,
            max_results: self.app_settings.search_result_limit(),
        };
        self.global_search.results_visible = false;
        self.global_search.results.clear();
        self.global_search.matcher = None;
        self.global_search.result_scope = None;
        self.global_search.pending_all_open_restore = None;
        self.global_search.all_open_context = RetainedGlobalSearchContext::default();
        self.refresh_global_result_rows(cx);
        self.reset_search_history_navigation();
        self.close_search_autocomplete();
        self.query
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.activity = Activity::Ready;
        self.schedule_workspace_search_state_save(window, cx);
        cx.notify();
    }

    fn clear_directory_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_search();
        self.global_search.revision = self.global_search.revision.saturating_add(1);
        self.global_search.directory_query = SearchQuery {
            text: String::new(),
            case_sensitive: self.app_settings.default_case_sensitive,
            regex: self.app_settings.default_use_regex,
            max_results: self.app_settings.search_result_limit(),
        };
        self.global_search.results_visible = false;
        self.global_search.results.clear();
        self.global_search.matcher = None;
        self.global_search.result_scope = None;
        self.global_search.pending_directory_restore = None;
        self.global_search.directory_context = RetainedGlobalSearchContext::default();
        self.global_search.clear_directory_document_ids();
        self.refresh_global_result_rows(cx);
        self.reset_search_history_navigation();
        self.close_search_autocomplete();
        self.query
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.activity = Activity::Ready;
        self.schedule_workspace_search_state_save(window, cx);
        cx.notify();
    }

    fn cancel_search(&mut self) -> bool {
        let was_active = self.searches.cancel();
        if was_active && matches!(self.activity, Activity::Searching) {
            self.activity = Activity::Ready;
        }
        was_active
    }

    fn cancel_search_for(&mut self, document_id: u64) {
        if self.searches.cancel_for_document(document_id)
            && matches!(self.activity, Activity::Searching)
        {
            self.activity = Activity::Ready;
        }
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

    fn first_wrapped_frame_range(visible: Range<usize>, count: usize) -> Range<usize> {
        visible.start.saturating_sub(2).min(count)..visible.end.saturating_add(2).min(count)
    }

    /// 正文、当前结果和全局结果共用的一行日志高度。
    ///
    /// 固定行高表格把行高交给布局引擎对齐到设备像素，换行列表则要自己按同一规则对齐，
    /// 否则同一条不换行的日志在两种模式下画出来不一样高。
    fn log_row_height(&self) -> Pixels {
        snap_to_device_pixels(
            log_line_height(
                self.app_settings.log_font_size,
                self.app_settings.log_line_spacing,
            ),
            self.scale_factor,
        )
    }

    fn measure_wrapped_line_height(
        line: SharedString,
        wrap_width: Pixels,
        font_size: u16,
        font_family: &SharedString,
        base_height: Pixels,
        window: &Window,
    ) -> Pixels {
        if line.is_empty() || wrap_width <= px(0.) {
            return base_height;
        }
        let font_size = px(font_size as f32);
        let text_style = TextStyle {
            font_family: font_family.clone(),
            font_size: font_size.into(),
            ..Default::default()
        };
        let runs = [text_style.to_run(line.len())];
        window
            .text_system()
            .shape_text(line, font_size, &runs, Some(wrap_width), None)
            .map(|lines| {
                lines
                    .iter()
                    .fold(px(0.), |height, line| {
                        height + line.size(base_height).height
                    })
                    .max(base_height)
            })
            .unwrap_or(base_height)
    }

    fn wrapped_layout_key(
        content_revision: u64,
        width: Pixels,
        font_size: u16,
        font_family: SharedString,
        base_height: Pixels,
        rem_size: Pixels,
        horizontal_padding: Pixels,
    ) -> WrappedLayoutKey {
        WrappedLayoutKey {
            content_revision,
            width,
            rem_size,
            font_family,
            font_size,
            base_height,
            horizontal_padding,
        }
    }

    fn prime_local_wrapped_frame(
        &mut self,
        tab_ix: usize,
        region: WrappedRegion,
        base_height: Pixels,
        reset_for_mode_switch: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let document_id = self.documents[tab_ix].id;
        let table = if region == WrappedRegion::Results {
            self.documents[tab_ix].result_table.clone()
        } else {
            self.documents[tab_ix].log_table.clone()
        };
        let bounds = self
            .row_drag_bounds
            .get(&(document_id, region))
            .or_else(|| {
                (region == WrappedRegion::Results)
                    .then(|| self.row_drag_bounds.get(&(document_id, WrappedRegion::Log)))
                    .flatten()
            })
            .copied();
        let wrapped_range = (!reset_for_mode_switch).then(|| {
            let wrapped = if region == WrappedRegion::Results {
                &self.documents[tab_ix].result_viewport
            } else {
                &self.documents[tab_ix].log_viewport
            };
            let viewport_height = wrapped.wrapped_viewport_height();
            wrapped_viewport_measurement_range(
                wrapped.wrapped_first_visible_row(),
                if viewport_height > px(0.) {
                    viewport_height
                } else {
                    bounds.map_or(base_height, |bounds| bounds.size.height)
                },
                base_height,
                table.read(cx).delegate().row_count(),
            )
        });
        let (count, content_revision, outer_width, font_size, font_family, rows) = {
            let table = table.read(cx);
            let delegate = table.delegate();
            let count = delegate.row_count();
            let range = wrapped_range.unwrap_or_else(|| {
                Self::first_wrapped_frame_range(table.visible_range().rows().clone(), count)
            });
            delegate.prepare_visible_rows(range.clone());
            let font_size = delegate.log_font_size();
            let line_number_width = if delegate.show_line_numbers() {
                px(delegate.line_number_width() as f32)
            } else {
                px(0.)
            };
            let outer_width = bounds.map_or(px(0.), |bounds| {
                (bounds.size.width - line_marker_column_width() - line_number_width).max(px(0.))
            });
            let rows = range
                .filter_map(|row_ix| {
                    delegate
                        .wrapped_row(row_ix)
                        .map(|row| (row_ix, row.text.display().clone()))
                })
                .collect::<Vec<_>>();
            (
                count,
                delegate.content_revision(),
                outer_width,
                font_size,
                delegate.resolved_font_family(cx),
                rows,
            )
        };
        let horizontal_padding = log_cell_horizontal_padding(cx);
        let text_width = (outer_width - horizontal_padding * 2.).max(px(0.));
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
        let wrapped = if region == WrappedRegion::Results {
            &mut self.documents[tab_ix].result_viewport
        } else {
            &mut self.documents[tab_ix].log_viewport
        };
        if reset_for_mode_switch {
            wrapped.reset_wrapped_scroll_for_mode_switch();
        }
        wrapped.invalidate_wrapped_layout_preserving_position(
            Self::wrapped_layout_key(
                content_revision,
                outer_width,
                font_size,
                font_family.clone(),
                base_height,
                window.rem_size(),
                horizontal_padding,
            ),
            table.read(cx).active_log_row(),
        );
        wrapped.prime_wrapped_measured_heights(count, base_height, heights);
    }

    fn prime_global_wrapped_frame(
        &mut self,
        base_height: Pixels,
        reset_for_mode_switch: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let bounds = self
            .row_drag_bounds
            .get(&(0, WrappedRegion::GlobalResults))
            .or_else(|| {
                self.active_document()
                    .and_then(|tab| self.row_drag_bounds.get(&(tab.id, WrappedRegion::Log)))
            })
            .copied();
        let wrapped_range = (!reset_for_mode_switch).then(|| {
            let viewport_height = self.global_viewport.wrapped_viewport_height();
            wrapped_viewport_measurement_range(
                self.global_viewport.wrapped_first_visible_row(),
                if viewport_height > px(0.) {
                    viewport_height
                } else {
                    bounds.map_or(base_height, |bounds| bounds.size.height)
                },
                base_height,
                self.global_table.read(cx).delegate().rows_len(),
            )
        });
        let (count, content_revision, outer_width, font_size, font_family, rows) = {
            let table = self.global_table.read(cx);
            let delegate = table.delegate();
            let count = delegate.rows_len();
            let range = wrapped_range.unwrap_or_else(|| {
                Self::first_wrapped_frame_range(table.visible_range().rows().clone(), count)
            });
            delegate.prepare_visible_rows(range.clone());
            let font_size = delegate.log_font_size();
            let outer_width = bounds.map_or(px(0.), |bounds| {
                (bounds.size.width
                    - line_marker_column_width()
                    - px(delegate.line_number_width() as f32))
                .max(px(0.))
            });
            let rows = range
                .filter_map(|row_ix| match delegate.wrapped_row(row_ix)? {
                    WrappedGlobalRow::Match { text, .. } => Some((row_ix, text.display().clone())),
                    WrappedGlobalRow::Group { .. } => None,
                })
                .collect::<Vec<_>>();
            (
                count,
                delegate.content_revision(),
                outer_width,
                font_size,
                delegate.resolved_font_family(cx),
                rows,
            )
        };
        let horizontal_padding = log_cell_horizontal_padding(cx);
        let text_width = (outer_width - horizontal_padding * 2.).max(px(0.));
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
        if reset_for_mode_switch {
            self.global_viewport.reset_wrapped_scroll_for_mode_switch();
        }
        self.global_viewport
            .invalidate_wrapped_layout_preserving_position(
                Self::wrapped_layout_key(
                    content_revision,
                    outer_width,
                    font_size,
                    font_family.clone(),
                    base_height,
                    window.rem_size(),
                    horizontal_padding,
                ),
                self.global_table.read(cx).active_log_row(),
            );
        self.global_viewport
            .prime_wrapped_measured_heights(count, base_height, heights);
    }

    fn prime_global_wrapped_group_toggle(
        &mut self,
        anchor: Option<RowViewportAnchor<LogRowKey>>,
        base_height: Pixels,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let count = self.global_table.read(cx).delegate().rows_len();
        if count == 0 {
            return;
        }

        self.global_viewport.wrapped_sizes(count, base_height);
        self.position_global_row_viewport_anchor(anchor, base_height, cx);

        let first_visible = self.global_viewport.wrapped_first_visible_row();
        let visible_range = wrapped_viewport_measurement_range(
            first_visible,
            self.global_viewport.wrapped_viewport_height(),
            base_height,
            count,
        );
        let (outer_width, font_size, font_family, rows) = {
            let table = self.global_table.read(cx);
            let delegate = table.delegate();
            delegate.prepare_visible_rows(visible_range.clone());
            let font_size = delegate.log_font_size();
            let outer_width = if let Some(width) = self.global_viewport.wrapped_layout_width() {
                width
            } else {
                self.row_drag_bounds
                    .get(&(0, WrappedRegion::GlobalResults))
                    .map_or(px(0.), |bounds| {
                        (bounds.size.width
                            - line_marker_column_width()
                            - px(delegate.line_number_width() as f32))
                        .max(px(0.))
                    })
            };
            let rows = visible_range
                .filter_map(|row_ix| match delegate.wrapped_row(row_ix)? {
                    WrappedGlobalRow::Match { text, .. } => Some((row_ix, text.display().clone())),
                    WrappedGlobalRow::Group { .. } => None,
                })
                .collect::<Vec<_>>();
            (
                outer_width,
                font_size,
                delegate.resolved_font_family(cx),
                rows,
            )
        };
        let text_width = (outer_width - log_cell_horizontal_padding(cx) * 2.).max(px(0.));
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
    }

    fn prime_wrapped_first_frame(
        &mut self,
        tab_ix: usize,
        base_height: Pixels,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        self.prime_local_wrapped_frame(tab_ix, WrappedRegion::Log, base_height, true, window, cx);
        self.prime_local_wrapped_frame(
            tab_ix,
            WrappedRegion::Results,
            base_height,
            true,
            window,
            cx,
        );
    }

    fn toggle_word_wrap(
        &mut self,
        _: &ToggleWordWrap,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let toggles_from_global_results = self.active_log_region == LogRegion::GlobalResults
            && self.global_search.results_visible
            && self.global_search.scope.owns_global_word_wrap();
        let enabled = if toggles_from_global_results {
            !self.global_viewport.is_wrapped()
        } else {
            let Some(active_ix) = self.active_ix else {
                return;
            };
            !self.documents[active_ix].log_viewport.is_wrapped()
        };
        let row_height = self.log_row_height();

        if self.global_search.scope.owns_global_word_wrap()
            && self.global_viewport.is_wrapped() != enabled
        {
            let anchor = self.capture_global_row_viewport_anchor(row_height, cx);
            if enabled {
                self.prime_global_wrapped_frame(row_height, true, window, cx);
            }
            self.global_viewport.set_word_wrap(enabled);
            self.position_global_row_viewport_anchor(anchor, row_height, cx);
            self.global_surface.update(cx, |_, cx| cx.notify());
            self.schedule_workspace_search_state_save(window, cx);
        }

        if let Some(active_ix) = self.active_ix {
            let was_enabled = self.documents[active_ix].log_viewport.is_wrapped();
            if was_enabled != enabled {
                let log_anchor = Self::capture_local_row_viewport_anchor(
                    &self.documents[active_ix],
                    WrappedRegion::Log,
                    row_height,
                    cx,
                );
                let result_anchor = Self::capture_local_row_viewport_anchor(
                    &self.documents[active_ix],
                    WrappedRegion::Results,
                    row_height,
                    cx,
                );
                if enabled {
                    self.prime_wrapped_first_frame(active_ix, row_height, window, cx);
                }
                let document_id = {
                    let tab = &mut self.documents[active_ix];
                    tab.log_viewport.set_word_wrap(enabled);
                    tab.result_viewport.set_word_wrap(enabled);
                    tab.id
                };
                let tab = &self.documents[active_ix];
                Self::position_local_row_viewport_anchor(
                    tab,
                    WrappedRegion::Log,
                    log_anchor,
                    row_height,
                    cx,
                );
                Self::position_local_row_viewport_anchor(
                    tab,
                    WrappedRegion::Results,
                    result_anchor,
                    row_height,
                    cx,
                );
                for surface in [tab.log_surface.clone(), tab.result_surface.clone()] {
                    surface.update(cx, |_, cx| cx.notify());
                }
                self.schedule_checkpoint(document_id, window, cx);
            }
        }

        window.push_notification(
            if enabled {
                crate::tr!("已开启自动换行", "Word wrap enabled")
            } else {
                crate::tr!("已关闭自动换行", "Word wrap disabled")
            },
            cx,
        );
        cx.notify();
    }

    fn active_navigation_region(&self) -> Option<(u64, WrappedRegion)> {
        let tab = self.active_document()?;
        if self.active_log_region == LogRegion::GlobalResults && self.global_search.results_visible
        {
            return Some((tab.id, WrappedRegion::GlobalResults));
        }
        Some((
            tab.id,
            if tab.selection_table == SelectionTable::Results && tab.results_visible {
                WrappedRegion::Results
            } else {
                WrappedRegion::Log
            },
        ))
    }

    fn navigate_log_rows(
        &mut self,
        direction: i32,
        page: bool,
        edge: Option<bool>,
        cx: &mut Context<Self>,
    ) {
        let Some((document_id, region)) = self.active_navigation_region() else {
            return;
        };
        let base_height = self.log_row_height();
        let (count, selected, page_step) = match region {
            WrappedRegion::Log | WrappedRegion::Results => {
                let Some(tab_ix) = self.documents.iter().position(|tab| tab.id == document_id)
                else {
                    return;
                };
                let (table, viewport) = if region == WrappedRegion::Results {
                    (
                        self.documents[tab_ix].result_table.clone(),
                        &self.documents[tab_ix].result_viewport,
                    )
                } else {
                    (
                        self.documents[tab_ix].log_table.clone(),
                        &self.documents[tab_ix].log_viewport,
                    )
                };
                (
                    table.read(cx).delegate().row_count(),
                    table.read(cx).active_log_row(),
                    viewport.page_size(table.read(cx).visible_range().rows().len(), base_height),
                )
            }
            WrappedRegion::GlobalResults => (
                self.global_table.read(cx).delegate().rows_len(),
                self.global_table.read(cx).active_log_row(),
                self.global_viewport.page_size(
                    self.global_table.read(cx).visible_range().rows().len(),
                    base_height,
                ),
            ),
        };
        if count == 0 {
            return;
        }
        let step = if page { page_step } else { 1 };
        let current = selected.unwrap_or_else(|| if direction < 0 { count - 1 } else { 0 });
        let target = match edge {
            Some(false) => 0,
            Some(true) => count - 1,
            None if direction < 0 => current.saturating_sub(step),
            None => current.saturating_add(step).min(count - 1),
        };
        match region {
            WrappedRegion::Log | WrappedRegion::Results => {
                let Some(tab_ix) = self.documents.iter().position(|tab| tab.id == document_id)
                else {
                    return;
                };
                let table = if region == WrappedRegion::Results {
                    self.documents[tab_ix].result_table.clone()
                } else {
                    self.documents[tab_ix].log_table.clone()
                };
                table.update(cx, |table, cx| table.set_active_log_row(target, cx));
            }
            WrappedRegion::GlobalResults => self
                .global_table
                .update(cx, |table, cx| table.set_active_log_row(target, cx)),
        }
        let strategy = if direction < 0 || edge == Some(false) {
            ScrollStrategy::Top
        } else {
            ScrollStrategy::Bottom
        };
        match region {
            WrappedRegion::Log | WrappedRegion::Results => {
                let Some(tab) = self.documents.iter().find(|tab| tab.id == document_id) else {
                    return;
                };
                if region == WrappedRegion::Results {
                    tab.result_viewport.reveal_row(target, strategy);
                } else {
                    tab.log_viewport.reveal_row(target, strategy);
                }
            }
            WrappedRegion::GlobalResults => {
                self.global_viewport.reveal_row(target, strategy);
            }
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn select_wrapped_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        self.navigate_log_rows(-1, false, None, cx);
    }

    fn select_wrapped_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        self.navigate_log_rows(1, false, None, cx);
    }

    fn select_wrapped_page_up(&mut self, _: &SelectPageUp, _: &mut Window, cx: &mut Context<Self>) {
        self.navigate_log_rows(-1, true, None, cx);
    }

    fn select_wrapped_page_down(
        &mut self,
        _: &SelectPageDown,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.navigate_log_rows(1, true, None, cx);
    }

    fn select_wrapped_first(&mut self, _: &SelectFirst, _: &mut Window, cx: &mut Context<Self>) {
        self.navigate_log_rows(-1, false, Some(false), cx);
    }

    fn select_wrapped_last(&mut self, _: &SelectLast, _: &mut Window, cx: &mut Context<Self>) {
        self.navigate_log_rows(1, false, Some(true), cx);
    }

    /// 菜单按钮为 28px 高、11px 横向内边距、8px 圆角和 12px 常规字重。
    /// `Button` 的 size 预设只有 24/32px 两档，够不到 28px；独立文字层同时避免
    /// 英文字母在组件标签的紧凑行高中被裁切。
    fn title_bar_menu_button(id: &'static str, label: &'static str) -> TitleBarMenuButton {
        TitleBarMenuButton {
            button: Button::new(id)
                .small()
                .ghost()
                .h(px(28.))
                .px(px(11.))
                .rounded(px(8.))
                .font_weight(FontWeight::NORMAL)
                .child(
                    div()
                        .text_size(px(12.))
                        .line_height(relative(1.25))
                        .child(label),
                ),
        }
        .accessibility_id(id)
        .aria_label(label)
    }

    /// 下拉菜单使用已挂载的工作区根焦点查询动作快捷键，确保首帧就按完整内容计算宽度。
    /// 弹层自身的焦点要到下一帧才进入分发树，不能作为稳定的菜单布局依据。
    fn popup_menu_with_workspace_action_context(
        menu: PopupMenu,
        workspace: &Entity<Self>,
        cx: &App,
    ) -> PopupMenu {
        menu.action_context(workspace.read(cx).focus_handle.clone())
    }

    fn render_title_bar(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _performance_scope = crate::ui_performance::scope("Workspace::render_title_bar");
        let workspace = cx.entity();
        let has_document = self.active_document().is_some();
        let has_selected_log_rows = if self.active_log_region == LogRegion::GlobalResults
            && self.global_search.results_visible
        {
            self.global_table.read(cx).delegate().selected_rows_count() > 0
        } else {
            self.selected_source_row.is_some()
        };
        let active_file_is_pinned = self.active_file_is_pinned();
        let auto_follow = self.active_document().is_some_and(|tab| tab.auto_follow);
        let show_line_numbers = self
            .active_document()
            .is_none_or(|tab| tab.show_line_numbers);
        let show_row_separators = self
            .active_document()
            .is_some_and(|tab| tab.show_row_separators);
        let word_wrap = if self.active_log_region == LogRegion::GlobalResults
            && self.global_search.results_visible
            && self.global_search.scope.owns_global_word_wrap()
        {
            self.global_viewport.is_wrapped()
        } else {
            self.active_document()
                .is_some_and(|tab| tab.log_viewport.is_wrapped())
        };
        let show_full_path = self.app_settings.show_full_path;
        let highlight_log_levels = self.app_settings.highlight_log_levels;
        let highlight_matches = self.app_settings.highlight_matches;
        let case_sensitive = self.case_sensitive;
        let regex = self.regex;
        let tab_count = self.tabs.len();
        let update_button_label = self.updates.button_label();
        let update_busy = self.updates.is_busy();
        let active_encoding = self.active_document().map(|tab| {
            (
                tab.id,
                SharedString::from(if tab.load_state == DocumentLoadState::Opening {
                    crate::tr!("检测中", "Detecting").to_string()
                } else {
                    tab.document.metadata().encoding_name.clone()
                }),
            )
        });

        let file_workspace = workspace.clone();
        let file_menu = Self::title_bar_menu_button("title-menu-file", crate::tr!("文件", "File"))
            .dropdown_menu(move |menu, window, cx| {
                let menu =
                    Self::popup_menu_with_workspace_action_context(menu, &file_workspace, cx);
                let open = window.listener_for(&file_workspace, |this, _, window, cx| {
                    this.open_files(&OpenFiles, window, cx);
                });
                let new_window = window.listener_for(&file_workspace, |this, _, window, cx| {
                    this.new_window(&NewWindow, window, cx);
                });
                let reload = window.listener_for(&file_workspace, |this, _, window, cx| {
                    this.reload_active(&ReloadActive, window, cx);
                });
                let close = window.listener_for(&file_workspace, |this, _, window, cx| {
                    this.close_active_tab(&CloseActiveTab, window, cx);
                });
                let history = window.listener_for(&file_workspace, |this, _, window, cx| {
                    this.open_history_dialog(window, cx);
                });
                let reveal = window.listener_for(&file_workspace, |this, _, window, cx| {
                    let Some(document_id) = this.active_document().map(|tab| tab.id) else {
                        return;
                    };
                    this.reveal_tab_file(document_id, window, cx);
                });
                let close_others = window.listener_for(&file_workspace, |this, _, window, cx| {
                    this.close_tab_group(this.active_tab_id, TabCloseGroup::Others, window, cx);
                });
                let close_all = window.listener_for(&file_workspace, |this, _, window, cx| {
                    this.close_tab_group(this.active_tab_id, TabCloseGroup::All, window, cx);
                });
                menu.item(
                    PopupMenuItem::new(crate::tr!("打开…", "Open…"))
                        .icon(IconName::FolderOpen)
                        .action(Box::new(OpenFiles))
                        .on_click(open),
                )
                .item(
                    PopupMenuItem::new(crate::tr!("新窗口", "New window"))
                        .action(Box::new(NewWindow))
                        .on_click(new_window),
                )
                .item(
                    PopupMenuItem::new(crate::tr!("重新加载", "Reload"))
                        .action(Box::new(ReloadActive))
                        .disabled(!has_document)
                        .on_click(reload),
                )
                .item(
                    PopupMenuItem::new(crate::tr!(
                        "在文件资源管理器中显示",
                        "Show in File Explorer"
                    ))
                    .disabled(!has_document)
                    .on_click(reveal),
                )
                .item(
                    PopupMenuItem::new(crate::tr!("历史…", "History…"))
                        .disabled(file_workspace.read_with(cx, |this, _| {
                            this.persistence.store.is_none() || this.history_dialog_loading
                        }))
                        .on_click(history),
                )
                .separator()
                .item(
                    PopupMenuItem::new(crate::tr!("关闭标签", "Close tab"))
                        .action(Box::new(CloseActiveTab))
                        .on_click(close),
                )
                .item(
                    PopupMenuItem::new(crate::tr!("关闭其他标签页", "Close other tabs"))
                        .disabled(tab_count < 2)
                        .on_click(close_others),
                )
                .item(
                    PopupMenuItem::new(crate::tr!("关闭全部标签页", "Close all tabs"))
                        .on_click(close_all),
                )
                .separator()
                .item(
                    PopupMenuItem::new(crate::tr!("退出 VCLogg2", "Quit VCLogg2"))
                        .on_click(|_, _, cx| cx.quit()),
                )
            });

        let edit_workspace = workspace.clone();
        let edit_menu = Self::title_bar_menu_button("title-menu-edit", crate::tr!("编辑", "Edit"))
            .dropdown_menu(move |menu, window, cx| {
                let menu =
                    Self::popup_menu_with_workspace_action_context(menu, &edit_workspace, cx);
                let copy = window.listener_for(&edit_workspace, |this, _, window, cx| {
                    this.copy_current_line(&CopyCurrentLine, window, cx);
                });
                let copy_with_number =
                    window.listener_for(&edit_workspace, |this, _, window, cx| {
                        this.copy_current_line_with_number(&CopyCurrentLineWithNumber, window, cx);
                    });
                let select_all = window.listener_for(&edit_workspace, |this, _, window, cx| {
                    this.select_all_rows(&SelectAllRows, window, cx);
                });
                let copy_path = window.listener_for(&edit_workspace, |this, _, window, cx| {
                    this.copy_file_path(&CopyFilePath, window, cx);
                });
                let go_to_line = window.listener_for(&edit_workspace, |this, _, window, cx| {
                    this.open_go_to_line(&GoToLine, window, cx);
                });
                let find = window.listener_for(&edit_workspace, |this, _, window, cx| {
                    this.focus_search(&FocusSearch, window, cx);
                });
                let clear_search = window.listener_for(&edit_workspace, |this, _, window, cx| {
                    this.clear_search(window, cx);
                });
                menu.item(
                    PopupMenuItem::new(crate::tr!("复制当前行", "Copy current line"))
                        .action(Box::new(CopyCurrentLine))
                        .disabled(!has_document || !has_selected_log_rows)
                        .on_click(copy),
                )
                .item(
                    PopupMenuItem::new(crate::tr!(
                        "复制当前行（含行号）",
                        "Copy current line with number"
                    ))
                    .action(Box::new(CopyCurrentLineWithNumber))
                    .disabled(!has_document || !has_selected_log_rows)
                    .on_click(copy_with_number),
                )
                .item(
                    PopupMenuItem::new(crate::tr!("复制文件路径", "Copy file path"))
                        .action(Box::new(CopyFilePath))
                        .disabled(!has_document)
                        .on_click(copy_path),
                )
                .item(
                    PopupMenuItem::new(crate::tr!("全选行", "Select all lines"))
                        .action(Box::new(SelectAllRows))
                        .disabled(!has_document)
                        .on_click(select_all),
                )
                .separator()
                .item(
                    PopupMenuItem::new(crate::tr!("查找", "Find"))
                        .action(Box::new(FocusSearch))
                        .disabled(!has_document)
                        .on_click(find),
                )
                .item(
                    PopupMenuItem::new(crate::tr!("转到行…", "Go to line…"))
                        .action(Box::new(GoToLine))
                        .disabled(!has_document)
                        .on_click(go_to_line),
                )
                .item(
                    PopupMenuItem::new(crate::tr!("清除搜索结果", "Clear search results"))
                        .action(Box::new(ClearSearch))
                        .disabled(!has_document)
                        .on_click(clear_search),
                )
            });

        let view_workspace = workspace.clone();
        let view_menu = Self::title_bar_menu_button("title-menu-view", crate::tr!("视图", "View"))
            .dropdown_menu(move |menu, window, cx| {
                let menu =
                    Self::popup_menu_with_workspace_action_context(menu, &view_workspace, cx);
                let fullscreen_label = if window.is_fullscreen() {
                    crate::tr!("退出全屏", "Exit full screen")
                } else {
                    crate::tr!("进入全屏", "Enter full screen")
                };
                let toggle_auto_follow = {
                    let workspace = view_workspace.clone();
                    window.listener_for(&workspace, |this, _, window, cx| {
                        this.toggle_auto_follow(window, cx);
                    })
                };
                let toggle_line_numbers = {
                    let workspace = view_workspace.clone();
                    window.listener_for(&workspace, |this, _, window, cx| {
                        this.toggle_line_numbers(window, cx);
                    })
                };
                let toggle_row_separators = window
                    .listener_for(&view_workspace, |this, _, window, cx| {
                        this.toggle_row_separators(window, cx)
                    });
                let toggle_word_wrap =
                    window.listener_for(&view_workspace, |this, _, window, cx| {
                        this.toggle_word_wrap(&ToggleWordWrap, window, cx);
                    });
                let jump_to_start = window.listener_for(&view_workspace, |this, _, window, cx| {
                    this.jump_to_start(&JumpToStart, window, cx);
                });
                let jump_to_end = window.listener_for(&view_workspace, |this, _, window, cx| {
                    this.jump_to_end(&JumpToEnd, window, cx);
                });
                let toggle_fullscreen =
                    window.listener_for(&view_workspace, |this, _, window, cx| {
                        this.toggle_fullscreen(&ToggleFullscreen, window, cx);
                    });
                let toggle_full_path =
                    window.listener_for(&view_workspace, |this, _, window, cx| {
                        this.update_app_setting(
                            |settings| settings.show_full_path = !settings.show_full_path,
                            window,
                            cx,
                        );
                    });
                menu.item(
                    PopupMenuItem::new(crate::tr!("自动换行", "Word wrap"))
                        .action(Box::new(ToggleWordWrap))
                        .checked(word_wrap)
                        .disabled(!has_document)
                        .on_click(toggle_word_wrap),
                )
                .item(
                    PopupMenuItem::new(crate::tr!("显示行号", "Show line numbers"))
                        .checked(show_line_numbers)
                        .disabled(!has_document)
                        .on_click(toggle_line_numbers),
                )
                .item(
                    PopupMenuItem::new(crate::tr!("日志分隔线", "Log separators"))
                        .checked(show_row_separators)
                        .disabled(!has_document)
                        .on_click(toggle_row_separators),
                )
                .item(
                    PopupMenuItem::new(crate::tr!("显示完整路径", "Show full path"))
                        .checked(show_full_path)
                        .on_click(toggle_full_path),
                )
                .item(
                    PopupMenuItem::new(crate::tr!("跟随末尾", "Follow end"))
                        .checked(auto_follow)
                        .disabled(!has_document)
                        .on_click(toggle_auto_follow),
                )
                .separator()
                .item(
                    PopupMenuItem::new(crate::tr!("文件开头", "Start of file"))
                        .action(Box::new(JumpToStart))
                        .disabled(!has_document)
                        .on_click(jump_to_start),
                )
                .item(
                    PopupMenuItem::new(crate::tr!("文件末尾", "End of file"))
                        .action(Box::new(JumpToEnd))
                        .disabled(!has_document)
                        .on_click(jump_to_end),
                )
                .separator()
                .item(
                    PopupMenuItem::new(fullscreen_label)
                        .action(Box::new(ToggleFullscreen))
                        .on_click(toggle_fullscreen),
                )
            });

        let tools_workspace = workspace.clone();
        let tools_menu =
            Self::title_bar_menu_button("title-menu-tools", crate::tr!("工具", "Tools"))
                .dropdown_menu(move |menu, window, cx| {
                    let menu =
                        Self::popup_menu_with_workspace_action_context(menu, &tools_workspace, cx);
                    let predefined_filters =
                        window.listener_for(&tools_workspace, |this, _, window, cx| {
                            this.open_predefined_filters_dialog(window, cx);
                        });
                    let clear_history =
                        window.listener_for(&tools_workspace, |this, _, window, cx| {
                            this.replace_search_history(Vec::new(), window, cx);
                            window.push_notification(
                                crate::tr!("已清除搜索历史", "Search history cleared"),
                                cx,
                            );
                        });
                    let settings = window.listener_for(&tools_workspace, |this, _, window, cx| {
                        this.open_settings_dialog(None, window, cx);
                    });
                    let (history_empty, settings_saving) = tools_workspace
                        .read_with(cx, |this, _| {
                            (this.search_history.is_empty(), this.settings_saving)
                        });
                    menu.item(
                        PopupMenuItem::new(crate::tr!("预定义过滤器…", "Predefined filters…"))
                            .icon(IconName::Settings2)
                            .on_click(predefined_filters),
                    )
                    .item(
                        PopupMenuItem::new(crate::tr!("清除搜索历史", "Clear search history"))
                            .disabled(history_empty)
                            .on_click(clear_history),
                    )
                    .separator()
                    .item(
                        PopupMenuItem::new(crate::tr!("设置", "Settings"))
                            .action(Box::new(OpenSettings))
                            .disabled(settings_saving)
                            .on_click(settings),
                    )
                });

        let highlight_workspace = workspace.clone();
        let highlight_menu =
            Self::title_bar_menu_button("title-menu-highlight", crate::tr!("高亮", "Highlight"))
                .dropdown_menu(move |menu, window, cx| {
                    let menu = Self::popup_menu_with_workspace_action_context(
                        menu,
                        &highlight_workspace,
                        cx,
                    );
                    let manage_labels =
                        window.listener_for(&highlight_workspace, |this, _, window, cx| {
                            this.open_color_labels_dialog(window, cx);
                        });
                    let toggle_marked =
                        window.listener_for(&highlight_workspace, |this, _, window, cx| {
                            this.toggle_marked_row(&ToggleMarkedRow, window, cx);
                        });
                    let cycle_color =
                        window.listener_for(&highlight_workspace, |this, _, window, cx| {
                            this.cycle_color_label(&CycleColorLabel, window, cx);
                        });
                    let toggle_levels =
                        window.listener_for(&highlight_workspace, |this, _, window, cx| {
                            this.update_app_setting(
                                |settings| {
                                    settings.highlight_log_levels = !settings.highlight_log_levels
                                },
                                window,
                                cx,
                            );
                        });
                    let toggle_match_highlight =
                        window.listener_for(&highlight_workspace, |this, _, window, cx| {
                            this.update_app_setting(
                                |settings| settings.highlight_matches = !settings.highlight_matches,
                                window,
                                cx,
                            );
                        });
                    let toggle_case =
                        window.listener_for(&highlight_workspace, |this, _, window, cx| {
                            this.toggle_case_sensitive(&ToggleCaseSensitive, window, cx);
                        });
                    let toggle_regex =
                        window.listener_for(&highlight_workspace, |this, _, window, cx| {
                            this.toggle_regex(&ToggleRegex, window, cx);
                        });
                    let clear_highlight =
                        window.listener_for(&highlight_workspace, |this, _, window, cx| {
                            this.clear_search(window, cx);
                        });
                    menu.item(
                        PopupMenuItem::new(crate::tr!("日志级别着色", "Log-level coloring"))
                            .checked(highlight_log_levels)
                            .on_click(toggle_levels),
                    )
                    .item(
                        PopupMenuItem::new(crate::tr!("高亮搜索匹配", "Highlight search matches"))
                            .checked(highlight_matches)
                            .on_click(toggle_match_highlight),
                    )
                    .separator()
                    .item(
                        PopupMenuItem::new(crate::tr!("标记行", "Mark lines"))
                            .action(Box::new(ToggleMarkedRow))
                            .disabled(!has_document || !has_selected_log_rows)
                            .on_click(toggle_marked),
                    )
                    .item(
                        PopupMenuItem::new(crate::tr!("轮换颜色", "Cycle color"))
                            .action(Box::new(CycleColorLabel))
                            .disabled(!has_document || !has_selected_log_rows)
                            .on_click(cycle_color),
                    )
                    .item(
                        PopupMenuItem::new(crate::tr!("颜色标签…", "Color labels…"))
                            .disabled(highlight_workspace.read_with(cx, |this, _| {
                                this.history_loading || this.color_labels_saving
                            }))
                            .on_click(manage_labels),
                    )
                    .separator()
                    .item(
                        PopupMenuItem::new(crate::tr!("搜索时区分大小写", "Case-sensitive search"))
                            .action(Box::new(ToggleCaseSensitive))
                            .checked(case_sensitive)
                            .disabled(!has_document)
                            .on_click(toggle_case),
                    )
                    .item(
                        PopupMenuItem::new(crate::tr!("使用正则表达式", "Use regular expressions"))
                            .action(Box::new(ToggleRegex))
                            .checked(regex)
                            .disabled(!has_document)
                            .on_click(toggle_regex),
                    )
                    .item(
                        PopupMenuItem::new(crate::tr!("清除搜索高亮", "Clear search highlighting"))
                            .action(Box::new(ClearSearch))
                            .disabled(!has_document)
                            .on_click(clear_highlight),
                    )
                });

        let encoding_menu = if let Some((document_id, encoding_name)) = active_encoding.clone() {
            let workspace = workspace.clone();
            let menu_encoding_name = encoding_name.clone();
            Self::title_bar_menu_button("title-menu-encoding", crate::tr!("编码", "Encoding"))
                .disabled(self.open_task.is_some())
                .dropdown_menu(move |menu, window, cx| {
                    Self::build_encoding_menu(
                        Self::popup_menu_with_workspace_action_context(menu, &workspace, cx),
                        document_id,
                        menu_encoding_name.clone(),
                        workspace.clone(),
                        window,
                    )
                })
                .into_any_element()
        } else {
            Self::title_bar_menu_button("title-menu-encoding", crate::tr!("编码", "Encoding"))
                .disabled(true)
                .into_any_element()
        };

        let favorite_workspace = workspace.clone();
        let favorite_menu =
            Self::title_bar_menu_button("title-menu-favorite", crate::tr!("收藏", "Favorites"))
                .dropdown_menu(move |menu, window, cx| {
                    let menu = Self::popup_menu_with_workspace_action_context(
                        menu,
                        &favorite_workspace,
                        cx,
                    );
                    let toggle = window.listener_for(&favorite_workspace, |this, _, window, cx| {
                        this.toggle_active_file_pinned(window, cx);
                    });
                    let clear = window.listener_for(&favorite_workspace, |this, _, window, cx| {
                        this.clear_pinned_files(window, cx);
                    });
                    let (pinned_files, busy, pinned_updating) =
                        favorite_workspace.read_with(cx, |this, _| {
                            (
                                this.pinned_files.clone(),
                                this.open_task.is_some(),
                                this.history_loading || this.pinned_updating,
                            )
                        });
                    let mut menu = menu
                        .item(
                            PopupMenuItem::new(if active_file_is_pinned {
                                crate::tr!("取消收藏文件", "Remove from favorites")
                            } else {
                                crate::tr!("收藏当前文件", "Favorite current file")
                            })
                            .checked(active_file_is_pinned)
                            .disabled(!has_document || pinned_updating)
                            .on_click(toggle),
                        )
                        .separator();
                    if pinned_files.is_empty() {
                        menu = menu.item(
                            PopupMenuItem::new(crate::tr!("暂无收藏文件", "No favorite files"))
                                .disabled(true),
                        );
                    } else {
                        for file in &pinned_files {
                            let path = file.path.clone();
                            let open = window.listener_for(
                                &favorite_workspace,
                                move |this, _, window, cx| {
                                    this.open_recent_file(path.clone(), window, cx);
                                },
                            );
                            menu = menu.item(
                                PopupMenuItem::new(recent_file_label(file))
                                    .disabled(busy)
                                    .on_click(open),
                            );
                        }
                    }
                    menu.separator().item(
                        PopupMenuItem::new(crate::tr!("清空收藏", "Clear favorites"))
                            .disabled(pinned_files.is_empty() || pinned_updating)
                            .on_click(clear),
                    )
                });

        let help_workspace = workspace;
        let help_menu = Self::title_bar_menu_button("title-menu-help", crate::tr!("帮助", "Help"))
            .dropdown_menu(move |menu, window, cx| {
                let menu =
                    Self::popup_menu_with_workspace_action_context(menu, &help_workspace, cx);
                let check_update = window.listener_for(&help_workspace, |this, _, window, cx| {
                    this.handle_update_button(window, cx);
                });
                let about = window.listener_for(&help_workspace, |this, _, window, cx| {
                    this.open_settings_dialog(Some(SettingsCategory::About), window, cx);
                });
                let shortcuts = window.listener_for(&help_workspace, |this, _, window, cx| {
                    this.open_settings_dialog(Some(SettingsCategory::Shortcuts), window, cx);
                });
                let settings_saving = help_workspace.read_with(cx, |this, _| this.settings_saving);
                menu.item(
                    PopupMenuItem::new(crate::tr!("键盘快捷键", "Keyboard shortcuts"))
                        .disabled(settings_saving)
                        .on_click(shortcuts),
                )
                .item(
                    PopupMenuItem::new(update_button_label.clone())
                        .disabled(update_busy)
                        .on_click(check_update),
                )
                .separator()
                .item(
                    PopupMenuItem::new(crate::tr!("关于", "About"))
                        .disabled(settings_saving)
                        .on_click(about),
                )
            });

        let colors = ui_theme::palette(cx);
        TitleBar::new()
            .when(cfg!(target_os = "macos") && window.is_fullscreen(), |bar| {
                bar.pl_0()
            })
            .h(px(36.))
            .border_b_0()
            .bg(ui_theme::header_material(&colors))
            .child(
                h_flex()
                    .relative()
                    .w_full()
                    .h_full()
                    .items_center()
                    .child(ui_theme::glass_sheen_layer(&colors))
                    .child(ui_theme::material_highlight_line(&colors))
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .right_0()
                            .bottom_0()
                            .left_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(12.))
                            .font_weight(FontWeight(620.))
                            .text_color(colors.foreground.opacity(0.86))
                            .child("VCLogg2"),
                    )
                    .child(
                        h_flex()
                            .h_full()
                            .gap_0()
                            .child(file_menu)
                            .child(edit_menu)
                            .child(view_menu)
                            .child(tools_menu)
                            .child(highlight_menu)
                            .child(encoding_menu)
                            .child(favorite_menu)
                            .child(help_menu),
                    ),
            )
    }

    fn render_file_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let _performance_scope = crate::ui_performance::scope("Workspace::render_file_toolbar");
        let open_files_tooltip = if cfg!(target_os = "macos") {
            crate::tr!("打开日志…（Cmd+O）", "Open log… (Cmd+O)")
        } else {
            crate::tr!("打开日志…（Ctrl+O）", "Open log… (Ctrl+O)")
        };
        let jump_to_end_tooltip = if cfg!(target_os = "macos") {
            crate::tr!("跳到文件末尾（Cmd+End）", "Jump to end of file (Cmd+End)")
        } else {
            crate::tr!("跳到文件末尾（Ctrl+End）", "Jump to end of file (Ctrl+End)")
        };
        let workspace = cx.entity();
        let has_document = self.active_document().is_some();
        let active_file_is_pinned = self.active_file_is_pinned();
        let active_encoding = self.active_document().map(|tab| {
            (
                tab.id,
                SharedString::from(if tab.load_state == DocumentLoadState::Opening {
                    crate::tr!("检测中", "Detecting").to_string()
                } else {
                    tab.document.metadata().encoding_name.clone()
                }),
            )
        });
        let file_size = self
            .active_document()
            .map(|tab| format_bytes(tab.document.metadata().file_size));
        let line_position = self.active_document().map(|tab| {
            format!(
                "Ln {}/{}",
                self.selected_source_row.map_or(1, |row| row + 1),
                tab.document.source_line_count()
            )
        });

        let colors = ui_theme::palette(cx);
        // 工具栏内四个圆角方钮为 34px 见方、3px 间距，与 `Button` 的 24/32px
        // 预设都对不上，所以逐个显式给尺寸。
        let toolbar_icon_button = |button: Button| {
            button
                .ghost()
                .w(px(34.))
                .h(px(34.))
                .rounded(px(10.))
                .flex_shrink_0()
        };
        // file-meta 的每一项之间是一条 `--divider-soft` 竖线，首项不画。
        let file_meta_item = |text: String, leading_divider: bool| {
            div()
                .px(px(9.))
                .text_size(px(11.))
                .text_color(colors.muted_foreground)
                .when(leading_divider, |item| {
                    item.border_l_1().border_color(colors.divider)
                })
                .child(text)
        };

        h_flex()
            .relative()
            .w_full()
            .min_h(px(44.))
            .flex_shrink_0()
            .items_center()
            .gap(px(9.))
            .px(px(12.))
            .py(px(4.))
            .bg(ui_theme::header_material(&colors))
            .border_b_1()
            .border_color(cx.theme().border)
            .child(ui_theme::glass_sheen_layer(&colors))
            .child(
                h_flex()
                    .gap(px(3.))
                    .flex_shrink_0()
                    .child(toolbar_icon_button(
                        Button::new("open-files")
                            .icon(IconName::FolderOpen)
                            .tooltip(open_files_tooltip)
                            .loading(matches!(self.activity, Activity::Opening))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_files(&OpenFiles, window, cx);
                            })),
                    ))
                    .child(toolbar_icon_button(
                        Button::new("reload-active-file")
                            .icon(IconName::Redo)
                            .tooltip(crate::tr!("重新加载（F5）", "Reload (F5)"))
                            .disabled(!has_document)
                            .loading(matches!(self.activity, Activity::Opening))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.reload_active(&ReloadActive, window, cx);
                            })),
                    ))
                    .child(toolbar_icon_button(
                        Button::new("toggle-file-pinned")
                            .icon(if active_file_is_pinned {
                                IconName::StarFill
                            } else {
                                IconName::Star
                            })
                            .selected(active_file_is_pinned)
                            .tooltip(if active_file_is_pinned {
                                crate::tr!("取消收藏当前文件", "Remove current file from favorites")
                            } else {
                                crate::tr!("收藏当前文件", "Favorite current file")
                            })
                            .loading(self.pinned_updating)
                            .disabled(!has_document || self.history_loading)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_active_file_pinned(window, cx);
                            })),
                    ))
                    .child(toolbar_icon_button(
                        Button::new("jump-to-end")
                            .icon(IconName::ArrowDown)
                            .tooltip(jump_to_end_tooltip)
                            .disabled(!has_document)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.jump_to_end(&JumpToEnd, window, cx);
                            })),
                    )),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .h(px(36.))
                    .items_center()
                    .gap(px(8.))
                    .px(px(12.))
                    .rounded(px(12.))
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(colors.control_surface)
                    .text_size(px(12.))
                    .text_color(cx.theme().foreground)
                    .child(
                        div()
                            .text_color(cx.theme().primary)
                            .child(Icon::new(IconName::File).xsmall()),
                    )
                    .child(div().min_w_0().flex_1().truncate().child(
                        self.active_document().map_or_else(
                            || crate::tr!("未打开文件", "No file open").to_string(),
                            |tab| {
                                if self.app_settings.show_full_path {
                                    tab.document.path().display().to_string()
                                } else {
                                    tab.title.to_string()
                                }
                            },
                        ),
                    )),
            )
            .child(
                h_flex()
                    .h(px(36.))
                    .flex_shrink_0()
                    .items_center()
                    .when_some(file_size, |meta, file_size| {
                        meta.child(file_meta_item(file_size, false))
                    })
                    .when_some(active_encoding, |meta, (document_id, encoding_name)| {
                        let menu_encoding_name = encoding_name.clone();
                        let workspace = workspace.clone();
                        meta.child(
                            Button::new("document-encoding")
                                .small()
                                .ghost()
                                .label(encoding_name)
                                .h(px(26.))
                                .px(px(9.))
                                .rounded(px(8.))
                                .text_size(px(11.))
                                .disabled(self.open_task.is_some())
                                .dropdown_menu(move |menu, window, cx| {
                                    Self::build_encoding_menu(
                                        Self::popup_menu_with_workspace_action_context(
                                            menu, &workspace, cx,
                                        ),
                                        document_id,
                                        menu_encoding_name.clone(),
                                        workspace.clone(),
                                        window,
                                    )
                                }),
                        )
                    })
                    .when_some(line_position, |meta, line_position| {
                        meta.child(file_meta_item(line_position, true))
                    }),
            )
    }

    fn render_tabs(&self, has_other_window: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let _performance_scope = crate::ui_performance::scope("Workspace::render_tabs");
        let workspace = cx.entity();
        let source_workspace = cx.weak_entity();
        let tab_count = self.tabs.len();
        let active_tab_id = self.active_tab_id;
        let active_tab_ix = self.active_workspace_tab_ix();
        self.reveal_pending_document_tab();
        let tab_list_items = self
            .tabs
            .iter()
            .map(|tab_id| (*tab_id, self.workspace_tab_title(*tab_id)))
            .collect::<Vec<_>>();
        let tab_list_workspace = workspace.clone();
        let tab_drop_layout = self.tab_drop_layout.clone();
        {
            let mut layout = tab_drop_layout.borrow_mut();
            layout.tabs.resize(tab_count, Bounds::default());
            layout.end = Bounds::default();
        }
        let colors = ui_theme::palette(cx);
        // Large 档的 segmented 标签外框为 36px；指示层内芯由组件按档位固定为 28px，
        // 无法从外部覆写。
        let mut tabs = TabBar::new("document-tabs")
            .w_full()
            .track_scroll(&self.document_tab_scroll)
            .with_size(gpui_component::Size::Large)
            .segmented()
            .h(px(48.))
            .p(px(5.))
            .gap(px(2.))
            .rounded_none()
            .bg(ui_theme::header_material(&colors))
            .suffix(
                Button::new("document-tab-list")
                    .small()
                    .ghost()
                    .dropdown_caret(true)
                    .tooltip(crate::tr!("所有标签", "All tabs"))
                    .dropdown_menu(move |menu, window, cx| {
                        let mut menu = Self::popup_menu_with_workspace_action_context(
                            menu,
                            &tab_list_workspace,
                            cx,
                        );
                        menu = menu.scrollable(true);
                        for (tab_id, title) in &tab_list_items {
                            let tab_id = *tab_id;
                            let workspace = tab_list_workspace.clone();
                            let activate =
                                window.listener_for(&workspace, move |this, _, window, cx| {
                                    this.activate_workspace_tab(tab_id, window, cx);
                                });
                            menu = menu.item(
                                PopupMenuItem::new(title.clone())
                                    .checked(active_tab_id == tab_id)
                                    .on_click(activate),
                            );
                        }
                        menu
                    }),
            )
            .on_click(cx.listener(|this, ix: &usize, window, cx| {
                if let Some(tab_id) = this.tabs.get(*ix).copied() {
                    this.activate_workspace_tab(tab_id, window, cx);
                }
            }));
        if let Some(active_ix) = active_tab_ix {
            tabs = tabs.selected_index(active_ix);
        }

        tabs.children(self.tabs.iter().enumerate().map(|(ix, tab_id)| {
            let tab_id = *tab_id;
            let document_id = tab_id.document_id();
            let tab_title = self.workspace_tab_title(tab_id);
            let can_restore_title = document_id.is_some_and(|document_id| {
                self.documents
                    .iter()
                    .find(|tab| tab.id == document_id)
                    .is_some_and(|tab| tab.custom_title.is_some())
            });
            let dragged_tab = DraggedTab::new(tab_id, tab_title.clone(), source_workspace.clone());
            let tab_menu_state = TabMenuState {
                tab_ix: ix,
                tab_count,
                can_restore_title,
                has_other_window,
            };
            let context_workspace = workspace.clone();
            let tab_layout = tab_drop_layout.clone();
            let selected = self.active_tab_id == tab_id;
            let file_icon_color = if selected {
                cx.theme().tab_active_foreground
            } else {
                cx.theme().tab_foreground
            };
            let (close_button_id, context_target_id) = match tab_id {
                WorkspaceTabId::Document(id) => (
                    ElementId::from(("close-document-tab", id)),
                    ElementId::from(("document-tab-context-target", id)),
                ),
                WorkspaceTabId::New(id) => (
                    ElementId::from(("close-new-tab", id)),
                    ElementId::from(("new-tab-context-target", id)),
                ),
            };
            let close_button = Button::new(close_button_id)
                .xsmall()
                .ghost()
                .icon(IconName::Close)
                .rounded(px(7.))
                .text_color(cx.theme().muted_foreground)
                .tooltip(crate::tr_args!("关闭 {tab_title}", "Close {tab_title}"))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.request_close_workspace_tabs(BTreeSet::from([tab_id]), window, cx);
                }));
            Tab::new()
                .aria_label(tab_title.clone())
                .selected(selected)
                .when(self.cross_window_drop_ix == Some(ix), |this| {
                    this.border_l_2().border_color(cx.theme().primary)
                })
                .on_prepaint(move |bounds, _, _| {
                    if let Some(slot) = tab_layout.borrow_mut().tabs.get_mut(ix) {
                        *slot = bounds;
                    }
                })
                .on_drag(dragged_tab, |dragged, position, _, cx| {
                    cx.new(|_| dragged.clone().position(position))
                })
                .drag_over::<DraggedTab>(move |this, dragged, _, cx| {
                    if dragged.tab_id == tab_id {
                        this
                    } else {
                        this.border_l_2().border_color(cx.theme().primary)
                    }
                })
                .on_drop(cx.listener(move |this, dragged: &DraggedTab, window, cx| {
                    this.reorder_tab(dragged.tab_id, ix, window, cx);
                }))
                .on_aux_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                    if event.is_middle_click() {
                        this.request_close_workspace_tabs(BTreeSet::from([tab_id]), window, cx);
                    }
                }))
                .child(
                    div()
                        .id(context_target_id)
                        .absolute()
                        .top_0()
                        .right_0()
                        .bottom_0()
                        .left_0()
                        .context_menu(move |menu, window, _| match document_id {
                            Some(document_id) => Self::build_tab_menu(
                                menu,
                                document_id,
                                tab_menu_state,
                                context_workspace.clone(),
                                window,
                            ),
                            None => Self::build_new_tab_menu(
                                menu,
                                tab_id,
                                tab_menu_state,
                                context_workspace.clone(),
                                window,
                            ),
                        }),
                )
                .child(
                    h_flex()
                        // Large tabs own a fixed 16px inset with no per-tab override. Reduce the
                        // effective horizontal inset to 10px without changing height or type size.
                        .mx(px(-6.))
                        .gap(px(8.))
                        .items_center()
                        .text_size(px(12.))
                        .child(
                            svg()
                                .data(include_bytes!(
                                    "../assets/icons/document-text-20-regular.svg"
                                ))
                                .size(px(20.))
                                .text_color(file_icon_color)
                                .opacity(0.72),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .line_height(relative(1.5))
                                .child(tab_title),
                        )
                        .child(close_button),
                )
        }))
        .last_empty_space(
            h_flex()
                .id("document-tab-end-drop")
                .h_full()
                .min_w_12()
                .flex_grow_1()
                .when(self.cross_window_drop_ix == Some(tab_count), |this| {
                    this.border_l_2().border_color(cx.theme().primary)
                })
                .on_prepaint({
                    let tab_drop_layout = tab_drop_layout.clone();
                    move |bounds, _, _| tab_drop_layout.borrow_mut().end = bounds
                })
                .drag_over::<DraggedTab>(|this, _, _, cx| {
                    this.border_l_2().border_color(cx.theme().primary)
                })
                .on_drop(cx.listener(move |this, dragged: &DraggedTab, window, cx| {
                    this.reorder_tab(dragged.tab_id, tab_count, window, cx);
                }))
                .child(
                    Button::new("new-workspace-tab")
                        .small()
                        .ghost()
                        .icon(IconName::Plus)
                        .tooltip(crate::tr!("新建标签页", "New tab"))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.create_new_tab(window, cx);
                        })),
                ),
        )
        .map(|tabs| {
            div()
                .id("document-tab-scroll-wheel")
                .w_full()
                .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, window, cx| {
                    this.scroll_document_tabs_from_wheel(event, window, cx);
                }))
                .child(tabs)
        })
    }

    fn highlighted_search_suggestion(value: &str, needle: &str, cx: &App) -> StyledText {
        let normalized_value = value.to_lowercase();
        let normalized_needle = needle.to_lowercase();
        if normalized_needle.is_empty() {
            return StyledText::new(value.to_string());
        }
        let highlights = normalized_value
            .match_indices(&normalized_needle)
            .filter_map(|(start, matched)| {
                let end = start + matched.len();
                (value.is_char_boundary(start) && value.is_char_boundary(end)).then_some((
                    start..end,
                    HighlightStyle {
                        background_color: Some(ui_theme::suggestion_match_highlight(cx)),
                        ..HighlightStyle::default()
                    },
                ))
            })
            .collect::<Vec<_>>();
        StyledText::new(value.to_string()).with_highlights(highlights)
    }

    fn render_search_suggestions(
        &self,
        suggestions: Vec<SearchSuggestion>,
        query: String,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let _performance_scope =
            crate::ui_performance::scope("Workspace::render_search_suggestions");
        let selected_ix = self.search_suggestion_ix;
        let needle = search_autocomplete_needle(&query);
        let suggestion_count = suggestions.len();
        let popup_height = rems(
            SEARCH_SUGGESTION_ROW_HEIGHT_REMS
                * suggestion_count.min(SEARCH_SUGGESTION_MAX_VISIBLE_ROWS) as f32,
        );
        let suggestions = Rc::new(suggestions);
        let workspace = cx.entity();
        let suggestions =
            uniform_list(
                "search-autocomplete-suggestions",
                suggestion_count,
                move |visible_range, _, cx| {
                    visible_range
                        .map(|ix| {
                            let suggestion = suggestions[ix].clone();
                            let selected = selected_ix == Some(ix);
                            let value = suggestion.value.clone();
                            let choose = suggestion.clone();
                            let source = match &suggestion.source {
                                SearchSuggestionSource::History => {
                                    crate::tr!("历史记录", "History").to_string()
                                }
                                SearchSuggestionSource::PredefinedFilter { name } => {
                                    crate::tr_args!(
                                        "预定义过滤器 · {name}",
                                        "Predefined filter · {name}"
                                    )
                                }
                            };
                            let workspace = workspace.clone();
                            v_flex()
                                .id(format!("search-autocomplete-suggestion:{value}"))
                                .w_full()
                                .h(rems(SEARCH_SUGGESTION_ROW_HEIGHT_REMS))
                                .justify_center()
                                .gap_1()
                                .px_3()
                                .when(selected, |row| {
                                    row.border_l_2()
                                        .border_color(cx.theme().primary)
                                        .bg(cx.theme().list_active)
                                })
                                .when(!selected, |row| {
                                    row.hover(|style| style.bg(cx.theme().tokens.list_hover))
                                })
                                .active(|row| row.bg(cx.theme().tokens.list_active))
                                .on_click(move |_, window, cx| {
                                    workspace.update(cx, |this, cx| {
                                        this.accept_search_suggestion(choose.clone(), window, cx);
                                    });
                                })
                                .child(div().w_full().min_w_0().truncate().child(
                                    Self::highlighted_search_suggestion(&value, &needle, cx),
                                ))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(source),
                                )
                                .into_any_element()
                        })
                        .collect()
                },
            )
            .absolute()
            .left_0()
            .right_0()
            .top(relative(1.))
            .mt_1()
            .h(popup_height)
            .track_scroll(&self.search_suggestion_scroll)
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().popover)
            .shadow_lg()
            .occlude()
            .with_animation(
                "search-suggestions-enter",
                Animation::new(TRANSIENT_SURFACE_ENTER_DURATION).with_easing(ease_out_cubic),
                |popup, delta| popup.mt(rems(-0.25 + 0.5 * delta)).opacity(delta),
            );
        deferred(suggestions)
            .with_priority(POPUP_PRIORITY)
            .into_any_element()
    }

    fn render_predefined_filters_popover(
        &self,
        has_document: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let _performance_scope =
            crate::ui_performance::scope("Workspace::render_predefined_filters_popover");
        let workspace = cx.entity();
        let filters = self.predefined_filters.clone();
        let query = self.query.read(cx).value().to_string();
        let saving = self.predefined_filters_saving;

        Popover::new("predefined-filters-popover")
            .w_80()
            .p_0()
            .text_sm()
            .trigger(
                Button::new("predefined-filters")
                    .small()
                    .h(SEARCH_CONTROL_HEIGHT)
                    .outline()
                    .icon(IconName::BookOpen)
                    .label(crate::tr!("过滤器", "Filters"))
                    .dropdown_caret(true)
                    .loading(saving)
                    .disabled(!has_document)
                    .tooltip(crate::tr!(
                        "选择、组合或编辑预定义过滤器",
                        "Select, combine, or edit predefined filters"
                    )),
            )
            .content(move |_, window, popover_cx| {
                let filters = filters.clone();
                let filter_count = filters.len();
                let mut options = v_flex().w_full().gap_1().p_2().pr_4();

                if filters.is_empty() {
                    options = options.child(
                        v_flex()
                            .items_center()
                            .gap_1()
                            .px_4()
                            .py_6()
                            .text_color(popover_cx.theme().muted_foreground)
                            .child(crate::tr!("尚未配置过滤器", "No filters configured"))
                            .child(div().text_xs().child(crate::tr!(
                                "可从下方进入编辑器添加",
                                "Open the editor below to add one"
                            ))),
                    );
                } else {
                    for filter in filters {
                        let checked = query_includes_filter(&query, &filter.value);
                        let selected_filter = filter.clone();
                        let filter_value = filter.value.clone();
                        let choose_workspace = workspace.clone();
                        let choose = window.listener_for(
                            &choose_workspace,
                            move |this, checked: &bool, window, cx| {
                                this.choose_predefined_filter(
                                    selected_filter.clone(),
                                    *checked,
                                    window,
                                    cx,
                                );
                            },
                        );

                        options = options.child(
                            Checkbox::new(format!("predefined-filter-option:{}", filter.id))
                                .small()
                                .w_full()
                                .px_3()
                                .py_2()
                                .rounded(popover_cx.theme().radius)
                                .checked(checked)
                                .label(filter.name.clone())
                                .tooltip(filter_value.clone())
                                .when(checked, |option| option.bg(popover_cx.theme().list_active))
                                .when(!checked, |option| {
                                    option.hover(|style| {
                                        style.bg(popover_cx.theme().tokens.list_hover)
                                    })
                                })
                                .on_click(choose)
                                .child(
                                    h_flex()
                                        .w_full()
                                        .min_w_0()
                                        .gap_2()
                                        .child(
                                            div()
                                                .min_w_0()
                                                .flex_1()
                                                .truncate()
                                                .text_xs()
                                                .text_color(popover_cx.theme().muted_foreground)
                                                .child(filter_value),
                                        )
                                        .when(filter.use_regex, |preview| {
                                            preview.child(
                                                div()
                                                    .flex_none()
                                                    .rounded_full()
                                                    .px_2()
                                                    .py_0p5()
                                                    .bg(popover_cx.theme().primary.opacity(0.12))
                                                    .text_xs()
                                                    .text_color(popover_cx.theme().primary)
                                                    .child(".*"),
                                            )
                                        }),
                                ),
                        );
                    }
                }

                let list = options
                    .when(filter_count > 4, |list| list.h_64())
                    .when(filter_count <= 4, |list| list.max_h_64())
                    .overflow_y_scrollbar()
                    .id("predefined-filter-options-scroll");

                let edit_workspace = workspace.clone();
                let edit = popover_cx.listener(move |popover, _, window, cx| {
                    popover.dismiss(window, cx);
                    edit_workspace.update(cx, |workspace, cx| {
                        workspace.open_predefined_filters_dialog(window, cx);
                    });
                });

                v_flex().w_full().child(list).child(
                    div()
                        .w_full()
                        .border_t_1()
                        .border_color(popover_cx.theme().border)
                        .p_2()
                        .child(
                            Button::new("edit-predefined-filters")
                                .small()
                                .ghost()
                                .w_full()
                                .justify_start()
                                .icon(IconName::Settings2)
                                .label(crate::tr!("编辑预定义过滤器…", "Edit predefined filters…"))
                                .on_click(edit),
                        ),
                )
            })
            .into_any_element()
    }

    fn render_search_scope_menu_row(
        label: &'static str,
        icon: IconName,
        selected: bool,
        trailing: Option<AnyElement>,
        cx: &mut App,
    ) -> AnyElement {
        const POPUP_MENU_ITEM_HORIZONTAL_INSET: Pixels = px(8.);
        const POPUP_MENU_ITEM_RADIUS_CAP: Pixels = px(8.);

        h_flex()
            .relative()
            .self_stretch()
            .w_full()
            .min_w_0()
            .justify_between()
            .gap_3()
            .when(selected, |row| {
                row.text_color(cx.theme().accent_foreground).child(
                    // PopupMenu wraps custom content with an 8 px horizontal inset.
                    // Expand this layer back to the owning item so its geometry is
                    // identical to the menu's native hover background.
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left(-POPUP_MENU_ITEM_HORIZONTAL_INSET)
                        .right(-POPUP_MENU_ITEM_HORIZONTAL_INSET)
                        .rounded(cx.theme().radius.min(POPUP_MENU_ITEM_RADIUS_CAP))
                        .bg(cx.theme().tokens.accent),
                )
            })
            .child(
                h_flex()
                    .relative()
                    .min_w_0()
                    .flex_1()
                    .gap_1()
                    .child(Icon::new(icon).xsmall())
                    .child(div().min_w_0().flex_1().child(label)),
            )
            .when_some(trailing, |row, trailing| row.child(trailing))
            .into_any_element()
    }

    fn render_search_scope_control(
        &self,
        has_document: bool,
        tooltip: String,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let _performance_scope =
            crate::ui_performance::scope("Workspace::render_search_scope_control");
        let workspace = cx.entity();
        let menu_workspace = workspace.clone();
        let selected_scope = self.global_search.scope;
        let has_scope_settings = matches!(
            self.global_search.scope,
            SearchScope::AllOpenFiles | SearchScope::Directory
        );
        let settings_button = match self.global_search.scope {
            SearchScope::CurrentFile => None,
            SearchScope::AllOpenFiles => Some(
                Button::new("search-scope-global-settings")
                    .small()
                    .h(SEARCH_CONTROL_HEIGHT)
                    .outline()
                    .rounded_l_none()
                    .border_l_0()
                    .icon(IconName::Settings2)
                    .disabled(!has_document)
                    .tooltip(crate::tr!("配置全局搜索…", "Configure global search…"))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_global_search_files_dialog(window, cx);
                    })),
            ),
            SearchScope::Directory => Some(
                Button::new("search-scope-directory-settings")
                    .small()
                    .h(SEARCH_CONTROL_HEIGHT)
                    .outline()
                    .rounded_l_none()
                    .border_l_0()
                    .icon(IconName::Settings2)
                    .disabled(!has_document)
                    .tooltip(crate::tr!("配置目录搜索…", "Configure directory search…"))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_directory_search_dialog(window, cx);
                    })),
            ),
        };
        let scope_button = Button::new("search-scope")
            .small()
            .h(SEARCH_CONTROL_HEIGHT)
            .outline()
            .dropdown_caret(true)
            .when(has_scope_settings, |button| button.rounded_r_none())
            .icon(match self.global_search.scope {
                SearchScope::CurrentFile => IconName::Search,
                SearchScope::AllOpenFiles => IconName::File,
                SearchScope::Directory => IconName::FolderOpen,
            })
            .label(match self.global_search.scope {
                SearchScope::CurrentFile => crate::tr!("当前", "Current"),
                SearchScope::AllOpenFiles => crate::tr!("全局", "Global"),
                SearchScope::Directory => crate::tr!("目录", "Directory"),
            })
            .disabled(!has_document)
            .tooltip(tooltip)
            .dropdown_menu(move |menu, window, cx| {
                let menu =
                    Self::popup_menu_with_workspace_action_context(menu, &menu_workspace, cx);
                let current_workspace = menu_workspace.clone();
                let multi_workspace = menu_workspace.clone();
                let directory_workspace = menu_workspace.clone();
                let multi_options_workspace = menu_workspace.clone();
                let directory_options_workspace = menu_workspace.clone();

                menu.min_w(window.rem_size() * 10.)
                    .item(
                        PopupMenuItem::element(move |_, cx| {
                            Self::render_search_scope_menu_row(
                                crate::tr!("当前文件", "Current file"),
                                IconName::Search,
                                selected_scope == SearchScope::CurrentFile,
                                None,
                                cx,
                            )
                        })
                        .on_click(window.listener_for(
                            &current_workspace,
                            |this, _, window, cx| {
                                this.set_search_scope(SearchScope::CurrentFile, window, cx)
                            },
                        )),
                    )
                    .item(
                        PopupMenuItem::element(move |_, cx| {
                            let workspace = multi_options_workspace.clone();
                            let trailing = Button::new("search-scope-multi-options")
                                .relative()
                                .xsmall()
                                .ghost()
                                .icon(IconName::Settings2)
                                .tooltip(crate::tr!(
                                    "选择参与搜索的标签…",
                                    "Select tabs to search…"
                                ))
                                .on_click(move |_, window, cx| {
                                    workspace.update(cx, |this, cx| {
                                        this.set_search_scope(
                                            SearchScope::AllOpenFiles,
                                            window,
                                            cx,
                                        );
                                        this.open_global_search_files_dialog(window, cx);
                                    });
                                })
                                .into_any_element();
                            Self::render_search_scope_menu_row(
                                crate::tr!("全局搜索", "Global search"),
                                IconName::File,
                                selected_scope == SearchScope::AllOpenFiles,
                                Some(trailing),
                                cx,
                            )
                        })
                        .on_click(window.listener_for(
                            &multi_workspace,
                            |this, _, window, cx| {
                                this.set_search_scope(SearchScope::AllOpenFiles, window, cx)
                            },
                        )),
                    )
                    .item(
                        PopupMenuItem::element(move |_, cx| {
                            let workspace = directory_options_workspace.clone();
                            let trailing = Button::new("search-scope-directory-options")
                                .relative()
                                .xsmall()
                                .ghost()
                                .icon(IconName::Settings2)
                                .tooltip(crate::tr!(
                                    "设置目录搜索范围…",
                                    "Set directory search scope…"
                                ))
                                .on_click(move |_, window, cx| {
                                    workspace.update(cx, |this, cx| {
                                        this.set_search_scope(SearchScope::Directory, window, cx);
                                        this.open_directory_search_dialog(window, cx);
                                    });
                                })
                                .into_any_element();
                            Self::render_search_scope_menu_row(
                                crate::tr!("目录搜索", "Directory search"),
                                IconName::FolderOpen,
                                selected_scope == SearchScope::Directory,
                                Some(trailing),
                                cx,
                            )
                        })
                        .on_click(window.listener_for(
                            &directory_workspace,
                            |this, _, window, cx| {
                                this.set_search_scope(SearchScope::Directory, window, cx)
                            },
                        )),
                    )
            });

        h_flex()
            .flex_none()
            .child(scope_button)
            .when_some(settings_button, |control, settings_button| {
                control.child(settings_button)
            })
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, _, window, cx| {
                    if !has_document {
                        return;
                    }

                    window.prevent_default();
                    cx.stop_propagation();
                    GlobalState::suppress_text_selection(cx);
                    match this.global_search.scope {
                        SearchScope::CurrentFile => {}
                        SearchScope::AllOpenFiles => {
                            this.open_global_search_files_dialog(window, cx);
                        }
                        SearchScope::Directory => {
                            this.open_directory_search_dialog(window, cx);
                        }
                    }
                }),
            )
            .into_any_element()
    }

    fn render_quick_find_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let _performance_scope = crate::ui_performance::scope("Workspace::render_quick_find_bar");
        let colors = ui_theme::palette(cx);
        let query_empty = self.quick_find.query.read(cx).value().is_empty();
        let invalid_message = self.quick_find.error.as_deref();
        let target_label = match self.quick_find.target {
            Some(QuickFindTarget::Log(_)) => crate::tr!("正文", "Log"),
            Some(QuickFindTarget::Results(_)) => crate::tr!("当前结果", "Current results"),
            Some(QuickFindTarget::GlobalResults) => crate::tr!("全局结果", "Global results"),
            None => crate::tr!("当前视图", "Current view"),
        };
        let boundary_message = (!self.quick_find.no_match && invalid_message.is_none())
            .then(|| {
                self.quick_find.boundary.map(|boundary| match boundary {
                    QuickFindBoundary::Start => crate::tr!(
                        "已到达开头，没有更早的匹配项",
                        "Reached the beginning; there are no earlier matches"
                    ),
                    QuickFindBoundary::End => crate::tr!(
                        "已到达末尾，没有更多匹配项",
                        "Reached the end; there are no more matches"
                    ),
                })
            })
            .flatten();
        let status_message = invalid_message.or_else(|| {
            self.quick_find
                .no_match
                .then_some(crate::tr!("没有找到匹配项", "No matches found"))
                .or(boundary_message)
        });
        let input_label = status_message.map_or_else(
            || crate::tr_args!("在{target_label}中查找", "Find in {target_label}"),
            |message| {
                crate::tr_args!(
                    "在{target_label}中查找；{message}",
                    "Find in {target_label}; {message}"
                )
            },
        );
        let previous_tooltip = if let Some(message) = invalid_message {
            message
        } else if self.quick_find.no_match {
            crate::tr!("没有找到匹配项", "No matches found")
        } else if self.quick_find.boundary == Some(QuickFindBoundary::Start) {
            crate::tr!(
                "已到达开头，没有更早的匹配项",
                "Reached the beginning; there are no earlier matches"
            )
        } else {
            crate::tr!(
                "查找上一处（Shift+Enter / Shift+F3）",
                "Find previous (Shift+Enter / Shift+F3)"
            )
        };
        let next_tooltip = if let Some(message) = invalid_message {
            message
        } else if self.quick_find.no_match {
            crate::tr!("没有找到匹配项", "No matches found")
        } else if self.quick_find.boundary == Some(QuickFindBoundary::End) {
            crate::tr!(
                "已到达末尾，没有更多匹配项",
                "Reached the end; there are no more matches"
            )
        } else {
            crate::tr!("查找下一处（Enter / F3）", "Find next (Enter / F3)")
        };
        let controls_disabled = query_empty || self.quick_find.busy || invalid_message.is_some();
        let case_sensitive_tooltip = if self.quick_find.case_sensitive {
            crate::tr!("关闭匹配大小写", "Turn off match case")
        } else {
            crate::tr!("匹配大小写", "Match case")
        };
        let whole_word_tooltip = if self.quick_find.whole_word {
            crate::tr!("关闭全词匹配", "Turn off whole-word matching")
        } else {
            crate::tr!("全词匹配", "Whole-word matching")
        };
        let regex_tooltip = if self.quick_find.regex {
            crate::tr!("关闭正则表达式", "Turn off regular expressions")
        } else {
            crate::tr!("使用正则表达式", "Use regular expressions")
        };
        let option_icon_color = |selected| {
            if selected {
                cx.theme().primary
            } else {
                cx.theme().foreground.opacity(0.9)
            }
        };

        div()
            .id("quick-find-overlay-anchor")
            .absolute()
            .top_3()
            .left_6()
            .right_6()
            .flex()
            .justify_end()
            .child(
                h_flex()
                    .id("quick-find-bar")
                    .w_96()
                    .max_w_full()
                    .min_w_0()
                    .gap_0p5()
                    .px_1p5()
                    .py_1p5()
                    .rounded(cx.theme().radius_lg)
                    .border_1()
                    .border_color(if self.quick_find.no_match || invalid_message.is_some() {
                        cx.theme().danger
                    } else if boundary_message.is_some() {
                        cx.theme().warning
                    } else {
                        cx.theme().border
                    })
                    .bg(colors.surface)
                    .text_color(cx.theme().popover_foreground)
                    .shadow_lg()
                    .occlude()
                    .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                        match event.keystroke.key.as_str() {
                            "escape" => {
                                this.close_quick_find(window, cx);
                                cx.stop_propagation();
                            }
                            "f3" => {
                                this.start_quick_find(
                                    if event.keystroke.modifiers.shift {
                                        QuickFindDirection::Backward
                                    } else {
                                        QuickFindDirection::Forward
                                    },
                                    false,
                                    window,
                                    cx,
                                );
                                cx.stop_propagation();
                            }
                            _ => {}
                        }
                    }))
                    .child(
                        div().flex_1().min_w_0().h_8().child(
                            Input::new(&self.quick_find.query)
                                .size_full()
                                .bg(colors.control_surface)
                                .accessibility_id("A150")
                                .aria_label(input_label)
                                .prefix(Icon::new(IconName::Search))
                                .suffix(
                                    h_flex()
                                        .flex_none()
                                        .gap_0p5()
                                        .child(
                                            Button::new("quick-find-case-sensitive")
                                                .xsmall()
                                                .compact()
                                                .text()
                                                .size(px(20.))
                                                .p_0()
                                                .rounded(px(4.))
                                                .accessibility_id("A463")
                                                .selected(self.quick_find.case_sensitive)
                                                .toggled(self.quick_find.case_sensitive)
                                                .tooltip(case_sensitive_tooltip)
                                                .child(
                                                    h_flex()
                                                        .relative()
                                                        .size_full()
                                                        .justify_center()
                                                        .child(
                                                            svg()
                                                                .data(include_bytes!(
                                                                    "../assets/icons/case-sensitive.svg"
                                                                ))
                                                                .size(px(15.))
                                                                .text_color(option_icon_color(
                                                                    self.quick_find.case_sensitive,
                                                                )),
                                                        )
                                                        .child(
                                                            div()
                                                                .absolute()
                                                                .w_0()
                                                                .h_0()
                                                                .overflow_hidden()
                                                                .opacity(0.)
                                                                .child(crate::tr!("匹配大小写", "Match case")),
                                                        ),
                                                )
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.toggle_quick_find_case_sensitive(
                                                        window, cx,
                                                    );
                                                })),
                                        )
                                        .child(
                                            Button::new("quick-find-whole-word")
                                                .xsmall()
                                                .compact()
                                                .text()
                                                .size(px(20.))
                                                .p_0()
                                                .rounded(px(4.))
                                                .accessibility_id("A464")
                                                .selected(self.quick_find.whole_word)
                                                .toggled(self.quick_find.whole_word)
                                                .tooltip(whole_word_tooltip)
                                                .child(
                                                    h_flex()
                                                        .relative()
                                                        .size_full()
                                                        .justify_center()
                                                        .child(
                                                            svg()
                                                                .data(include_bytes!(
                                                                    "../assets/icons/whole-word.svg"
                                                                ))
                                                                .size(px(15.))
                                                                .text_color(option_icon_color(
                                                                    self.quick_find.whole_word,
                                                                )),
                                                        )
                                                        .child(
                                                            div()
                                                                .absolute()
                                                                .w_0()
                                                                .h_0()
                                                                .overflow_hidden()
                                                                .opacity(0.)
                                                                .child(crate::tr!("全词匹配", "Whole word")),
                                                        ),
                                                )
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.toggle_quick_find_whole_word(window, cx);
                                                })),
                                        )
                                        .child(
                                            Button::new("quick-find-regex")
                                                .xsmall()
                                                .compact()
                                                .text()
                                                .size(px(20.))
                                                .p_0()
                                                .rounded(px(4.))
                                                .accessibility_id("A465")
                                                .selected(self.quick_find.regex)
                                                .toggled(self.quick_find.regex)
                                                .tooltip(regex_tooltip)
                                                .child(
                                                    h_flex()
                                                        .relative()
                                                        .size_full()
                                                        .justify_center()
                                                        .child(
                                                            svg()
                                                                .data(include_bytes!(
                                                                    "../assets/icons/regex.svg"
                                                                ))
                                                                .size(px(15.))
                                                                .text_color(option_icon_color(
                                                                    self.quick_find.regex,
                                                                )),
                                                        )
                                                        .child(
                                                            div()
                                                                .absolute()
                                                                .w_0()
                                                                .h_0()
                                                                .overflow_hidden()
                                                                .opacity(0.)
                                                                .child(crate::tr!("正则表达式", "Regular expression")),
                                                        ),
                                                )
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.toggle_quick_find_regex(window, cx);
                                                })),
                                        ),
                                ),
                        ),
                    )
                    .child(
                        Button::new("quick-find-previous")
                            .ghost()
                            .icon(IconName::ArrowUp)
                            .loading(
                                self.quick_find.busy
                                    && self.quick_find.direction
                                        == Some(QuickFindDirection::Backward),
                            )
                            .disabled(controls_disabled)
                            .tooltip(previous_tooltip)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.start_quick_find(
                                    QuickFindDirection::Backward,
                                    false,
                                    window,
                                    cx,
                                );
                            })),
                    )
                    .child(
                        Button::new("quick-find-next")
                            .ghost()
                            .icon(IconName::ArrowDown)
                            .loading(
                                self.quick_find.busy
                                    && self.quick_find.direction
                                        == Some(QuickFindDirection::Forward),
                            )
                            .disabled(controls_disabled)
                            .tooltip(next_tooltip)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.start_quick_find(
                                    QuickFindDirection::Forward,
                                    false,
                                    window,
                                    cx,
                                );
                            })),
                    )
                    .child(
                        Button::new("quick-find-close")
                            .ghost()
                            .icon(IconName::Close)
                            .tooltip(crate::tr!("关闭页内查找（Esc）", "Close find (Esc)"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.close_quick_find(window, cx);
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_search_bar(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _performance_scope = crate::ui_performance::scope("Workspace::render_search_bar");
        let colors = ui_theme::palette(cx);
        let search_history_empty = self.search_history.is_empty();
        let has_document = self.active_document().is_some();
        let predefined_filters = self.render_predefined_filters_popover(has_document, cx);
        let active_document_ready = self
            .active_document()
            .is_some_and(|tab| tab.load_state == DocumentLoadState::Ready);
        let query_value = self.query.read(cx).value().to_string();
        let query_empty = query_value.is_empty();
        let search_suggestions = self.search_autocomplete_suggestions(cx);
        let show_search_suggestions =
            !search_suggestions.is_empty() && self.query.focus_handle(cx).is_focused(window);
        let search_history_open = self.search_autocomplete_mode == SearchAutocompleteMode::History;
        let global_selected_count = self.global_search.selected_documents.len();
        let search_scope_tooltip = match self.global_search.scope {
            SearchScope::CurrentFile => crate::tr!(
                "选择搜索范围（当前文件）",
                "Choose search scope (current file)"
            )
            .to_string(),
            SearchScope::AllOpenFiles => crate::tr_args!(
                "左键选择搜索范围；右键配置全局搜索（{} / {}）",
                "Left-click to choose scope; right-click to configure global search ({} / {})",
                global_selected_count,
                self.documents.len(),
            ),
            SearchScope::Directory => self
                .global_search
                .directory_options
                .directory
                .as_deref()
                .map(|directory| {
                    crate::tr_args!(
                        "左键选择搜索范围；右键配置目录搜索：{}",
                        "Left-click to choose scope; right-click to configure directory search: {}",
                        directory.display(),
                    )
                })
                .unwrap_or_else(|| {
                    crate::tr!(
                        "左键选择搜索范围；右键配置目录搜索",
                        "Left-click to choose scope; right-click to configure directory search"
                    )
                    .to_string()
                }),
        };
        let case_sensitive_variant = ButtonCustomVariant::new(cx)
            .color(cx.theme().transparent)
            .foreground(if self.case_sensitive {
                cx.theme().primary
            } else {
                cx.theme().foreground
            })
            .hover(cx.theme().muted)
            .active(cx.theme().primary.opacity(0.18));
        let regex_variant = ButtonCustomVariant::new(cx)
            .color(cx.theme().transparent)
            .foreground(if self.regex {
                cx.theme().primary
            } else {
                cx.theme().foreground
            })
            .hover(cx.theme().muted)
            .active(cx.theme().primary.opacity(0.18));
        let (result_mode_select, result_count_label, committed_results_visible) =
            match self.global_search.scope {
                SearchScope::CurrentFile => self.active_document().map_or(
                    (None, crate::tr!("0 条结果", "0 results").to_string(), false),
                    |tab| {
                        let truncation =
                            if tab.search_result.truncated && tab.result_mode.includes_matches() {
                                crate::tr!(" · 已截断", " · truncated")
                            } else {
                                ""
                            };
                        (
                            Some(tab.result_mode_select.clone()),
                            crate::tr_args!(
                                "{} 条结果{truncation}",
                                "{} results{truncation}",
                                tab.result_row_count(cx)
                            ),
                            tab.results_visible,
                        )
                    },
                ),
                SearchScope::AllOpenFiles | SearchScope::Directory => {
                    let delegate = self.global_table.read(cx).delegate();
                    let truncation = if delegate.has_truncated_results() {
                        crate::tr!(" · 已截断", " · truncated")
                    } else {
                        ""
                    };
                    (
                        Some(self.global_search.result_mode_select.clone()),
                        crate::tr_args!(
                            "{} 条 · {} 个文件{truncation}",
                            "{} results · {} files{truncation}",
                            delegate.results_count(),
                            delegate.groups_count(),
                        ),
                        self.global_search.results_visible,
                    )
                }
            };
        let search_disabled = match self.global_search.scope {
            SearchScope::CurrentFile => !active_document_ready,
            SearchScope::AllOpenFiles => {
                !has_document
                    || global_selected_count == 0
                    || self.documents.iter().any(|tab| {
                        self.global_search.selected_documents.contains(&tab.id)
                            && tab.load_state != DocumentLoadState::Ready
                    })
            }
            SearchScope::Directory => self.global_search.directory_options.directory.is_none(),
        };
        let clear_disabled = !has_document
            || (query_empty
                && match self.global_search.scope {
                    SearchScope::CurrentFile => self
                        .active_document()
                        .is_none_or(|tab| !tab.results_visible),
                    SearchScope::AllOpenFiles | SearchScope::Directory => {
                        !self.global_search.results_visible
                    }
                });
        let active_document_id = self.active_document().map(|tab| tab.id);
        let searching_current_scope = match self.global_search.scope {
            SearchScope::CurrentFile => active_document_id.is_some_and(|document_id| {
                self.searches
                    .has_target(SearchTarget::Document(document_id))
            }),
            SearchScope::AllOpenFiles => self.searches.has_target(SearchTarget::AllOpenFiles),
            SearchScope::Directory => self.searches.has_target(SearchTarget::Directory),
        };
        let search_scope_control =
            self.render_search_scope_control(has_document, search_scope_tooltip, cx);

        v_flex()
            .w_full()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .relative()
                    .w_full()
                    .min_h(px(50.))
                    .items_center()
                    .gap(px(6.))
                    .px(px(12.))
                    .py(SEARCH_BAR_VERTICAL_INSET)
                    .bg(ui_theme::header_material(&colors))
                    .child(ui_theme::glass_sheen_layer(&colors))
                    .when_some(result_mode_select, |controls, result_mode_select| {
                        controls.child(
                            div().w(px(110.)).h(px(34.)).flex_none().child(
                                Select::new(&result_mode_select)
                                    .small()
                                    .h(px(34.))
                                    .focus_ring(false),
                            ),
                        )
                    })
                    .child(
                        Button::new("case-sensitive")
                            .small()
                            .w(px(34.))
                            .h(px(34.))
                            .p_0()
                            .rounded(px(10.))
                            .font_weight(FontWeight(700.))
                            .custom(case_sensitive_variant)
                            .label("Aa")
                            .selected(self.case_sensitive)
                            .toggled(self.case_sensitive)
                            .disabled(!has_document)
                            .tooltip(crate::tr!("区分大小写（Alt+C）", "Case-sensitive (Alt+C)"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_case_sensitive(&ToggleCaseSensitive, window, cx);
                            })),
                    )
                    .child(
                        Button::new("regular-expression")
                            .small()
                            .w(px(34.))
                            .h(px(34.))
                            .p_0()
                            .rounded(px(10.))
                            .font_weight(FontWeight(700.))
                            .custom(regex_variant)
                            .label(".*")
                            .selected(self.regex)
                            .toggled(self.regex)
                            .disabled(!has_document)
                            .tooltip(crate::tr!("使用正则表达式", "Use regular expressions"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_regex(&ToggleRegex, window, cx);
                            })),
                    )
                    .child(predefined_filters)
                    .child(search_scope_control)
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(180.))
                            .h(px(34.))
                            .relative()
                            .capture_key_down(cx.listener(
                                |this, event: &KeyDownEvent, window, cx| {
                                    if !this.query.focus_handle(cx).is_focused(window)
                                        || event.keystroke.modifiers.control
                                        || event.keystroke.modifiers.platform
                                    {
                                        return;
                                    }
                                    if this.navigate_search_autocomplete_by_key(
                                        event.keystroke.key.as_str(),
                                        cx,
                                    ) {
                                        cx.stop_propagation();
                                    }
                                },
                            ))
                            .on_scroll_wheel(cx.listener(
                                |this, event: &ScrollWheelEvent, window, cx| {
                                    let delta_y = event.delta.pixel_delta(window.line_height()).y;
                                    if delta_y == px(0.)
                                        || event.modifiers.control
                                        || event.modifiers.platform
                                    {
                                        return;
                                    }
                                    this.query.focus_handle(cx).focus(window, cx);
                                    if this.navigate_search_history_by_wheel(
                                        delta_y > px(0.),
                                        window,
                                        cx,
                                    ) {
                                        cx.stop_propagation();
                                    }
                                },
                            ))
                            .child(
                                Input::new(&self.query)
                                    .small()
                                    .size_full()
                                    .prefix(
                                        Icon::new(IconName::Search)
                                            .text_color(cx.theme().muted_foreground),
                                    )
                                    .suffix(
                                        Button::new("search-history")
                                            .xsmall()
                                            .ghost()
                                            .icon(IconName::ChevronDown)
                                            .selected(search_history_open)
                                            .disabled(search_history_empty)
                                            .tooltip(if search_history_empty {
                                                crate::tr!("暂无搜索历史", "No search history")
                                            } else if search_history_open {
                                                crate::tr!("收起搜索历史", "Hide search history")
                                            } else {
                                                crate::tr!("显示搜索历史", "Show search history")
                                            })
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.toggle_search_history_popup(window, cx);
                                            })),
                                    ),
                            )
                            .when(show_search_suggestions, |input| {
                                input.child(self.render_search_suggestions(
                                    search_suggestions,
                                    query_value,
                                    cx,
                                ))
                            }),
                    )
                    .child(
                        Button::new("start-search")
                            .small()
                            .primary()
                            .icon(IconName::Search)
                            .label(crate::tr!("搜索", "Search"))
                            .min_w(px(88.))
                            .h(px(34.))
                            .rounded(px(10.))
                            .loading(searching_current_scope)
                            .disabled(search_disabled)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.start_search(window, cx);
                            })),
                    )
                    .child(
                        Button::new("clear-search")
                            .small()
                            .ghost()
                            .icon(IconName::Close)
                            .w(px(34.))
                            .h(px(34.))
                            .rounded(px(10.))
                            .disabled(clear_disabled)
                            .tooltip(crate::tr!("清除搜索结果", "Clear search results"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.clear_search(window, cx);
                            })),
                    )
                    .when(committed_results_visible, |controls| {
                        controls.child(
                            h_flex()
                                .flex_shrink_0()
                                .text_color(cx.theme().muted_foreground)
                                .child(
                                    h_flex()
                                        .min_w(px(68.))
                                        .h(px(24.))
                                        .justify_center()
                                        .px(px(8.))
                                        .rounded(px(999.))
                                        .border_1()
                                        .border_color(cx.theme().primary.opacity(0.24))
                                        .bg(cx.theme().primary.opacity(0.08))
                                        .text_size(px(11.))
                                        .font_weight(FontWeight(650.))
                                        .text_color(cx.theme().primary)
                                        .child(result_count_label),
                                ),
                        )
                    }),
            )
    }

    fn render_pinned_files(&self, opening: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let _performance_scope = crate::ui_performance::scope("Workspace::render_pinned_files");
        let hidden_count = self.pinned_files.len().saturating_sub(8);
        v_flex()
            .w_full()
            .rounded(cx.theme().radius_lg * 2.)
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().group_box)
            .shadow_lg()
            .child(
                h_flex()
                    .justify_between()
                    .gap_2()
                    .h(rems(EMPTY_WORKSPACE_CARD_HEADER_HEIGHT_REMS))
                    .px_5()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .text_sm()
                            .child(crate::tr!("收藏文件", "Favorite files")),
                    )
                    .child(
                        Button::new("clear-pinned-files")
                            .xsmall()
                            .ghost()
                            .text_color(cx.theme().primary)
                            .label(crate::tr!("清空收藏", "Clear favorites"))
                            .loading(self.pinned_updating)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.clear_pinned_files(window, cx);
                            })),
                    ),
            )
            .child(
                v_flex()
                    .children(self.pinned_files.iter().take(8).map(|file| {
                        let path = file.path.clone();
                        Button::new(("pinned-file", file.id.unsigned_abs()))
                            .small()
                            .ghost()
                            .w_full()
                            .h(rems(EMPTY_WORKSPACE_FILE_ROW_HEIGHT_REMS))
                            .px_5()
                            .rounded(ButtonRounded::None)
                            .child(empty_file_button_content(
                                &file.path,
                                Some(file.last_opened_at),
                                cx,
                            ))
                            .disabled(opening)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_recent_file(path.clone(), window, cx);
                            }))
                    }))
                    .when(hidden_count > 0, |this| {
                        this.child(
                            div()
                                .px_5()
                                .py_2()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(crate::tr_args!(
                                    "另有 {hidden_count} 个收藏文件",
                                    "{hidden_count} more favorite files"
                                )),
                        )
                    }),
            )
    }

    fn render_last_workspace_files(
        &self,
        opening: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let _performance_scope =
            crate::ui_performance::scope("Workspace::render_last_workspace_files");
        let hidden_count = self.last_workspace_files.len().saturating_sub(8);
        v_flex()
            .w_full()
            .rounded(cx.theme().radius_lg * 2.)
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().group_box)
            .shadow_lg()
            .child(
                h_flex()
                    .justify_between()
                    .gap_2()
                    .h(rems(EMPTY_WORKSPACE_CARD_HEADER_HEIGHT_REMS))
                    .px_5()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .text_sm()
                            .child(crate::tr!("上一次文件", "Previous files")),
                    )
                    .child(
                        Button::new("restore-last-workspace")
                            .xsmall()
                            .ghost()
                            .text_color(cx.theme().primary)
                            .label(crate::tr!("全部打开", "Open all"))
                            .disabled(opening)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.restore_last_workspace(window, cx);
                            })),
                    ),
            )
            .child(
                v_flex()
                    .children(self.last_workspace_files.iter().take(8).map(|file| {
                        let path = file.path.clone();
                        Button::new(("last-workspace-file", file.id.unsigned_abs()))
                            .small()
                            .ghost()
                            .w_full()
                            .h(rems(EMPTY_WORKSPACE_FILE_ROW_HEIGHT_REMS))
                            .px_5()
                            .rounded(ButtonRounded::None)
                            .child(empty_file_button_content(
                                &file.path,
                                Some(file.last_opened_at),
                                cx,
                            ))
                            .disabled(opening)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_recent_file(path.clone(), window, cx);
                            }))
                    }))
                    .when(hidden_count > 0, |this| {
                        this.child(
                            div()
                                .px_5()
                                .py_2()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(crate::tr_args!(
                                    "另有 {hidden_count} 个文件，可使用全部打开",
                                    "{hidden_count} more files; use Open all to open them"
                                )),
                        )
                    }),
            )
    }

    fn render_recent_files(&self, opening: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let _performance_scope = crate::ui_performance::scope("Workspace::render_recent_files");
        v_flex()
            .w_full()
            .rounded(cx.theme().radius_lg * 2.)
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().group_box)
            .shadow_lg()
            .child(
                h_flex()
                    .justify_between()
                    .gap_2()
                    .h(rems(EMPTY_WORKSPACE_CARD_HEADER_HEIGHT_REMS))
                    .px_5()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .text_sm()
                            .child(crate::tr!("最近文件", "Recent files")),
                    )
                    .when(
                        !self.history_loading && !self.recent_files.is_empty(),
                        |this| {
                            this.child(
                                Button::new("open-file-history")
                                    .xsmall()
                                    .ghost()
                                    .text_color(cx.theme().primary)
                                    .label(crate::tr!("查看全部", "View all"))
                                    .loading(self.history_dialog_loading)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.open_history_dialog(window, cx);
                                    })),
                            )
                        },
                    ),
            )
            .child(
                v_flex()
                    .when(self.history_loading, |this| {
                        this.child(
                            div()
                                .px_5()
                                .py_3()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(crate::tr!("正在读取最近文件…", "Reading recent files…")),
                        )
                    })
                    .when(
                        !self.history_loading && self.recent_files.is_empty(),
                        |this| {
                            this.child(
                                div()
                                    .px_5()
                                    .py_3()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(crate::tr!("暂无最近文件", "No recent files")),
                            )
                        },
                    )
                    .children(self.recent_files.iter().map(|recent| {
                        let path = recent.path.clone();
                        Button::new(("recent-file", recent.id.unsigned_abs()))
                            .small()
                            .ghost()
                            .w_full()
                            .h(rems(EMPTY_WORKSPACE_FILE_ROW_HEIGHT_REMS))
                            .px_5()
                            .rounded(ButtonRounded::None)
                            .child(empty_file_button_content(
                                &recent.path,
                                Some(recent.last_opened_at),
                                cx,
                            ))
                            .disabled(opening)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_recent_file(path.clone(), window, cx);
                            }))
                    })),
            )
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
    }

    fn end_all_row_drag_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.row_drag_selection.is_some() {
            self.advance_row_drag_selection(cx);
        }
        // Result replacement clears the delegate's pointer anchor before MouseUp. Keep the
        // workspace-owned drag target so its text-selection suppression is still released.
        let active_drag = self.row_drag_selection;
        let clear_text_selection = self
            .row_drag_selection
            .is_some_and(|drag| drag.mode == RowDragMode::Lines);
        self.row_drag_selection = None;
        if clear_text_selection {
            TextSelection::clear(window, cx);
        }
        let mut ended_selection = false;
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
        if ended_global_selection {
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

    fn update_wrapped_layout(
        &mut self,
        document_id: u64,
        region: WrappedRegion,
        width: Pixels,
        rem_size: Pixels,
        cx: &mut Context<Self>,
    ) {
        let base_height = self.log_row_height();
        let horizontal_padding = log_cell_horizontal_padding(cx);
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
                let key = {
                    let table = table.read(cx);
                    let delegate = table.delegate();
                    Self::wrapped_layout_key(
                        delegate.content_revision(),
                        width,
                        delegate.log_font_size(),
                        delegate.resolved_font_family(cx),
                        base_height,
                        rem_size,
                        horizontal_padding,
                    )
                };
                let wrapped = if region == WrappedRegion::Results {
                    &mut self.documents[tab_ix].result_viewport
                } else {
                    &mut self.documents[tab_ix].log_viewport
                };
                let preferred = table.read(cx).active_log_row();
                wrapped.invalidate_wrapped_layout_preserving_position(key, preferred)
            }
            WrappedRegion::GlobalResults => {
                if !self.global_viewport.is_wrapped() {
                    return;
                }
                let key = {
                    let table = self.global_table.read(cx);
                    let delegate = table.delegate();
                    Self::wrapped_layout_key(
                        delegate.content_revision(),
                        width,
                        delegate.log_font_size(),
                        delegate.resolved_font_family(cx),
                        base_height,
                        rem_size,
                        horizontal_padding,
                    )
                };
                let preferred = self.global_table.read(cx).active_log_row();
                self.global_viewport
                    .invalidate_wrapped_layout_preserving_position(key, preferred)
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
        table
            .read(cx)
            .delegate()
            .prepare_visible_rows(visible_range.clone());
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
                let severity = row
                    .highlight_severity
                    .then(|| severity_style(row.text.source(), cx))
                    .flatten();
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
                        v_virtual_list(surface, list_id, sizes, move |_, range, window, cx| {
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
                        })
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
    ) -> std::result::Result<(usize, BTreeSet<String>), String> {
        if self.active_log_region == LogRegion::GlobalResults {
            let rows = self.global_table.read(cx).delegate().selected_matches();
            let document_ids = rows
                .iter()
                .map(|(document_id, _)| *document_id)
                .collect::<BTreeSet<_>>();
            let Some(&document_id) = document_ids.first() else {
                return Err(crate::tr!("请先选择日志行", "Select log lines first").to_string());
            };
            if document_ids.len() > 1 {
                return Err(crate::tr!(
                    "颜色标签一次只能应用到同一文件的全局结果",
                    "A color label can be applied only to global results from one file at a time"
                )
                .to_string());
            }
            let active_ix = self.open_document_ix_for_global_result(document_id).ok_or_else(|| {
                if self.global_search.scope == SearchScope::Directory {
                    crate::tr!(
                        "请先打开目录结果所属文件，再应用颜色标签",
                        "Open the file containing the directory result before applying a color label"
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
            let keywords =
                if let Some(text) = selected_text.map(str::trim).filter(|text| !text.is_empty()) {
                    std::iter::once(text.to_string()).collect()
                } else {
                    rows.into_iter()
                        .filter_map(|(_, row)| self.documents[active_ix].document.line(row))
                        .map(|line| line.trim().to_string())
                        .filter(|line| !line.is_empty())
                        .collect()
                };
            return Ok((active_ix, keywords));
        }
        if let Some(text) = selected_text.map(str::trim).filter(|text| !text.is_empty()) {
            let active_ix = self.active_ix.ok_or_else(|| {
                crate::tr!("当前没有活动日志文件", "There is no active log file").to_string()
            })?;
            return Ok((active_ix, std::iter::once(text.to_string()).collect()));
        }
        let active_ix = self.active_ix.ok_or_else(|| {
            crate::tr!("当前没有活动日志文件", "There is no active log file").to_string()
        })?;
        let keywords = self.documents[active_ix]
            .selected_source_rows(cx)
            .into_iter()
            .filter_map(|row| self.documents[active_ix].document.line(row))
            .map(|line| line.trim().to_string())
            .filter(|line| !line.is_empty())
            .collect();
        Ok((active_ix, keywords))
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

    fn apply_context_color_label(
        &mut self,
        label_id: Option<String>,
        selected_text: Option<String>,
        clear_all: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (active_ix, keywords) = match self.context_color_target(selected_text.as_deref(), cx) {
            Ok(target) => target,
            Err(message) => {
                window.push_notification(message, cx);
                return;
            }
        };
        let applying_label = label_id.is_some();
        if clear_all {
            self.documents[active_ix].keyword_color_rules.clear();
        } else if let Some(label_id) = label_id {
            let Some(label) = self
                .color_labels
                .iter()
                .find(|label| label.id == label_id)
                .cloned()
            else {
                window.push_notification(
                    crate::tr!("颜色标签已不存在", "The color label no longer exists"),
                    cx,
                );
                return;
            };
            for keyword in &keywords {
                if let Some(rule) = self.documents[active_ix]
                    .keyword_color_rules
                    .iter_mut()
                    .find(|rule| rule.case_sensitive && rule.keyword == *keyword)
                {
                    rule.label_id = Some(label.id.clone());
                    rule.color = label.color;
                    rule.alpha = label.alpha;
                    rule.enabled = true;
                } else {
                    self.documents[active_ix]
                        .keyword_color_rules
                        .push(KeywordColorRule {
                            label_id: Some(label.id.clone()),
                            keyword: keyword.clone(),
                            color: label.color,
                            alpha: label.alpha,
                            case_sensitive: true,
                            enabled: true,
                        });
                }
            }
            self.last_color_label_id = Some(label.id);
        } else {
            self.documents[active_ix]
                .keyword_color_rules
                .retain(|rule| !(rule.case_sensitive && keywords.contains(rule.keyword.as_str())));
        }
        let resolved = resolve_color_rules(
            &self.documents[active_ix].keyword_color_rules,
            &self.color_labels,
        );
        self.documents[active_ix].resolved_color_rules = resolved.clone();
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
        self.refresh_global_result_rows(cx);
        self.schedule_checkpoint(document_id, window, cx);
        window.push_notification(
            if clear_all {
                crate::tr!(
                    "已清除当前文件的所有颜色",
                    "Cleared all colors from the current file"
                )
            } else if applying_label {
                crate::tr!("已应用颜色标签", "Color label applied")
            } else {
                crate::tr!("已移除颜色标签", "Color label removed")
            },
            cx,
        );
        cx.notify();
    }

    fn context_mark_label(&self, cx: &App) -> &'static str {
        if self.active_log_region == LogRegion::GlobalResults {
            let selected = self.global_table.read(cx).delegate().selected_matches();
            if !selected.is_empty()
                && selected.iter().all(|(document_id, source_row)| {
                    self.documents
                        .iter()
                        .find(|tab| tab.id == *document_id)
                        .is_some_and(|tab| tab.marked_rows.contains(source_row))
                })
            {
                crate::tr!("取消标记", "Unmark")
            } else {
                crate::tr!("标记", "Mark")
            }
        } else if self.active_document().is_some_and(|tab| {
            let rows = tab.selected_source_rows(cx);
            !rows.is_empty() && rows.iter().all(|row| tab.marked_rows.contains(row))
        }) {
            crate::tr!("取消标记", "Unmark")
        } else {
            crate::tr!("标记", "Mark")
        }
    }

    fn context_color_label_id(&self, selected_text: Option<&str>, cx: &App) -> Option<String> {
        let (tab_ix, keywords) = self.context_color_target(selected_text, cx).ok()?;
        if keywords.is_empty() {
            return None;
        }
        let mut current: Option<Option<String>> = None;
        for keyword in keywords {
            let label_id = self.documents[tab_ix]
                .keyword_color_rules
                .iter()
                .find(|rule| rule.enabled && rule.case_sensitive && rule.keyword == keyword)
                .and_then(|rule| rule.label_id.clone());
            match &current {
                None => current = Some(label_id),
                Some(existing) if *existing == label_id => {}
                Some(_) => return None,
            }
        }
        current.flatten()
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
                            .checked(current_label_id.is_none())
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
        Some(ViewportBookmark::new(
            source_row,
            position.viewport_y.as_f32(),
            viewport.horizontal_offset().as_f32(),
            viewport.is_at_end(),
        ))
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
        if viewport.is_wrapped() {
            viewport.wrapped_sizes(table.read(cx).delegate().row_count(), row_height);
        }
        let fallback_ix = {
            let table = table.read(cx);
            match region {
                WrappedRegion::Results => (0..table.delegate().row_count())
                    .find(|ix| table.delegate().source_row(*ix) == Some(bookmark.anchor_source_row))
                    .unwrap_or_default(),
                _ => tab
                    .document
                    .local_row(bookmark.anchor_source_row)
                    .unwrap_or_default(),
            }
        };
        Self::restore_local_viewport_anchor(
            tab,
            region,
            Some(ViewportAnchor {
                key: LogRowKey::Row {
                    document_id: tab.id,
                    source_row: bookmark.anchor_source_row,
                },
                viewport_y: px(bookmark.anchor_viewport_y()),
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
        let source_row = match anchor.key {
            LogRowKey::Row { source_row, .. } => source_row,
            LogRowKey::FileGroup { .. } => return,
        };
        let row_ix = {
            let table = table.read(cx);
            match region {
                WrappedRegion::Results => (0..table.delegate().row_count())
                    .find(|ix| table.delegate().source_row(*ix) == Some(source_row)),
                _ => tab.document.local_row(source_row),
            }
            .unwrap_or_else(|| {
                anchor
                    .fallback_ix
                    .min(table.delegate().row_count().saturating_sub(1))
            })
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
        let row = match anchor.key {
            LogRowKey::Row {
                document_id,
                source_row,
            } => GlobalSearchRow::Match {
                document_id,
                source_row,
            },
            LogRowKey::FileGroup { document_id } => GlobalSearchRow::Group { document_id },
        };
        let row_ix = self
            .global_table
            .read(cx)
            .delegate()
            .row_ix(row)
            .or_else(|| match row {
                GlobalSearchRow::Match { document_id, .. } => self
                    .global_table
                    .read(cx)
                    .delegate()
                    .row_ix(GlobalSearchRow::Group { document_id }),
                _ => None,
            })
            .unwrap_or_else(|| {
                anchor.fallback_ix.min(
                    self.global_table
                        .read(cx)
                        .delegate()
                        .rows_len()
                        .saturating_sub(1),
                )
            });
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
        self.global_table
            .read(cx)
            .delegate()
            .prepare_visible_rows(visible_range.clone());
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
                        let severity = highlight_severity
                            .then(|| severity_style(text.source(), cx))
                            .flatten();
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
                        v_virtual_list(
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
        row_height: Pixels,
        cx: &mut Context<Self>,
    ) -> AnyElement
    where
        D: TableDelegate,
    {
        let _performance_scope =
            crate::ui_performance::scope("Workspace::render_headerless_data_table");
        let (vertical_scroll_handle, horizontal_scroll_handle, horizontal_content_width) = {
            let table = table.read(cx);
            let delegate = table.delegate();
            let horizontal_content_width = (0..delegate.columns_count(cx))
                .map(|col_ix| delegate.column(col_ix, cx).width)
                .fold(px(0.), |width, column_width| width + column_width);
            (
                table.vertical_scroll_handle.clone(),
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
            if self.global_viewport.is_wrapped() {
                if self
                    .global_viewport
                    .take_wrapped_scrollbar_measurement_request()
                {
                    self.prime_global_wrapped_frame(row_height, false, window, cx);
                }
                return self.render_wrapped_global_table(surface, cx.weak_entity(), cx);
            }
            return Self::render_headerless_data_table(&self.global_table, row_height, cx);
        }
        let Some(tab_ix) = self.documents.iter().position(|tab| tab.id == document_id) else {
            return div().into_any_element();
        };
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
            let measurement_pending = if region == WrappedRegion::Results {
                tab.result_viewport
                    .take_wrapped_scrollbar_measurement_request()
            } else {
                tab.log_viewport
                    .take_wrapped_scrollbar_measurement_request()
            };
            if measurement_pending {
                self.prime_local_wrapped_frame(tab_ix, region, row_height, false, window, cx);
            }
            self.render_wrapped_log_table(document_id, region, surface, cx.weak_entity(), cx)
        } else {
            Self::render_headerless_data_table(table, row_height, cx)
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
                                    window.rem_size(),
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
                                    window.rem_size(),
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
                                            window.rem_size(),
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
            .unwrap_or_else(|| format!("core {}", vclogg_core::CORE_VERSION));

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

fn prepare_paths_bounded<T, F>(paths: Vec<PathBuf>, operation: F) -> Vec<(PathBuf, T)>
where
    T: Send,
    F: Fn(&std::path::Path) -> T + Sync,
{
    if paths.len() <= 1 {
        return paths
            .into_iter()
            .map(|path| {
                let result = operation(&path);
                (path, result)
            })
            .collect();
    }
    let worker_count = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
        .min(MAX_DOCUMENT_PREPARE_WORKERS)
        .min(paths.len());
    let mut worker_paths = (0..worker_count).map(|_| Vec::new()).collect::<Vec<_>>();
    for (path_ix, path) in paths.into_iter().enumerate() {
        worker_paths[path_ix % worker_count].push((path_ix, path));
    }
    std::thread::scope(|scope| {
        let operation = &operation;
        let handles = worker_paths
            .into_iter()
            .map(|worker_paths| {
                scope.spawn(move || {
                    worker_paths
                        .into_iter()
                        .map(|(path_ix, path)| {
                            let result = operation(&path);
                            (path_ix, path, result)
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect::<Vec<_>>();
        let mut prepared = Vec::new();
        for handle in handles {
            match handle.join() {
                Ok(worker_prepared) => prepared.extend(worker_prepared),
                Err(panic) => std::panic::resume_unwind(panic),
            }
        }
        prepared.sort_by_key(|(path_ix, _, _)| *path_ix);
        prepared
            .into_iter()
            .map(|(_, path, result)| (path, result))
            .collect()
    })
}

fn prepare_document(
    path: &std::path::Path,
    store: Option<&StateStore>,
    session_override: Option<FileSessionState>,
    search_result_limit: Option<usize>,
) -> Result<PreparedDocument> {
    let (document, pending_index_cache) = if let Some(cache_root) = crate::app_paths::cache_dir() {
        LogDocument::open_with_index_cache(path, cache_root.join("VCLogg2").join("index"))?
    } else {
        (LogDocument::open(path)?, None)
    };
    let document = Arc::new(document);
    let mut warning = None;
    let session = if session_override.is_some() {
        session_override
    } else {
        match store.map(|store| store.load_session(path)).transpose() {
            Ok(session) => session.flatten(),
            Err(error) => {
                warning = Some(crate::tr_args!(
                    "{} 的会话未能读取，将使用默认视图：{error}",
                    "The session for {} couldn’t be read; the default view will be used: {error}",
                    path.display(),
                ));
                None
            }
        }
    };
    let query = session
        .as_ref()
        .map(|state| SearchQuery {
            text: state.query_text.clone(),
            case_sensitive: state.case_sensitive,
            regex: state.regex,
            max_results: search_result_limit,
        })
        .unwrap_or_default();
    let search = (|| {
        let matcher = SearchMatcher::new(&query)?;
        let result = if query.text.is_empty() {
            SearchResult::default()
        } else {
            search_document_with_matcher(&document, &query, matcher.as_ref())
        };
        Ok::<_, anyhow::Error>((result, matcher))
    })();
    let (search_result, search_matcher) = match search {
        Ok(search) => search,
        Err(error) => {
            warning = Some(crate::tr_args!(
                "{} 的已保存查询未能恢复：{error}",
                "The saved query for {} couldn’t be restored: {error}",
                path.display()
            ));
            (SearchResult::default(), None)
        }
    };

    Ok(PreparedDocument {
        document,
        session,
        search_result,
        search_matcher,
        warning,
        load_state: DocumentLoadState::Ready,
        pending_index_cache,
    })
}

fn prepare_document_shell(
    path: &std::path::Path,
    session: Option<FileSessionState>,
) -> PreparedDocument {
    PreparedDocument {
        document: Arc::new(LogDocument::placeholder(path)),
        session,
        search_result: SearchResult::default(),
        search_matcher: None,
        warning: None,
        load_state: DocumentLoadState::Opening,
        pending_index_cache: None,
    }
}

fn search_document_with_matcher(
    document: &LogDocument,
    query: &SearchQuery,
    matcher: Option<&SearchMatcher>,
) -> SearchResult {
    let cancellation = SearchCancellation::default();
    let progress = SearchProgress::new(document.line_count());
    match search_with_compiled_matcher(
        document,
        matcher,
        query.max_results,
        &cancellation,
        &progress,
    ) {
        SearchRun::Completed(result) => result,
        SearchRun::Cancelled => SearchResult::default(),
    }
}

fn prepare_document_preview(
    path: &std::path::Path,
    store: Option<&StateStore>,
    session_override: Option<FileSessionState>,
    search_result_limit: Option<usize>,
) -> Result<PreparedDocument> {
    let mut warning = None;
    let session = if session_override.is_some() {
        session_override
    } else {
        match store.map(|store| store.load_session(path)).transpose() {
            Ok(session) => session.flatten(),
            Err(error) => {
                warning = Some(crate::tr_args!(
                    "{} 的会话未能读取，将使用默认视图：{error}",
                    "The session for {} couldn’t be read; the default view will be used: {error}",
                    path.display(),
                ));
                None
            }
        }
    };
    let preferred_row = session.as_ref().and_then(|session| {
        session
            .resume
            .viewer
            .viewport
            .as_ref()
            .map(|viewport| viewport.anchor_source_row)
            .or(session.selected_row)
    });
    let cached_preview = match (preferred_row, crate::app_paths::cache_dir()) {
        (Some(preferred_row), Some(cache_root)) if preferred_row > 0 => {
            LogDocument::open_cached_preview(
                path,
                cache_root.join("VCLogg2").join("index"),
                preferred_row,
                PREVIEW_LINE_LIMIT,
            )?
        }
        _ => None,
    };
    let document = match cached_preview {
        Some(document) => document,
        None => LogDocument::open_preview(path, PREVIEW_BYTE_LIMIT, PREVIEW_LINE_LIMIT)?.0,
    };
    let document = Arc::new(document);
    let query = session
        .as_ref()
        .map(|state| SearchQuery {
            text: state.query_text.clone(),
            case_sensitive: state.case_sensitive,
            regex: state.regex,
            max_results: search_result_limit,
        })
        .unwrap_or_default();
    let search_matcher = match SearchMatcher::new(&query) {
        Ok(matcher) => matcher,
        Err(error) => {
            warning = Some(crate::tr_args!(
                "{} 的已保存查询无效：{error}",
                "The saved query for {} is invalid: {error}",
                path.display()
            ));
            None
        }
    };
    Ok(PreparedDocument {
        document,
        session,
        search_result: SearchResult::default(),
        search_matcher,
        warning,
        load_state: DocumentLoadState::Preview,
        pending_index_cache: None,
    })
}

fn compute_result_rows(
    mode: ResultMode,
    search_result: Option<&SearchResult>,
    marked_rows: &BTreeSet<usize>,
) -> CompressedRows {
    let matched_rows = search_result
        .map(|result| result.line_indices.clone())
        .unwrap_or_default();
    match mode {
        ResultMode::MatchesOnly => matched_rows,
        ResultMode::MarksOnly => marked_rows.iter().copied().collect(),
        ResultMode::MatchesAndMarks => matched_rows.union(marked_rows.iter().copied()),
    }
}

#[cfg(not(windows))]
type PathMatchKey = PathBuf;
#[cfg(windows)]
type PathMatchKey = String;

#[cfg(not(windows))]
fn path_match_key(path: &Path) -> PathMatchKey {
    path.to_path_buf()
}

#[cfg(windows)]
fn path_match_key(path: &Path) -> PathMatchKey {
    path.to_string_lossy().to_ascii_lowercase()
}

#[cfg(not(windows))]
fn path_match_set_contains(paths: &BTreeSet<PathMatchKey>, path: &Path) -> bool {
    paths.contains(path)
}

#[cfg(windows)]
fn path_match_set_contains(paths: &BTreeSet<PathMatchKey>, path: &Path) -> bool {
    paths.contains(&path_match_key(path))
}

#[cfg(not(windows))]
fn path_match_map_get<'a, V>(paths: &'a BTreeMap<PathMatchKey, V>, path: &Path) -> Option<&'a V> {
    paths.get(path)
}

#[cfg(windows)]
fn path_match_map_get<'a, V>(paths: &'a BTreeMap<PathMatchKey, V>, path: &Path) -> Option<&'a V> {
    paths.get(&path_match_key(path))
}

fn paths_match(left: &Path, right: &Path) -> bool {
    #[cfg(not(windows))]
    {
        left == right
    }
    #[cfg(windows)]
    {
        path_match_key(left) == path_match_key(right)
    }
}

#[cfg(test)]
mod path_match_tests {
    use super::*;

    #[test]
    fn path_indexes_use_the_same_identity_as_direct_matching() {
        let stored = Path::new("logs/a.log");
        let key = path_match_key(stored);
        let set = BTreeSet::from([key.clone()]);
        let map = BTreeMap::from([(key, 7)]);

        assert!(path_match_set_contains(&set, stored));
        assert_eq!(path_match_map_get(&map, stored), Some(&7));
        assert!(paths_match(stored, stored));

        let differently_cased = Path::new("LOGS/A.LOG");
        assert_eq!(
            path_match_set_contains(&set, differently_cased),
            cfg!(windows)
        );
        assert_eq!(paths_match(stored, differently_cased), cfg!(windows));
    }
}

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
mod document_prepare_performance_tests {
    use std::{
        fs::{self, File},
        hint::black_box,
        io::{BufWriter, Write as _},
        path::PathBuf,
        time::Instant,
    };

    use vclogg_core::LogDocument;

    use super::prepare_paths_bounded;

    struct TemporaryDirectory(PathBuf);

    impl TemporaryDirectory {
        fn new(label: &str) -> Self {
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .expect("系统时间应晚于 Unix 纪元")
                .as_nanos();
            let path = std::env::temp_dir()
                .join(format!("vclogg2-{label}-{}-{nonce}", std::process::id()));
            fs::create_dir_all(&path).expect("应能创建临时文件准备目录");
            Self(path)
        }
    }

    impl Drop for TemporaryDirectory {
        fn drop(&mut self) {
            _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn bounded_path_mapping_preserves_input_order() {
        let paths = (0..12)
            .map(|index| PathBuf::from(format!("file-{index:02}.log")))
            .collect::<Vec<_>>();

        let prepared = prepare_paths_bounded(paths.clone(), |path| path.to_path_buf());

        assert_eq!(
            prepared.into_iter().collect::<Vec<_>>(),
            paths
                .into_iter()
                .map(|path| (path.clone(), path))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    #[ignore = "手动性能基准：cargo test -p vclogg2 --release benchmark_parallel_document_prepare -- --ignored --nocapture"]
    fn benchmark_parallel_document_prepare() {
        const FILE_COUNT: usize = 8;
        const FILE_BYTES: usize = 16 * 1024 * 1024;
        let temporary = TemporaryDirectory::new("parallel-document-prepare");
        let line = b"2026-08-27 INFO parallel document preparation line\n";
        let paths = (0..FILE_COUNT)
            .map(|index| {
                let path = temporary.0.join(format!("file-{index:02}.log"));
                let mut writer = BufWriter::new(File::create(&path).expect("应能创建性能测试日志"));
                for _ in 0..FILE_BYTES.div_ceil(line.len()) {
                    writer.write_all(line).expect("应能写入性能测试日志");
                }
                writer.flush().expect("应能刷新性能测试日志");
                path
            })
            .collect::<Vec<_>>();

        let sequential_started = Instant::now();
        let sequential = paths
            .iter()
            .map(|path| LogDocument::open(black_box(path)).expect("串行文件准备应成功"))
            .collect::<Vec<_>>();
        let sequential_elapsed = sequential_started.elapsed();
        black_box(sequential);

        let parallel_started = Instant::now();
        let parallel = prepare_paths_bounded(paths, |path| {
            LogDocument::open(black_box(path)).expect("并行文件准备应成功")
        });
        let parallel_elapsed = parallel_started.elapsed();
        black_box(parallel);

        eprintln!(
            "准备 {FILE_COUNT} 个 16 MiB 文件：串行 {sequential_elapsed:?}；最多 4 路并行 {parallel_elapsed:?}"
        );
    }
}
