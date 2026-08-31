use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::Arc,
};

use gpui::{Entity, Pixels, SharedString, Task};
use gpui_component::{
    input::InputState,
    select::{SelectItem, SelectState},
};
use vclogg_core::{
    CompressedRows, LogDocument, SearchCancellation, SearchMatcher, SearchQuery, SearchResult,
};

use crate::{
    cloud_filters::{CloudClient, CloudConnectionProfile},
    directory_search_dialog::DirectorySearchOptions,
    global_search_table::{GlobalQuickFindGroup, GlobalSearchRow},
    search_context::{PersistedGlobalSearchContext, WorkspaceSearchState},
    state_store::{CloudSettings, FileSessionState, StateStore},
    updater::{AvailableUpdate, DownloadedUpdate, UpdateClient},
    virtual_log_lines::LogRowKey,
};

#[derive(Clone, Copy, Debug)]
pub(crate) struct ViewportAnchor<K> {
    pub(crate) key: K,
    pub(crate) viewport_y: Pixels,
    pub(crate) at_end: bool,
    pub(crate) fallback_ix: usize,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct RowViewportAnchor<K> {
    pub(crate) key: K,
    pub(crate) viewport_y: Pixels,
    pub(crate) fallback_ix: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum ResultMode {
    #[default]
    MatchesAndMarks,
    MatchesOnly,
    MarksOnly,
}

impl ResultMode {
    pub(crate) const ALL: [Self; 3] = [Self::MatchesAndMarks, Self::MatchesOnly, Self::MarksOnly];

    pub(crate) fn includes_matches(self) -> bool {
        matches!(self, Self::MatchesAndMarks | Self::MatchesOnly)
    }

    pub(crate) fn includes_marks(self) -> bool {
        matches!(self, Self::MatchesAndMarks | Self::MarksOnly)
    }

    pub(crate) fn from_database(value: i64) -> Self {
        match value {
            1 => Self::MatchesOnly,
            2 => Self::MarksOnly,
            _ => Self::MatchesAndMarks,
        }
    }

    pub(crate) fn database_value(self) -> i64 {
        match self {
            Self::MatchesAndMarks => 0,
            Self::MatchesOnly => 1,
            Self::MarksOnly => 2,
        }
    }

    pub(crate) fn select_index(self) -> usize {
        usize::try_from(self.database_value()).unwrap_or_default()
    }
}

impl SelectItem for ResultMode {
    type Value = Self;

    fn title(&self) -> SharedString {
        match self {
            Self::MatchesAndMarks => crate::tr!("标记与匹配", "Marks & matches").into(),
            Self::MatchesOnly => crate::tr!("仅匹配", "Matches only").into(),
            Self::MarksOnly => crate::tr!("仅标记", "Marks only").into(),
        }
    }

    fn value(&self) -> &Self::Value {
        self
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum SearchScope {
    #[default]
    CurrentFile,
    AllOpenFiles,
    Directory,
}

impl SearchScope {
    pub(crate) fn owns_global_word_wrap(self) -> bool {
        matches!(self, Self::AllOpenFiles | Self::Directory)
    }
}

#[derive(Clone)]
pub(crate) struct GlobalSearchDocumentResult {
    pub(crate) document_id: u64,
    pub(crate) title: SharedString,
    pub(crate) path: PathBuf,
    pub(crate) document: Arc<LogDocument>,
    pub(crate) search_result: SearchResult,
    pub(crate) failure: Option<SharedString>,
}

#[derive(Clone)]
pub(crate) struct RetainedGlobalSearchContext {
    pub(crate) initialized: bool,
    pub(crate) results: Vec<GlobalSearchDocumentResult>,
    pub(crate) matcher: Option<SearchMatcher>,
    pub(crate) result_mode: ResultMode,
    pub(crate) results_visible: bool,
    pub(crate) collapsed_document_ids: BTreeSet<u64>,
    pub(crate) selection: BTreeMap<u64, CompressedRows>,
    pub(crate) selected_row: Option<GlobalSearchRow>,
    pub(crate) viewport: Option<ViewportAnchor<LogRowKey>>,
    pub(crate) horizontal_offset: f32,
    pub(crate) word_wrap: bool,
    pub(crate) active: bool,
}

impl Default for RetainedGlobalSearchContext {
    fn default() -> Self {
        Self {
            initialized: false,
            results: Vec::new(),
            matcher: None,
            result_mode: ResultMode::MatchesAndMarks,
            results_visible: false,
            collapsed_document_ids: BTreeSet::new(),
            selection: BTreeMap::new(),
            selected_row: None,
            viewport: None,
            horizontal_offset: 0.,
            word_wrap: false,
            active: false,
        }
    }
}

#[derive(Clone)]
pub(crate) enum AppUpdateState {
    Unsupported,
    Idle,
    Checking,
    Current,
    Available(Box<AvailableUpdate>),
    Downloading { version: String },
    Downloaded(DownloadedUpdate),
    Error,
}

impl AppUpdateState {
    fn button_label(&self, transferred: u64, total: u64) -> String {
        match self {
            Self::Unsupported => {
                crate::tr!("更新（开发版）", "Updates (development build)").to_string()
            }
            Self::Idle => crate::tr!("检查更新", "Check for updates").to_string(),
            Self::Checking => crate::tr!("检查更新中…", "Checking for updates…").to_string(),
            Self::Current => crate::tr!("已是最新版", "Up to date").to_string(),
            Self::Available(update) => {
                crate::tr_args!("下载 {}", "Download {}", update.manifest.version)
            }
            Self::Downloading { version } if total > 0 => crate::tr_args!(
                "下载 {version} · {:.0}%",
                "Downloading {version} · {:.0}%",
                transferred as f64 / total as f64 * 100.,
            ),
            Self::Downloading { version } => {
                crate::tr_args!("下载 {version}…", "Downloading {version}…")
            }
            Self::Downloaded(update) => crate::tr_args!("安装 {}", "Install {}", update.version),
            Self::Error => crate::tr!("重试更新", "Retry update").to_string(),
        }
    }
}

pub(crate) struct UpdateController {
    pub(crate) client: Option<UpdateClient>,
    pub(crate) state: AppUpdateState,
    pub(crate) transferred: u64,
    pub(crate) total: u64,
    pub(crate) task: Option<Task<()>>,
    pub(crate) progress_task: Option<Task<()>>,
}

impl UpdateController {
    pub(crate) fn new(state: AppUpdateState) -> Self {
        Self {
            client: None,
            state,
            transferred: 0,
            total: 0,
            task: None,
            progress_task: None,
        }
    }

    pub(crate) fn button_label(&self) -> String {
        self.state.button_label(self.transferred, self.total)
    }

    pub(crate) fn is_busy(&self) -> bool {
        matches!(
            self.state,
            AppUpdateState::Checking | AppUpdateState::Downloading { .. }
        )
    }
}

#[derive(Default)]
pub(crate) struct CloudController {
    pub(crate) settings: CloudSettings,
    pub(crate) client: Option<CloudClient>,
    pub(crate) connection: Option<CloudConnectionProfile>,
    pub(crate) client_error: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum QuickFindTarget {
    Log(u64),
    Results(u64),
    GlobalResults,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum QuickFindDirection {
    Forward,
    Backward,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum QuickFindBoundary {
    Start,
    End,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct QuickFindMatch {
    pub(crate) target: QuickFindTarget,
    pub(crate) view_row: usize,
    pub(crate) source_row: usize,
}

pub(crate) enum QuickFindSource {
    Document {
        document: Arc<LogDocument>,
        rows: Option<CompressedRows>,
        row_count: usize,
    },
    Global(Vec<GlobalQuickFindGroup>),
}

pub(crate) struct QuickFindState {
    pub(crate) query: Entity<InputState>,
    pub(crate) open: bool,
    pub(crate) target: Option<QuickFindTarget>,
    pub(crate) anchor: usize,
    pub(crate) matched: Option<QuickFindMatch>,
    pub(crate) matcher: Option<SearchMatcher>,
    pub(crate) case_sensitive: bool,
    pub(crate) whole_word: bool,
    pub(crate) regex: bool,
    pub(crate) error: Option<SharedString>,
    pub(crate) no_match: bool,
    pub(crate) boundary: Option<QuickFindBoundary>,
    pub(crate) busy: bool,
    pub(crate) direction: Option<QuickFindDirection>,
    pub(crate) revision: u64,
    pub(crate) cancellation: Option<SearchCancellation>,
    pub(crate) task: Option<Task<()>>,
}

impl QuickFindState {
    pub(crate) fn new(query: Entity<InputState>) -> Self {
        Self {
            query,
            open: false,
            target: None,
            anchor: 0,
            matched: None,
            matcher: None,
            case_sensitive: false,
            whole_word: false,
            regex: false,
            error: None,
            no_match: false,
            boundary: None,
            busy: false,
            direction: None,
            revision: 0,
            cancellation: None,
            task: None,
        }
    }

    pub(crate) fn cancel_work(&mut self) {
        self.revision = self.revision.saturating_add(1);
        if let Some(cancellation) = self.cancellation.take() {
            cancellation.cancel();
        }
        self.task = None;
        self.busy = false;
        self.direction = None;
    }

    pub(crate) fn open(&mut self, target: QuickFindTarget, anchor: usize) {
        self.cancel_work();
        self.open = true;
        self.target = Some(target);
        self.anchor = anchor;
        self.matched = None;
        self.no_match = false;
        self.boundary = None;
    }

    pub(crate) fn close(&mut self) -> Option<QuickFindTarget> {
        let target = self.target;
        self.cancel_work();
        self.open = false;
        self.target = None;
        self.matched = None;
        self.matcher = None;
        self.error = None;
        self.no_match = false;
        self.boundary = None;
        target
    }
}

pub(crate) struct GlobalSearchState {
    pub(crate) scope: SearchScope,
    pub(crate) query: SearchQuery,
    pub(crate) directory_query: SearchQuery,
    pub(crate) directory_options: DirectorySearchOptions,
    pub(crate) result_mode: ResultMode,
    pub(crate) result_mode_select: Entity<SelectState<Vec<ResultMode>>>,
    pub(crate) selected_documents: std::collections::BTreeSet<u64>,
    pub(crate) preferences: BTreeMap<PathBuf, bool>,
    pub(crate) results: Vec<GlobalSearchDocumentResult>,
    pub(crate) matcher: Option<SearchMatcher>,
    pub(crate) result_scope: Option<SearchScope>,
    pub(crate) all_open_context: RetainedGlobalSearchContext,
    pub(crate) directory_context: RetainedGlobalSearchContext,
    pub(crate) pending_all_open_restore: Option<PersistedGlobalSearchContext>,
    pub(crate) pending_directory_restore: Option<PersistedGlobalSearchContext>,
    pub(crate) restoring_selection: bool,
    pub(crate) revision: u64,
    pub(crate) results_visible: bool,
}

impl GlobalSearchState {
    pub(crate) fn new(result_mode_select: Entity<SelectState<Vec<ResultMode>>>) -> Self {
        Self {
            scope: SearchScope::CurrentFile,
            query: SearchQuery::default(),
            directory_query: SearchQuery::default(),
            directory_options: DirectorySearchOptions::default(),
            result_mode: ResultMode::MatchesAndMarks,
            result_mode_select,
            selected_documents: Default::default(),
            preferences: BTreeMap::new(),
            results: Vec::new(),
            matcher: None,
            result_scope: None,
            all_open_context: RetainedGlobalSearchContext::default(),
            directory_context: RetainedGlobalSearchContext::default(),
            pending_all_open_restore: None,
            pending_directory_restore: None,
            restoring_selection: false,
            revision: 0,
            results_visible: false,
        }
    }
}

pub(crate) struct PersistenceController {
    pub(crate) state_tasks: Vec<Task<()>>,
    pub(crate) checkpoint_task: Option<Task<()>>,
    pub(crate) workspace_order_task: Option<Task<()>>,
    pub(crate) search_history_save_task: Option<Task<()>>,
    pub(crate) app_settings_save_task: Option<Task<()>>,
    pub(crate) appearance_save_task: Option<Task<()>>,
    pub(crate) settings_category_save_task: Option<Task<()>>,
    pub(crate) search_panel_height_save_task: Option<Task<()>>,
    pub(crate) search_context_save_task: Option<Task<()>>,
    pub(crate) pending_workspace_search_save: Option<WorkspaceSearchState>,
    pub(crate) store: Option<Arc<StateStore>>,
    pub(crate) pending_sessions: Vec<(PathBuf, FileSessionState, FileSessionState)>,
    pub(crate) pending_session_overrides: BTreeMap<PathBuf, FileSessionState>,
    pub(crate) last_saved_sessions: BTreeMap<PathBuf, FileSessionState>,
    pub(crate) session_save_task: Option<Task<()>>,
    pub(crate) _bootstrap_task: Task<()>,
}

impl PersistenceController {
    pub(crate) fn new(bootstrap_task: Task<()>) -> Self {
        Self {
            state_tasks: Vec::new(),
            checkpoint_task: None,
            workspace_order_task: None,
            search_history_save_task: None,
            app_settings_save_task: None,
            appearance_save_task: None,
            settings_category_save_task: None,
            search_panel_height_save_task: None,
            search_context_save_task: None,
            pending_workspace_search_save: None,
            store: None,
            pending_sessions: Vec::new(),
            pending_session_overrides: BTreeMap::new(),
            last_saved_sessions: BTreeMap::new(),
            session_save_task: None,
            _bootstrap_task: bootstrap_task,
        }
    }
}
