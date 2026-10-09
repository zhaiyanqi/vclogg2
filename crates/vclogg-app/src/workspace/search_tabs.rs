//! Search sessions own queries and results; the existing tables are only installed projections.
use super::search_tab_tasks::SearchTabJob;
use super::*;
use crate::search_context::{PersistedSearchTab, PersistedSearchTabGroup, SearchTabQuery};
use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(super) struct SearchTabId(pub(super) u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(super) enum SearchTabOwner {
    File(u64),
    AllOpen,
    Directory,
}

impl SearchTabOwner {
    pub(super) fn icon(self) -> gpui_kit::assets::IconName {
        match self {
            Self::File(_) => gpui_kit::assets::IconName::Search,
            Self::AllOpen => gpui_kit::assets::IconName::Earth,
            Self::Directory => gpui_kit::assets::IconName::FolderOpen,
        }
    }

    pub(super) fn scope(self) -> SearchScope {
        match self {
            Self::File(_) => SearchScope::CurrentFile,
            Self::AllOpen => SearchScope::AllOpenFiles,
            Self::Directory => SearchScope::Directory,
        }
    }
}

#[derive(Clone)]
pub(super) struct SearchTabState {
    pub(super) saved: PersistedSearchTab,
    pub(super) local_result: Option<(Arc<LogDocument>, SearchResult, Option<SearchMatcher>)>,
    pub(super) context: SearchSessionState,
    pub(super) ranges: search_limits::FileSearchRanges,
    pub(super) revision: u64,
    pub(super) needs_restore: bool,
    pub(super) facade_dirty: bool,
}

impl SearchTabState {
    pub(super) fn prepare_result_viewport(&mut self, restore: bool) {
        if restore {
            return;
        }
        // A new query replaces the projection. A short previous result also counts as
        // being at the bottom, but must retain its row anchor when more matches arrive.
        if let Some(viewport) = self.saved.local.viewport.as_mut() {
            viewport.at_end = false;
        }
        if let Some(viewport) = self.saved.context.viewport.as_mut() {
            viewport.at_end = false;
        }
    }

    pub(super) fn restored(mut saved: PersistedSearchTab) -> Self {
        saved
            .created_at
            .get_or_insert_with(|| Local::now().format("%Y-%m-%d %H:%M:%S").to_string());
        Self {
            needs_restore: saved.completed.is_some() || saved.submitted.is_some(),
            ranges: search_limits::FileSearchRanges::restored(&saved.ranges),
            facade_dirty: false,
            context: SearchSessionState {
                query: saved.completed.as_ref().unwrap_or(&saved.draft).query(),
                result_mode: ResultMode::from_database(saved.context.result_mode),
                word_wrap: saved.context.word_wrap,
                active: saved.context.active,
                ..SearchSessionState::default()
            },
            saved,
            local_result: None,
            revision: 0,
        }
    }
}

pub(super) struct SearchTabGroup {
    pub(super) active: SearchTabId,
    pub(super) next_id: u64,
    pub(super) tabs: Vec<SearchTabState>,
}

impl SearchTabGroup {
    pub(super) fn restored(mut saved: PersistedSearchTabGroup) -> Self {
        let mut seen = BTreeSet::new();
        saved.tabs.retain(|tab| tab.id != 0 && seen.insert(tab.id));
        let active = if saved.tabs.iter().any(|tab| tab.id == saved.active) {
            saved.active
        } else {
            saved.tabs.first().map_or(0, |tab| tab.id)
        };
        Self {
            active: SearchTabId(active),
            next_id: saved.next_id.max(
                saved
                    .tabs
                    .iter()
                    .map(|tab| tab.id)
                    .max()
                    .unwrap_or(0)
                    .saturating_add(1),
            ),
            tabs: saved
                .tabs
                .into_iter()
                .map(SearchTabState::restored)
                .collect(),
        }
    }
    pub(super) fn scope_target(&self, owner: SearchTabOwner) -> Option<SearchTabId> {
        match owner {
            SearchTabOwner::File(_) => self.tabs.iter().find(|tab| tab.saved.id == self.active.0),
            _ => self.tabs.first(),
        }
        .map(|tab| SearchTabId(tab.saved.id))
    }

    pub(super) fn is_closable(&self, owner: SearchTabOwner, id: SearchTabId) -> bool {
        self.tabs.iter().any(|tab| tab.saved.id == id.0)
            && (!matches!(owner, SearchTabOwner::File(_)) || self.tabs.len() > 1)
    }

    pub(super) fn close(&mut self, owner: SearchTabOwner, id: SearchTabId) {
        if !self.is_closable(owner, id) {
            return;
        }
        let Some(ix) = self.tabs.iter().position(|tab| tab.saved.id == id.0) else {
            return;
        };
        self.tabs.remove(ix);
        if self.active == id {
            self.active = self
                .tabs
                .get(ix.min(self.tabs.len().saturating_sub(1)))
                .map_or(SearchTabId(0), |tab| SearchTabId(tab.saved.id));
            if matches!(owner, SearchTabOwner::File(_))
                && let Some(tab) = self
                    .tabs
                    .iter_mut()
                    .find(|tab| tab.saved.id == self.active.0)
            {
                // Closing a visible file tab while a global search is selected must not
                // capture the old file result table into the newly selected local session.
                tab.facade_dirty = true;
            }
        }
    }

    pub(super) fn persisted(&self) -> PersistedSearchTabGroup {
        PersistedSearchTabGroup {
            active: self.active.0,
            next_id: self.next_id,
            tabs: self.tabs.iter().map(|tab| tab.saved.clone()).collect(),
        }
    }
}

pub(super) struct ClosingSearchTab {
    pub(super) key: (SearchTabOwner, SearchTabId),
    pub(super) index: usize,
    pub(super) title: SharedString,
    pub(super) selected: bool,
    pub(super) width: Pixels,
}

pub(super) struct SearchTabs {
    pub(super) closing: Vec<ClosingSearchTab>,
    pub(super) strip_collapsed: bool,
    pub(super) strip_expanded_width: Pixels,
    pub(super) panel_expansion: BTreeMap<WorkspaceTabId, bool>,
    pub(super) groups: BTreeMap<SearchTabOwner, SearchTabGroup>,
    pub(super) installed: Option<(SearchTabOwner, SearchTabId)>,
    pub(super) focus: FocusHandle,
    pub(super) scroll: ScrollHandle,
    pub(super) layout: Rc<RefCell<super::search_tab_order::SearchTabLayout>>,
    pub(super) order_revision: u64,
    pub(super) hidden_drag: Option<(SearchTabOwner, SearchTabId)>,
    pub(super) dragging: bool,
    pub(super) motion_offsets: BTreeMap<(SearchTabOwner, SearchTabId), Pixels>,
    pub(super) queue: VecDeque<SearchTabJob>,
    pub(super) running: Option<(SearchTabOwner, SearchTabId, u64, SearchCancellation)>,
    pub(super) task: Option<Task<()>>,
    pub(super) syncing: bool,
    pub(super) activation_revision: u64,
    pub(super) activation_task: Option<Task<()>>,
    pub(super) activation_cancellation: Option<Arc<AtomicBool>>,
    pub(super) prepared_frame: Option<super::search_tab_activation::PreparedSearchTabFrame>,
}

impl SearchTabs {
    pub(super) fn new(cx: &mut App) -> Self {
        Self {
            strip_collapsed: false,
            closing: Vec::new(),
            strip_expanded_width: px(0.),
            panel_expansion: BTreeMap::new(),
            groups: BTreeMap::new(),
            installed: None,
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            layout: Rc::default(),
            order_revision: 0,
            hidden_drag: None,
            dragging: false,
            motion_offsets: BTreeMap::new(),
            queue: VecDeque::new(),
            running: None,
            task: None,
            syncing: false,
            activation_revision: 0,
            activation_task: None,
            activation_cancellation: None,
            prepared_frame: None,
        }
    }
    pub(super) fn state(&self, owner: SearchTabOwner, id: SearchTabId) -> Option<&SearchTabState> {
        self.groups
            .get(&owner)?
            .tabs
            .iter()
            .find(|tab| tab.saved.id == id.0)
    }
    pub(super) fn state_mut(
        &mut self,
        owner: SearchTabOwner,
        id: SearchTabId,
    ) -> Option<&mut SearchTabState> {
        self.groups
            .get_mut(&owner)?
            .tabs
            .iter_mut()
            .find(|tab| tab.saved.id == id.0)
    }
    pub(super) fn visible_keys(&self) -> Vec<(SearchTabOwner, SearchTabId)> {
        let mut keys = self
            .groups
            .iter()
            .flat_map(|(owner, group)| {
                group
                    .tabs
                    .iter()
                    .map(move |tab| (*owner, SearchTabId(tab.saved.id)))
            })
            .collect::<Vec<_>>();
        keys.sort_by_key(|&(owner, id)| {
            self.state(owner, id)
                .unwrap()
                .saved
                .position
                .unwrap_or(u64::MAX)
        });
        keys
    }

    pub(super) fn next_position(&mut self) -> u64 {
        let mut next = self
            .groups
            .values()
            .flat_map(|group| &group.tabs)
            .filter_map(|tab| tab.saved.position)
            .max()
            .map_or(0, |position| position.saturating_add(1));
        // Give legacy sessions a position before appending anything new, preserving
        // the order already displayed across all three search scopes.
        for (owner, id) in self.visible_keys() {
            let saved = &mut self.state_mut(owner, id).unwrap().saved;
            if saved.position.is_none() {
                saved.position = Some(next);
                next = next.saturating_add(1);
            }
        }
        next
    }

    pub(super) fn reorder(&mut self, key: (SearchTabOwner, SearchTabId), target_ix: usize) -> bool {
        let mut keys = self.visible_keys();
        let Some(from) = keys.iter().position(|candidate| *candidate == key) else {
            return false;
        };
        let to = target_ix.min(keys.len() - 1);
        if from == to {
            return false;
        }
        keys.remove(from);
        keys.insert(to, key);
        for (ix, &(owner, id)) in keys.iter().enumerate() {
            self.state_mut(owner, id).unwrap().saved.position = Some(ix as u64);
        }
        for group in self.groups.values_mut() {
            group.tabs.sort_by_key(|tab| tab.saved.position);
        }
        true
    }

    pub(super) fn busy(&self) -> bool {
        self.running.is_some() || !self.queue.is_empty()
    }
    pub(super) fn cancel(&mut self, owner: SearchTabOwner, id: SearchTabId) -> bool {
        let previous = self.queue.len();
        self.queue.retain(|job| job.owner != owner || job.id != id);
        let running = self
            .running
            .as_ref()
            .is_some_and(|(o, i, _, _)| *o == owner && *i == id);
        // Keep the physical worker slot until its cancellation completes, avoiding overlapping scans.
        if running && let Some((_, _, _, cancellation)) = &self.running {
            cancellation.cancel();
        }
        if let Some(state) = self.state_mut(owner, id) {
            state.revision = state.revision.saturating_add(1);
            state.saved.submitted = None;
        }
        running || previous != self.queue.len()
    }
}

impl Drop for SearchTabs {
    fn drop(&mut self) {
        if let Some(cancel) = &self.activation_cancellation {
            cancel.store(true, Ordering::Release);
        }
        if let Some((_, _, _, cancellation)) = &self.running {
            cancellation.cancel();
        }
    }
}

impl SearchTabQuery {
    pub(super) fn from_query(query: &SearchQuery) -> Self {
        Self {
            text: query.text.clone(),
            case_sensitive: query.case_sensitive,
            regex: query.regex,
            max_results: query.max_results,
        }
    }
    pub(super) fn query(&self) -> SearchQuery {
        SearchQuery {
            text: self.text.clone(),
            case_sensitive: self.case_sensitive,
            regex: self.regex,
            max_results: self.max_results,
        }
    }
}

impl Workspace {
    pub(super) fn search_tab_title(
        &self,
        owner: SearchTabOwner,
        state: &SearchTabState,
    ) -> SharedString {
        if let Some(name) = &state.saved.name {
            return name.clone().into();
        }
        match owner {
            SearchTabOwner::File(id) => {
                let filename = self
                    .documents
                    .iter()
                    .find(|tab| tab.id == id)
                    .and_then(|tab| tab.document.path().file_name())
                    .map(|name| name.to_string_lossy())
                    .unwrap_or_else(|| crate::tr!("当前文件", "Current file").into());
                format!("{}_{}", filename, state.saved.id).into()
            }
            SearchTabOwner::AllOpen => crate::tr_args!(
                "全局搜索{}",
                "Global search {}",
                state.saved.created_at.as_deref().unwrap_or_default()
            )
            .into(),
            SearchTabOwner::Directory => crate::tr_args!(
                "目录搜索{}",
                "Directory search {}",
                state.saved.created_at.as_deref().unwrap_or_default()
            )
            .into(),
        }
    }

    pub(super) fn persist_search_tab_changes(
        &mut self,
        owner: SearchTabOwner,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let SearchTabOwner::File(id) = owner
            && self.active_document().map(|tab| tab.id) != Some(id)
        {
            self.schedule_checkpoint(id, window, cx);
        }
        self.persist_search_tabs(window, cx);
    }

    pub(super) fn search_tab_owner(&self) -> Option<SearchTabOwner> {
        match self.global_search.scope {
            SearchScope::CurrentFile => self
                .active_document()
                .map(|tab| SearchTabOwner::File(tab.id)),
            SearchScope::AllOpenFiles => Some(SearchTabOwner::AllOpen),
            SearchScope::Directory => Some(SearchTabOwner::Directory),
        }
    }

    pub(super) fn active_search_tab_key(&self) -> Option<(SearchTabOwner, SearchTabId)> {
        let owner = self.search_tab_owner()?;
        let id = self.search_tabs.groups.get(&owner)?.active;
        self.search_tabs.state(owner, id)?;
        Some((owner, id))
    }

    fn initial_search_tab(&self, owner: SearchTabOwner) -> PersistedSearchTab {
        let query = match owner {
            SearchTabOwner::File(id) => self
                .documents
                .iter()
                .find(|tab| tab.id == id)
                .map(|tab| tab.search_query.clone())
                .unwrap_or_default(),
            SearchTabOwner::AllOpen => self.global_search.query.clone(),
            SearchTabOwner::Directory => self.global_search.directory_query.clone(),
        };
        let context = match owner {
            SearchTabOwner::AllOpen => self.persisted_global_context(
                owner.scope(),
                &self.global_search.all_open_context,
                self.global_search.pending_all_open_restore.as_ref(),
            ),
            SearchTabOwner::Directory => self.persisted_global_context(
                owner.scope(),
                &self.global_search.directory_context,
                self.global_search.pending_directory_restore.as_ref(),
            ),
            SearchTabOwner::File(id) => self
                .documents
                .iter()
                .find(|tab| tab.id == id)
                .map(|tab| PersistedGlobalSearchContext {
                    results_visible: tab.results_visible,
                    result_mode: tab.result_mode.database_value(),
                    word_wrap: tab.result_viewport.is_wrapped(),
                    ..Default::default()
                })
                .unwrap_or_default(),
        };
        PersistedSearchTab {
            id: 1,
            draft: SearchTabQuery::from_query(&query),
            completed: context
                .results_visible
                .then(|| SearchTabQuery::from_query(&query)),
            selected_paths: if self.documents.is_empty() {
                context.source_paths.clone()
            } else {
                self.documents
                    .iter()
                    .filter(|tab| self.global_search.selected_documents.contains(&tab.id))
                    .map(|tab| encode_persisted_path(tab.document.path()))
                    .collect()
            },
            targets_configured: !self.documents.is_empty() || !context.source_paths.is_empty(),
            context,
            directory: Self::persisted_directory_options(&self.global_search.directory_options),
            ranges: self.search_ranges.persisted(),
            ..Default::default()
        }
    }

    pub(super) fn ensure_search_tab_group(&mut self, owner: SearchTabOwner, cx: &App) {
        if self
            .search_tabs
            .groups
            .get(&owner)
            .is_some_and(|group| !group.tabs.is_empty())
        {
            return;
        }
        let position = self.search_tabs.next_position();
        if let Some(group) = self.search_tabs.groups.get_mut(&owner) {
            if group.tabs.is_empty() {
                let id = group.next_id.max(1);
                group.next_id = id.saturating_add(1);
                group.active = SearchTabId(id);
                group
                    .tabs
                    .push(SearchTabState::restored(PersistedSearchTab {
                        id,
                        position: Some(position),
                        ..Default::default()
                    }));
            }
            return;
        }
        let persisted = match owner {
            SearchTabOwner::File(id) => {
                self.documents
                    .iter()
                    .find(|tab| tab.id == id)
                    .and_then(|tab| {
                        tab.view
                            .pending_resume
                            .as_ref()
                            .and_then(|resume| resume.search_tabs.clone())
                            .or_else(|| tab.session_base.resume.search_tabs.clone())
                    })
            }
            _ => None,
        };
        let persisted = persisted.filter(|group| !group.tabs.is_empty());
        let has_saved = persisted.is_some();
        let saved = persisted.unwrap_or_else(|| PersistedSearchTabGroup {
            active: 1,
            next_id: 2,
            tabs: vec![self.initial_search_tab(owner)],
        });
        self.search_tabs
            .groups
            .insert(owner, SearchTabGroup::restored(saved));
        self.search_tabs.next_position();
        if self.search_tabs.groups[&owner].tabs.is_empty() {
            self.ensure_search_tab_group(owner, cx);
        }
        if !has_saved {
            let id = self.search_tabs.groups[&owner].active;
            let state = self.capture_search_tab_state(owner, id, cx);
            if let Some(state) = state {
                *self.search_tabs.state_mut(owner, id).unwrap() = state;
            }
        }
    }

    pub(super) fn capture_search_tab_state(
        &self,
        owner: SearchTabOwner,
        id: SearchTabId,
        cx: &App,
    ) -> Option<SearchTabState> {
        let mut state = self.search_tabs.state(owner, id)?.clone();
        if self.search_tabs.groups.get(&owner)?.active != id {
            return Some(state);
        }
        let installed = self.search_tabs.installed == Some((owner, id))
            && self.search_tab_owner() == Some(owner);
        if installed {
            state.saved.draft = SearchTabQuery {
                text: self.query.read(cx).value().to_string(),
                case_sensitive: self.case_sensitive,
                regex: self.regex,
                max_results: state.saved.draft.max_results,
            };
            state.ranges = self.search_ranges.clone();
            state.saved.ranges = state.ranges.persisted();
        }
        match owner {
            SearchTabOwner::File(document_id) => {
                let tab = self.documents.iter().find(|tab| tab.id == document_id)?;
                if state.needs_restore
                    || state.facade_dirty
                    || (state.saved.completed.is_some() && state.local_result.is_none())
                {
                    return Some(state);
                }
                let table = tab.result_table.read(cx);
                state.local_result = Some((
                    tab.document.clone(),
                    tab.search_result.clone(),
                    tab.search_matcher.clone(),
                ));
                state.saved.context.results_visible = tab.results_visible;
                state.saved.context.result_mode = tab.result_mode.database_value();
                state.saved.context.word_wrap = tab.result_viewport.is_wrapped();
                state.saved.context.active = tab.view.selection_table == SelectionTable::Results;
                state.saved.context.selection = vec![PersistedPathSelection::new(
                    encode_persisted_path(tab.document.path()),
                    &table.delegate().selected_source_rows_compressed(),
                )];
                state.saved.local.results_visible = tab.results_visible;
                state.saved.local.selected_source_row = table
                    .active_log_row()
                    .and_then(|ix| table.delegate().source_row(ix));
                state.saved.local.viewport = Self::capture_persisted_local_viewport(
                    tab,
                    WrappedRegion::Results,
                    self.log_row_height(),
                    cx,
                );
                if tab.results_visible {
                    state.saved.completed = Some(SearchTabQuery::from_query(&tab.search_query));
                }
            }
            _ if installed => {
                let context = if owner == SearchTabOwner::AllOpen {
                    &self.global_search.all_open_context
                } else {
                    &self.global_search.directory_context
                };
                if !state.needs_restore
                    && (state.saved.completed.is_none() || state.context.initialized)
                {
                    state.context = context.clone();
                    state.context.visible_lines = None;
                    state.saved.context =
                        self.persisted_global_context(owner.scope(), context, None);
                }
                state.saved.directory =
                    Self::persisted_directory_options(&self.global_search.directory_options);
                if owner == SearchTabOwner::AllOpen {
                    state.saved.targets_configured |= !self.documents.is_empty();
                    state.saved.selected_paths = self
                        .documents
                        .iter()
                        .filter(|tab| self.global_search.selected_documents.contains(&tab.id))
                        .map(|tab| encode_persisted_path(tab.document.path()))
                        .collect();
                }
            }
            _ => {}
        }
        Some(state)
    }

    pub(super) fn capture_active_search_tab(&mut self, cx: &App) {
        if self.search_tabs.syncing {
            return;
        }
        let Some((owner, id)) = self.search_tabs.installed else {
            return;
        };
        if owner.scope() != SearchScope::CurrentFile && self.search_tab_owner() == Some(owner) {
            self.capture_retained_global_context(owner.scope(), cx);
        }
        if let Some(state) = self.capture_search_tab_state(owner, id, cx)
            && let Some(slot) = self.search_tabs.state_mut(owner, id)
        {
            *slot = state;
        }
        self.global_search.all_open_context.visible_lines = None;
        self.global_search.directory_context.visible_lines = None;
    }

    pub(super) fn visible_search_tab_keys(&self) -> Vec<(SearchTabOwner, SearchTabId)> {
        self.search_tabs.visible_keys()
    }

    pub(super) fn sync_search_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(tab) = self
            .active_document()
            .filter(|tab| tab.load_state == DocumentLoadState::Ready)
        {
            self.ensure_search_tab_group(SearchTabOwner::File(tab.id), cx);
        }
        let Some(owner) = self.search_tab_owner() else {
            self.search_tabs.installed = None;
            return;
        };
        if let SearchTabOwner::File(document_id) = owner
            && self
                .documents
                .iter()
                .find(|tab| tab.id == document_id)
                .is_none_or(|tab| tab.load_state != DocumentLoadState::Ready)
        {
            self.search_tabs.installed = None;
            return;
        }
        self.ensure_search_tab_group(owner, cx);
        let id = self.search_tabs.groups[&owner].active;
        if self.search_tabs.installed != Some((owner, id)) {
            self.install_search_tab(owner, id, window, cx);
        }
        // A transferred running search is resumed even when its result tab is not selected.
        let pending = self.search_tabs.groups[&owner]
            .tabs
            .iter()
            .filter(|tab| tab.needs_restore && tab.saved.submitted.is_some())
            .map(|tab| SearchTabId(tab.saved.id))
            .collect::<Vec<_>>();
        for pending_id in pending {
            self.enqueue_search_tab(owner, pending_id, true, window, cx);
        }
    }

    pub(super) fn install_search_tab(
        &mut self,
        owner: SearchTabOwner,
        id: SearchTabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(mut state) = self.search_tabs.state(owner, id).cloned() else {
            return;
        };
        if owner == SearchTabOwner::AllOpen && !state.saved.targets_configured {
            state.saved.selected_paths = self
                .documents
                .iter()
                .filter(|tab| self.global_search.selected_documents.contains(&tab.id))
                .map(|tab| encode_persisted_path(tab.document.path()))
                .collect();
            state.saved.targets_configured = !self.documents.is_empty();
            if let Some(slot) = self.search_tabs.state_mut(owner, id) {
                slot.saved = state.saved.clone();
            }
        }
        self.search_tabs.syncing = true;
        let previous_owner = self.search_tabs.installed.map(|(owner, _)| owner);
        let changing = self.search_tabs.installed != Some((owner, id));
        if changing {
            self.cancel_search_tab_activation();
            self.file_refresh_task.take();
            self.quick_find.close();
            self.refresh_quick_find_highlights(cx);
        }
        self.search_tabs.installed = Some((owner, id));
        self.view_state.active_search = Some(SearchSessionKey::SearchTab(owner, id));
        self.search_ranges = state.ranges.clone();
        // Drafts follow the shared switches; completed/submitted queries retain their semantics.
        state.saved.draft.case_sensitive = self.case_sensitive;
        state.saved.draft.regex = self.regex;
        if let Some(slot) = self.search_tabs.state_mut(owner, id) {
            slot.saved.draft = state.saved.draft.clone();
        }
        self.query.update(cx, |input, cx| {
            input.set_value(state.saved.draft.text.clone(), window, cx)
        });
        self.close_search_autocomplete();
        self.reset_search_history_navigation();
        self.cancel_pending_global_jump_for_search_tab();
        match owner {
            SearchTabOwner::File(document_id) => {
                if changing
                    && previous_owner.is_some_and(|owner| owner.scope() != SearchScope::CurrentFile)
                {
                    self.global_table.update(cx, |table, cx| {
                        table.delegate_mut().set_groups(Vec::new());
                        table.refresh(cx);
                    });
                }
                if let Some(tab) = self.documents.iter_mut().find(|tab| tab.id == document_id) {
                    if let Some(cancel) = tab.result_replace_cancellation.take() {
                        cancel.store(true, Ordering::Release);
                    }
                    tab.result_replace_task = None;
                    tab.result_replace_revision = tab.result_replace_revision.saturating_add(1);
                    tab.search_revision = tab.search_revision.saturating_add(1);
                    tab.search_query = state
                        .saved
                        .completed
                        .as_ref()
                        .unwrap_or(&state.saved.draft)
                        .query();
                    let valid = state
                        .local_result
                        .as_ref()
                        .filter(|(document, _, _)| Arc::ptr_eq(document, &tab.document));
                    if state.local_result.is_some()
                        && valid.is_none()
                        && let Some(slot) = self.search_tabs.state_mut(owner, id)
                    {
                        slot.needs_restore = slot.saved.completed.is_some();
                        slot.local_result = None;
                    }
                    tab.search_result = valid
                        .map(|(_, result, _)| result.clone())
                        .unwrap_or_default();
                    tab.search_matcher = valid.and_then(|(_, _, matcher)| matcher.clone());
                    tab.result_mode = ResultMode::from_database(state.saved.context.result_mode);
                    tab.result_viewport
                        .set_word_wrap(state.saved.context.word_wrap);
                    tab.result_viewport.invalidate_wrapped();
                    tab.results_visible = valid.is_some() && state.saved.context.results_visible;
                    tab.view.selection_table = if state.saved.context.active && tab.results_visible
                    {
                        SelectionTable::Results
                    } else {
                        SelectionTable::Log
                    };
                    tab.result_mode_select.update(cx, |select, cx| {
                        select.set_selected_index(
                            Some(IndexPath::new(tab.result_mode.select_index())),
                            window,
                            cx,
                        )
                    });
                    tab.refresh_search_matcher(self.app_settings.highlight_matches, cx);
                    if let Some(super::search_tab_activation::PreparedSearchTabFrame::File {
                        rows,
                        selection,
                        active,
                        lines,
                    }) = self.search_tabs.prepared_frame.take()
                    {
                        // Delegate-only restoration emits no SelectRow event to suppress.
                        tab.restoring_result_selection = false;
                        tab.result_table.update(cx, |table, cx| {
                            table
                                .delegate_mut()
                                .set_marked_rows(tab.file.marked_rows.clone());
                            table
                                .delegate_mut()
                                .install_search_tab_frame(rows, selection, active, lines);
                            table.refresh(cx);
                            cx.notify();
                        });
                    } else {
                        // Different tabs do not inherit one another's selection.
                        tab.result_table.update(cx, |table, _| {
                            table.delegate().clear_row_selection();
                            table.delegate().set_active_log_row(None);
                        });
                        let rows = tab.compute_result_rows();
                        tab.install_result_rows(rows, cx);
                        let selection = state
                            .saved
                            .context
                            .selection
                            .first()
                            .map(PersistedPathSelection::decoded_rows)
                            .unwrap_or_default();
                        tab.result_table.update(cx, |table, cx| {
                            table.delegate_mut().restore_search_tab_selection(
                                selection,
                                state.saved.local.selected_source_row,
                            );
                            table.refresh(cx);
                            cx.notify();
                        });
                    }
                }
                if let Some(tab) = self.documents.iter().find(|tab| tab.id == document_id) {
                    Self::restore_persisted_local_viewport(
                        tab,
                        WrappedRegion::Results,
                        state
                            .saved
                            .local
                            .viewport
                            .or(Some(ViewportBookmark::default())),
                        self.log_row_height(),
                        cx,
                    );
                    self.active_log_region = if tab.view.selection_table == SelectionTable::Results
                    {
                        LogRegion::CurrentResults
                    } else {
                        LogRegion::Body
                    };
                }
            }
            _ => {
                if let Some(cancel) = self.global_result_replace_cancellation.take() {
                    cancel.store(true, Ordering::Release);
                }
                self.global_result_replace_task = None;
                self.global_result_replace_revision =
                    self.global_result_replace_revision.saturating_add(1);
                self.global_search.revision = self.global_search.revision.saturating_add(1);
                self.global_search.directory_options =
                    Self::restored_directory_options(state.saved.directory.clone());
                if owner == SearchTabOwner::AllOpen {
                    self.global_search.selected_documents =
                        self.documents
                            .iter()
                            .filter(|tab| {
                                state.saved.selected_paths.iter().any(|path| {
                                    Self::persisted_path_matches(tab.document.path(), path)
                                })
                            })
                            .map(|tab| tab.id)
                            .collect();
                    self.global_search.all_open_context = state.context.clone();
                    self.global_search.pending_all_open_restore = None;
                } else {
                    self.global_search.directory_context = state.context.clone();
                    self.global_search.pending_directory_restore = None;
                }
                self.restore_retained_global_context(owner.scope(), window, cx);
            }
        }
        if let Some(slot) = self.search_tabs.state_mut(owner, id) {
            slot.facade_dirty = false;
        }
        self.search_tabs.syncing = false;
        self.bind_active_display_tables(cx);
        Self::refresh_log_surfaces_atomically(
            [
                self.log_viewer.surface.clone(),
                self.search_results_viewer.surface.clone(),
            ],
            window,
            cx,
        );
        self.restore_active_search_tab_if_needed(window, cx);
        cx.notify();
    }

    fn cancel_pending_global_jump_for_search_tab(&mut self) {
        self.pending_search_result_jump = None;
    }

    pub(super) fn persist_search_tabs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.active_document().map(|tab| tab.id) {
            self.schedule_checkpoint(id, window, cx);
        }
        self.schedule_workspace_search_state_save(window, cx);
    }

    pub(super) fn search_ranges_for_owner(
        &self,
        owner: SearchTabOwner,
    ) -> search_limits::FileSearchRanges {
        if self.search_tab_owner() == Some(owner) {
            return self.search_ranges.clone();
        }
        self.search_tabs
            .groups
            .get(&owner)
            .and_then(|group| self.search_tabs.state(owner, group.active))
            .map(|state| state.ranges.clone())
            .unwrap_or_default()
    }

    pub(super) fn search_tabs_document_refreshed(&mut self, document_id: u64, cx: &App) {
        let active = self.active_search_tab_key();
        let path = self
            .documents
            .iter()
            .find(|tab| tab.id == document_id)
            .map(|tab| tab.document.path().to_path_buf());
        for (owner, group) in &mut self.search_tabs.groups {
            for state in &mut group.tabs {
                if active == Some((*owner, SearchTabId(state.saved.id))) {
                    continue;
                }
                let affected = match owner {
                    SearchTabOwner::File(id) => *id == document_id,
                    SearchTabOwner::AllOpen => path.as_ref().is_some_and(|path| {
                        state
                            .saved
                            .context
                            .source_paths
                            .iter()
                            .any(|source| Self::persisted_path_matches(path, source))
                    }),
                    SearchTabOwner::Directory => false,
                };
                if affected && state.saved.completed.is_some() {
                    state.needs_restore = true;
                    state.local_result = None;
                    state.context.visible_lines = None;
                }
            }
        }
        self.capture_active_search_tab(cx);
    }

    pub(super) fn persisted_file_search_tabs(
        &self,
        document_id: u64,
        cx: &App,
    ) -> Option<PersistedSearchTabGroup> {
        let owner = SearchTabOwner::File(document_id);
        let group = self.search_tabs.groups.get(&owner)?;
        let mut saved = group.persisted();
        if let Some(state) = self.capture_search_tab_state(owner, group.active, cx)
            && let Some(slot) = saved.tabs.iter_mut().find(|tab| tab.id == group.active.0)
        {
            *slot = state.saved;
        }
        Some(saved)
    }

    pub(super) fn restore_workspace_search_tabs(&mut self, state: &WorkspaceSearchState) {
        for (owner, persisted) in [
            (SearchTabOwner::AllOpen, state.all_open_tabs.clone()),
            (SearchTabOwner::Directory, state.directory_tabs.clone()),
        ] {
            let group = persisted.unwrap_or_else(|| {
                let used = match owner {
                    SearchTabOwner::AllOpen => {
                        state.active_scope == PersistedSearchScope::AllOpenFiles
                            || state.all_open.results_visible
                            || !state.all_open.query.text.is_empty()
                    }
                    SearchTabOwner::Directory => {
                        state.active_scope == PersistedSearchScope::Directory
                            || !state.directories.is_empty()
                            || state.directory_options.directory.is_some()
                    }
                    SearchTabOwner::File(_) => false,
                };
                if !used {
                    return PersistedSearchTabGroup::default();
                }
                if owner == SearchTabOwner::Directory && !state.directories.is_empty() {
                    let tabs = state
                        .directories
                        .iter()
                        .enumerate()
                        .map(|(ix, session)| {
                            let mut tab = self.initial_search_tab(owner);
                            tab.id = ix as u64 + 1;
                            tab.directory = session.options.clone();
                            tab.context = session.context.clone();
                            let query = Self::restored_search_query(
                                &session.context.query,
                                self.app_settings.default_case_sensitive,
                                self.app_settings.default_use_regex,
                                self.app_settings.search_result_limit(),
                            );
                            tab.draft = SearchTabQuery::from_query(&query);
                            tab.completed =
                                session.context.results_visible.then(|| tab.draft.clone());
                            tab
                        })
                        .collect::<Vec<_>>();
                    let active = tabs
                        .iter()
                        .find(|tab| tab.directory.directory == state.active_directory)
                        .map_or(1, |tab| tab.id);
                    PersistedSearchTabGroup {
                        active,
                        next_id: tabs.len() as u64 + 1,
                        tabs,
                    }
                } else {
                    PersistedSearchTabGroup {
                        active: 1,
                        next_id: 2,
                        tabs: vec![self.initial_search_tab(owner)],
                    }
                }
            });
            self.search_tabs
                .groups
                .insert(owner, SearchTabGroup::restored(group));
        }
        self.search_tabs.installed = None;
    }
}
