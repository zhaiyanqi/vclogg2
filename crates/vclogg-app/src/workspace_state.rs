use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
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
    path_identity::{PathMatchKey, path_match_key},
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
    pub(crate) title: SharedString,
    pub(crate) path: PathBuf,
    pub(crate) document: Arc<LogDocument>,
    pub(crate) search_result: SearchResult,
    pub(crate) failure: Option<SharedString>,
}

/// Search-result snapshot with stable identity kept separate from presentation order.
#[derive(Clone, Default)]
pub(crate) struct GlobalSearchResults {
    order: Arc<Vec<u64>>,
    by_document: Arc<BTreeMap<u64, GlobalSearchDocumentResult>>,
}

impl GlobalSearchResults {
    pub(crate) fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    pub(crate) fn get(&self, document_id: &u64) -> Option<&GlobalSearchDocumentResult> {
        self.by_document.get(document_id)
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (&u64, &GlobalSearchDocumentResult)> {
        self.order.iter().map(|document_id| {
            let result = self
                .by_document
                .get(document_id)
                .expect("global result order and identity index must stay in sync");
            (document_id, result)
        })
    }

    pub(crate) fn values(&self) -> impl Iterator<Item = &GlobalSearchDocumentResult> {
        self.iter().map(|(_, result)| result)
    }

    pub(crate) fn remove_documents(&mut self, document_ids: &BTreeSet<u64>) {
        if !document_ids
            .iter()
            .any(|document_id| self.by_document.contains_key(document_id))
        {
            return;
        }
        Arc::make_mut(&mut self.by_document)
            .retain(|document_id, _| !document_ids.contains(document_id));
        let by_document = &self.by_document;
        Arc::make_mut(&mut self.order).retain(|document_id| by_document.contains_key(document_id));
    }

    pub(crate) fn clear(&mut self) {
        self.order = Arc::default();
        self.by_document = Arc::default();
    }
}

impl FromIterator<(u64, GlobalSearchDocumentResult)> for GlobalSearchResults {
    fn from_iter<T: IntoIterator<Item = (u64, GlobalSearchDocumentResult)>>(iter: T) -> Self {
        let mut order = Vec::new();
        let mut by_document = BTreeMap::new();
        for (document_id, result) in iter {
            let previous = by_document.insert(document_id, result);
            if previous.is_none() {
                order.push(document_id);
            }
            debug_assert!(
                previous.is_none(),
                "duplicate global search document identity"
            );
        }
        Self {
            order: Arc::new(order),
            by_document: Arc::new(by_document),
        }
    }
}

#[derive(Clone)]
pub(crate) struct RetainedGlobalSearchContext {
    pub(crate) initialized: bool,
    pub(crate) results: GlobalSearchResults,
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
            results: GlobalSearchResults::default(),
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

impl RetainedGlobalSearchContext {
    pub(crate) fn invalidate_results(&mut self) {
        self.initialized = false;
        self.results.clear();
        self.matcher = None;
        self.results_visible = false;
        self.collapsed_document_ids.clear();
        self.selection.clear();
        self.selected_row = None;
        self.viewport = None;
        self.horizontal_offset = 0.;
        self.active = false;
    }

    pub(crate) fn remove_documents(&mut self, document_ids: &BTreeSet<u64>) {
        self.results.remove_documents(document_ids);
        self.collapsed_document_ids
            .retain(|document_id| !document_ids.contains(document_id));
        self.selection
            .retain(|document_id, _| !document_ids.contains(document_id));
        if self.selected_row.is_some_and(|row| {
            let document_id = match row {
                GlobalSearchRow::Group { document_id }
                | GlobalSearchRow::Match { document_id, .. } => document_id,
            };
            document_ids.contains(&document_id)
        }) {
            self.selected_row = None;
        }
        if self.viewport.as_ref().is_some_and(|viewport| {
            let document_id = match viewport.key {
                LogRowKey::Row { document_id, .. } | LogRowKey::FileGroup { document_id } => {
                    document_id
                }
            };
            document_ids.contains(&document_id)
        }) {
            self.viewport = None;
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

pub(crate) enum QuickFindSourceVersion {
    Document {
        document: Arc<LogDocument>,
        rows: Option<CompressedRows>,
    },
    Global {
        content_revision: u64,
        layout_revision: u64,
    },
}

impl QuickFindSourceVersion {
    pub(crate) fn is_same_as(&self, other: &Self) -> bool {
        match (self, other) {
            (
                Self::Document {
                    document: left_document,
                    rows: left_rows,
                },
                Self::Document {
                    document: right_document,
                    rows: right_rows,
                },
            ) => Arc::ptr_eq(left_document, right_document) && left_rows == right_rows,
            (
                Self::Global {
                    content_revision: left_content,
                    layout_revision: left_layout,
                },
                Self::Global {
                    content_revision: right_content,
                    layout_revision: right_layout,
                },
            ) => left_content == right_content && left_layout == right_layout,
            (Self::Document { .. }, Self::Global { .. })
            | (Self::Global { .. }, Self::Document { .. }) => false,
        }
    }
}

pub(crate) struct QuickFindState {
    pub(crate) query: Entity<InputState>,
    pub(crate) open: bool,
    pub(crate) target: Option<QuickFindTarget>,
    pub(crate) anchor: usize,
    pub(crate) matched: Option<QuickFindMatch>,
    pub(crate) matched_source_version: Option<QuickFindSourceVersion>,
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
            matched_source_version: None,
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
        self.clear_match();
        self.no_match = false;
        self.boundary = None;
    }

    pub(crate) fn close(&mut self) -> Option<QuickFindTarget> {
        let target = self.target;
        self.cancel_work();
        self.open = false;
        self.target = None;
        self.clear_match();
        self.matcher = None;
        self.error = None;
        self.no_match = false;
        self.boundary = None;
        target
    }

    pub(crate) fn clear_match(&mut self) {
        self.matched = None;
        self.matched_source_version = None;
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
    preferences: PathPreferences,
    pub(crate) results: GlobalSearchResults,
    pub(crate) matcher: Option<SearchMatcher>,
    pub(crate) result_scope: Option<SearchScope>,
    pub(crate) all_open_context: RetainedGlobalSearchContext,
    pub(crate) directory_context: RetainedGlobalSearchContext,
    pub(crate) pending_all_open_restore: Option<PersistedGlobalSearchContext>,
    pub(crate) pending_directory_restore: Option<PersistedGlobalSearchContext>,
    pub(crate) restoring_selection: bool,
    pub(crate) revision: u64,
    pub(crate) results_visible: bool,
    directory_document_ids: DirectoryDocumentIds,
}

const DIRECTORY_DOCUMENT_ID_BASE: u64 = 1 << 63;

#[derive(Default)]
struct PathPreferences {
    by_path: BTreeMap<PathMatchKey, (PathBuf, bool)>,
}

impl PathPreferences {
    fn replace(&mut self, preferences: BTreeMap<PathBuf, bool>) {
        self.by_path = preferences
            .into_iter()
            .map(|(path, selected)| (path_match_key(&path), (path, selected)))
            .collect();
    }

    fn get(&self, path: &Path) -> Option<bool> {
        self.by_path
            .get(&path_match_key(path))
            .map(|(_, selected)| *selected)
    }

    fn insert(&mut self, path: PathBuf, selected: bool) {
        self.by_path.insert(path_match_key(&path), (path, selected));
    }
}

struct DirectoryDocumentIds {
    by_path: BTreeMap<PathMatchKey, u64>,
    next: u64,
}

impl Default for DirectoryDocumentIds {
    fn default() -> Self {
        Self {
            by_path: BTreeMap::new(),
            next: DIRECTORY_DOCUMENT_ID_BASE,
        }
    }
}

impl DirectoryDocumentIds {
    fn id_for_path(&mut self, path: &Path) -> u64 {
        let key = path_match_key(path);
        if let Some(id) = self.by_path.get(&key) {
            return *id;
        }
        let id = self.next;
        self.next = self
            .next
            .checked_add(1)
            .expect("directory result identity space exhausted");
        self.by_path.insert(key, id);
        id
    }

    fn retain_paths(&mut self, paths: &BTreeSet<PathBuf>) {
        let keys = paths
            .iter()
            .map(|path| path_match_key(path))
            .collect::<BTreeSet<_>>();
        self.by_path.retain(|path, _| keys.contains(path));
    }

    fn clear(&mut self) {
        self.by_path.clear();
    }
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
            preferences: PathPreferences::default(),
            results: GlobalSearchResults::default(),
            matcher: None,
            result_scope: None,
            all_open_context: RetainedGlobalSearchContext::default(),
            directory_context: RetainedGlobalSearchContext::default(),
            pending_all_open_restore: None,
            pending_directory_restore: None,
            restoring_selection: false,
            revision: 0,
            results_visible: false,
            directory_document_ids: DirectoryDocumentIds::default(),
        }
    }

    pub(crate) fn directory_document_id(&mut self, path: &Path) -> u64 {
        self.directory_document_ids.id_for_path(path)
    }

    pub(crate) fn replace_preferences(&mut self, preferences: BTreeMap<PathBuf, bool>) {
        self.preferences.replace(preferences);
    }

    pub(crate) fn preference_for(&self, path: &Path) -> Option<bool> {
        self.preferences.get(path)
    }

    pub(crate) fn set_preference(&mut self, path: PathBuf, selected: bool) {
        self.preferences.insert(path, selected);
    }

    pub(crate) fn retain_directory_document_paths(&mut self, paths: &BTreeSet<PathBuf>) {
        self.directory_document_ids.retain_paths(paths);
    }

    pub(crate) fn clear_directory_document_ids(&mut self) {
        self.directory_document_ids.clear();
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SearchTarget {
    Document(u64),
    AllOpenFiles,
    Directory,
}

struct ActiveSearch {
    target: SearchTarget,
    revision: u64,
    cancellation: SearchCancellation,
}

#[derive(Default)]
pub(crate) struct SearchController {
    active: Option<ActiveSearch>,
    task: Option<Task<()>>,
}

impl SearchController {
    pub(crate) fn begin(
        &mut self,
        target: SearchTarget,
        revision: u64,
        cancellation: SearchCancellation,
    ) {
        self.cancel();
        self.active = Some(ActiveSearch {
            target,
            revision,
            cancellation,
        });
    }

    pub(crate) fn is_active(&self) -> bool {
        self.active.is_some()
    }

    pub(crate) fn is_current(&self, target: SearchTarget, revision: u64) -> bool {
        self.active
            .as_ref()
            .is_some_and(|search| search.target == target && search.revision == revision)
    }

    pub(crate) fn has_target(&self, target: SearchTarget) -> bool {
        self.active
            .as_ref()
            .is_some_and(|search| search.target == target)
    }

    pub(crate) fn is_affected_by_removed_documents(&self, document_ids: &BTreeSet<u64>) -> bool {
        self.active
            .as_ref()
            .is_some_and(|search| match search.target {
                SearchTarget::Document(document_id) => document_ids.contains(&document_id),
                SearchTarget::AllOpenFiles => !document_ids.is_empty(),
                SearchTarget::Directory => !document_ids.is_empty(),
            })
    }

    pub(crate) fn is_affected_by_added_documents(&self) -> bool {
        self.active.as_ref().is_some_and(|search| {
            matches!(
                search.target,
                SearchTarget::AllOpenFiles | SearchTarget::Directory
            )
        })
    }

    pub(crate) fn cancel_for_document(&mut self, document_id: u64) -> bool {
        let should_cancel = self.active.as_ref().is_some_and(|search| {
            matches!(search.target, SearchTarget::Document(id) if id == document_id)
                || search.target == SearchTarget::AllOpenFiles
        });
        should_cancel && self.cancel()
    }

    pub(crate) fn set_task(&mut self, task: Task<()>) {
        self.task = Some(task);
    }

    pub(crate) fn finish(&mut self, target: SearchTarget, revision: u64) -> bool {
        if !self.is_current(target, revision) {
            return false;
        }
        self.active = None;
        self.task = None;
        true
    }

    pub(crate) fn cancel(&mut self) -> bool {
        let was_active = if let Some(search) = self.active.take() {
            search.cancellation.cancel();
            true
        } else {
            false
        };
        self.task = None;
        was_active
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

#[cfg(test)]
mod state_controller_tests {
    use super::*;

    fn global_result(path: &str) -> GlobalSearchDocumentResult {
        GlobalSearchDocumentResult {
            title: path.to_string().into(),
            path: PathBuf::from(path),
            document: Arc::new(LogDocument::placeholder(path)),
            search_result: SearchResult::default(),
            failure: None,
        }
    }

    #[test]
    fn quick_find_document_version_tracks_snapshot_and_projection() {
        let document = Arc::new(LogDocument::placeholder("quick-find.log"));
        let rows: CompressedRows = [2, 7].into_iter().collect();
        let version = QuickFindSourceVersion::Document {
            document: document.clone(),
            rows: Some(rows.clone()),
        };

        assert!(version.is_same_as(&QuickFindSourceVersion::Document {
            document: document.clone(),
            rows: Some(rows),
        }));
        assert!(!version.is_same_as(&QuickFindSourceVersion::Document {
            document: document.clone(),
            rows: Some([2, 8].into_iter().collect()),
        }));
        assert!(!version.is_same_as(&QuickFindSourceVersion::Document {
            document: Arc::new(LogDocument::placeholder("quick-find.log")),
            rows: Some([2, 7].into_iter().collect()),
        }));
    }

    #[test]
    fn quick_find_global_version_tracks_content_and_layout() {
        let version = QuickFindSourceVersion::Global {
            content_revision: 3,
            layout_revision: 5,
        };

        assert!(version.is_same_as(&QuickFindSourceVersion::Global {
            content_revision: 3,
            layout_revision: 5,
        }));
        assert!(!version.is_same_as(&QuickFindSourceVersion::Global {
            content_revision: 4,
            layout_revision: 5,
        }));
        assert!(!version.is_same_as(&QuickFindSourceVersion::Global {
            content_revision: 3,
            layout_revision: 6,
        }));
    }

    #[test]
    fn global_results_keep_search_order_separate_from_identity_lookup() {
        let mut results = [
            (9, global_result("nine.log")),
            (3, global_result("three.log")),
        ]
        .into_iter()
        .collect::<GlobalSearchResults>();

        assert_eq!(
            results.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            [9, 3]
        );
        assert_eq!(
            results.get(&3).map(|result| result.path.as_path()),
            Some(Path::new("three.log"))
        );

        results.remove_documents(&BTreeSet::from([9]));
        assert_eq!(results.iter().map(|(id, _)| *id).collect::<Vec<_>>(), [3]);
        assert!(results.get(&9).is_none());
    }

    #[test]
    fn cloned_global_results_share_storage_until_mutated() {
        let mut results = [(3, global_result("three.log"))]
            .into_iter()
            .collect::<GlobalSearchResults>();
        let snapshot = results.clone();

        assert!(Arc::ptr_eq(&results.order, &snapshot.order));
        assert!(Arc::ptr_eq(&results.by_document, &snapshot.by_document));

        results.remove_documents(&BTreeSet::from([99]));

        assert!(Arc::ptr_eq(&results.order, &snapshot.order));
        assert!(Arc::ptr_eq(&results.by_document, &snapshot.by_document));

        results.remove_documents(&BTreeSet::from([3]));

        assert!(!Arc::ptr_eq(&results.order, &snapshot.order));
        assert!(!Arc::ptr_eq(&results.by_document, &snapshot.by_document));
        assert!(results.is_empty());
        assert!(snapshot.get(&3).is_some());
    }

    #[test]
    fn retained_global_context_drops_closed_documents_and_interactions() {
        let mut context = RetainedGlobalSearchContext {
            initialized: true,
            results: [
                (9, global_result("nine.log")),
                (3, global_result("three.log")),
            ]
            .into_iter()
            .collect(),
            collapsed_document_ids: BTreeSet::from([3, 9]),
            selection: BTreeMap::from([
                (3, [1].into_iter().collect()),
                (9, [2].into_iter().collect()),
            ]),
            selected_row: Some(GlobalSearchRow::Match {
                document_id: 9,
                source_row: 2,
            }),
            viewport: Some(ViewportAnchor {
                key: LogRowKey::Row {
                    document_id: 9,
                    source_row: 2,
                },
                viewport_y: gpui::px(0.),
                at_end: false,
                fallback_ix: 0,
            }),
            ..Default::default()
        };

        context.remove_documents(&BTreeSet::from([9]));

        assert_eq!(
            context
                .results
                .iter()
                .map(|(id, _)| *id)
                .collect::<Vec<_>>(),
            [3]
        );
        assert_eq!(context.collapsed_document_ids, BTreeSet::from([3]));
        assert_eq!(context.selection.keys().copied().collect::<Vec<_>>(), [3]);
        assert_eq!(context.selected_row, None);
        assert!(context.viewport.is_none());
    }

    #[test]
    fn invalidating_retained_results_preserves_presentation_preferences() {
        let mut context = RetainedGlobalSearchContext {
            initialized: true,
            results: [(3, global_result("a.log"))].into_iter().collect(),
            matcher: SearchMatcher::literal_phrase("needle").expect("matcher should compile"),
            result_mode: ResultMode::MarksOnly,
            results_visible: true,
            collapsed_document_ids: BTreeSet::from([3]),
            selection: BTreeMap::from([(3, [2].into_iter().collect())]),
            selected_row: Some(GlobalSearchRow::Match {
                document_id: 3,
                source_row: 2,
            }),
            viewport: Some(ViewportAnchor {
                key: LogRowKey::Row {
                    document_id: 3,
                    source_row: 2,
                },
                viewport_y: gpui::px(0.),
                at_end: false,
                fallback_ix: 1,
            }),
            horizontal_offset: 12.,
            word_wrap: true,
            active: true,
        };

        context.invalidate_results();

        assert!(!context.initialized);
        assert!(context.results.is_empty());
        assert!(context.matcher.is_none());
        assert!(!context.results_visible);
        assert!(context.collapsed_document_ids.is_empty());
        assert!(context.selection.is_empty());
        assert!(context.selected_row.is_none());
        assert!(context.viewport.is_none());
        assert_eq!(context.horizontal_offset, 0.);
        assert_eq!(context.result_mode, ResultMode::MarksOnly);
        assert!(context.word_wrap);
        assert!(!context.active);
    }

    #[test]
    fn directory_result_identity_is_stable_by_path() {
        let mut identities = DirectoryDocumentIds::default();
        let first = identities.id_for_path(Path::new("logs/a.log"));
        let second = identities.id_for_path(Path::new("logs/b.log"));

        assert_ne!(first, second);
        assert_eq!(identities.id_for_path(Path::new("logs/a.log")), first);
        assert_eq!(identities.id_for_path(Path::new("logs/b.log")), second);
        assert!(first >= DIRECTORY_DOCUMENT_ID_BASE);
        assert_eq!(
            identities.id_for_path(Path::new("LOGS/A.LOG")) == first,
            cfg!(windows)
        );

        identities.retain_paths(&BTreeSet::from([PathBuf::from("logs/b.log")]));
        assert_eq!(identities.by_path.len(), 1);
        assert_eq!(identities.id_for_path(Path::new("logs/b.log")), second);
        assert_ne!(identities.id_for_path(Path::new("logs/a.log")), first);
    }

    #[test]
    fn global_search_preferences_use_platform_path_identity() {
        let mut preferences = PathPreferences::default();
        preferences.replace(BTreeMap::from([(PathBuf::from("logs/a.log"), false)]));

        assert_eq!(preferences.get(Path::new("logs/a.log")), Some(false));
        assert_eq!(
            preferences.get(Path::new("LOGS/A.LOG")),
            cfg!(windows).then_some(false)
        );
    }

    #[test]
    fn document_reload_cancels_open_file_searches_but_not_directory_searches() {
        let mut controller = SearchController::default();
        let directory_cancellation = SearchCancellation::default();
        controller.begin(SearchTarget::Directory, 1, directory_cancellation.clone());

        assert!(!controller.cancel_for_document(7));
        assert!(!directory_cancellation.is_cancelled());
        assert!(controller.is_current(SearchTarget::Directory, 1));

        let open_files_cancellation = SearchCancellation::default();
        controller.begin(
            SearchTarget::AllOpenFiles,
            2,
            open_files_cancellation.clone(),
        );

        assert!(directory_cancellation.is_cancelled());
        assert!(controller.cancel_for_document(7));
        assert!(open_files_cancellation.is_cancelled());
    }

    #[test]
    fn removing_documents_affects_every_search_that_captured_workspace_membership() {
        let removed = BTreeSet::from([7]);
        let mut controller = SearchController::default();

        controller.begin(SearchTarget::Directory, 1, SearchCancellation::default());
        assert!(controller.is_affected_by_removed_documents(&removed));

        controller.begin(SearchTarget::AllOpenFiles, 2, SearchCancellation::default());
        assert!(controller.is_affected_by_removed_documents(&removed));

        controller.begin(SearchTarget::Document(8), 3, SearchCancellation::default());
        assert!(!controller.is_affected_by_removed_documents(&removed));
        controller.begin(SearchTarget::Document(7), 4, SearchCancellation::default());
        assert!(controller.is_affected_by_removed_documents(&removed));
    }

    #[test]
    fn adding_documents_only_affects_workspace_wide_searches() {
        let mut controller = SearchController::default();

        controller.begin(SearchTarget::Directory, 1, SearchCancellation::default());
        assert!(controller.is_affected_by_added_documents());

        controller.begin(SearchTarget::AllOpenFiles, 2, SearchCancellation::default());
        assert!(controller.is_affected_by_added_documents());

        controller.begin(SearchTarget::Document(7), 3, SearchCancellation::default());
        assert!(!controller.is_affected_by_added_documents());
    }

    #[test]
    fn stale_completion_cannot_replace_the_current_search() {
        let mut controller = SearchController::default();
        let target = SearchTarget::Document(7);
        controller.begin(target, 2, SearchCancellation::default());

        assert!(!controller.finish(target, 1));
        assert!(controller.is_current(target, 2));
        assert!(controller.finish(target, 2));
        assert!(!controller.is_active());
    }
}
