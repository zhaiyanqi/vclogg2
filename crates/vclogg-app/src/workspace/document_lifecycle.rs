use super::*;

impl Workspace {
    pub(super) fn save_file_session(
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

    pub(super) fn session_contents_equal(
        left: &FileSessionState,
        right: &FileSessionState,
    ) -> bool {
        left.custom_title == right.custom_title
            && left.selected_row == right.selected_row
            && left.query_text == right.query_text
            && left.result_mode == right.result_mode
            && left.marked_rows == right.marked_rows
            && left.show_line_numbers == right.show_line_numbers
            && left.show_row_separators == right.show_row_separators
            && left.word_wrap == right.word_wrap
            && left.keyword_color_rules == right.keyword_color_rules
            && left.resume == right.resume
    }

    pub(super) fn file_session_state(&self, tab: &DocumentTab, cx: &App) -> FileSessionState {
        let mut marked_rows = tab.file.marked_rows.clone();
        marked_rows.insert_rows(&tab.file.pending_restore_marked_rows);
        let selected_row = tab
            .log_table
            .read(cx)
            .active_log_row()
            .and_then(|row_ix| tab.log_table.read(cx).delegate().source_row(row_ix))
            .or(tab.view.pending_restore_row);
        let (selected_result_ix, selected_result_source_row) = {
            let result_table = tab.result_table.read(cx);
            let selected_result_ix = result_table.active_log_row();
            let selected_result_source_row = selected_result_ix
                .and_then(|row_ix| result_table.delegate().source_row(row_ix))
                .or_else(|| {
                    tab.view
                        .pending_resume
                        .as_ref()
                        .and_then(|resume| resume.current_search.selected_source_row)
                });
            (selected_result_ix, selected_result_source_row)
        };

        let row_height = self.log_row_height();
        let mut resume = tab.view.pending_resume.clone().unwrap_or_default();
        resume.viewer.viewport =
            Self::capture_persisted_local_viewport(tab, WrappedRegion::Log, row_height, cx)
                .or(resume.viewer.viewport);
        resume.viewer.auto_follow = tab.view.auto_follow;
        resume.current_search.results_visible = tab.results_visible;
        resume.current_search.selected_source_row = selected_result_source_row;
        resume.current_search.selected_result_ix = selected_result_ix;
        resume.current_search.viewport =
            Self::capture_persisted_local_viewport(tab, WrappedRegion::Results, row_height, cx)
                .or(resume.current_search.viewport);
        resume.active_region = match tab.view.selection_table {
            SelectionTable::Log => PersistedLogRegion::Body,
            SelectionTable::Results => PersistedLogRegion::CurrentResults,
        };
        FileSessionState {
            revision: tab.session_base.revision,
            custom_title: tab.file.custom_title.clone(),
            selected_row,
            query_text: tab.search_query.text.clone(),
            result_mode: tab.result_mode.database_value(),
            marked_rows,
            show_line_numbers: tab.view.show_line_numbers,
            show_row_separators: tab.view.show_row_separators,
            word_wrap: tab.view.word_wrap,
            keyword_color_rules: tab.file.keyword_color_rules.clone(),
            resume,
        }
    }

    pub(super) fn take_quit_snapshot(&mut self, cx: &mut Context<Self>) -> QuitWorkspaceSnapshot {
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

    pub(super) fn schedule_checkpoint(
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

    pub(super) fn schedule_log_region_state_save(
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
    pub(super) fn install_documents(
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
        let mut restored_session_document_ids = BTreeSet::new();
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
            // A zero-result directory header is navigation, not an invitation to publish the
            // empty opening shell. Keep the source surface until the target's first frame exists.
            let is_pending_directory_group_target = self
                .pending_directory_group_activation
                .as_deref()
                .is_some_and(|pending_path| paths_match(pending_path, &path));
            let defer_directory_group_activation = should_defer_directory_group_activation(
                self.pending_directory_group_activation.as_deref(),
                &path,
                prepared.load_state,
            );
            let prepared_load_state = prepared.load_state;

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
                let target_frame_is_prepared = if should_upgrade {
                    let restores_session =
                        current_state == DocumentLoadState::Opening && prepared.session.is_some();
                    let document_id = self.documents[existing_ix].id;
                    let ready = prepared.load_state == DocumentLoadState::Ready;
                    let prepared_frame_installed =
                        self.upgrade_loading_document(existing_ix, prepared, window, cx);
                    if restores_session {
                        restored_session_document_ids.insert(document_id);
                    }
                    global_sources_changed = true;
                    if ready && !path_match_set_contains(&self.transient_paths, &path) {
                        recorded_paths.push(path.clone());
                    }
                    prepared_frame_installed
                } else {
                    false
                };
                if self.active_ix != Some(existing_ix) && !defer_directory_group_activation {
                    if is_pending_directory_group_target
                        && prepared_load_state == DocumentLoadState::Ready
                    {
                        let tab_id = WorkspaceTabId::Document(self.documents[existing_ix].id);
                        if target_frame_is_prepared {
                            self.commit_workspace_tab_activation(tab_id, true, window, cx);
                        } else {
                            self.activate_workspace_tab(tab_id, window, cx);
                        }
                    } else {
                        self.activate_tab(existing_ix, window, cx);
                    }
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
                case_sensitive: prepared.search_case_sensitive,
                regex: prepared.search_regex,
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
            let log_table = cx.new(|_| {
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
                VirtualLogListState::new(delegate, VirtualLogViewport::new())
            });
            let result_table = cx.new(|_| {
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
                VirtualLogListState::new(delegate, VirtualLogViewport::new())
            });
            let result_mode_select = cx.new(|cx| {
                SelectState::new(
                    ResultMode::ALL.to_vec(),
                    Some(IndexPath::new(result_mode.select_index())),
                    window,
                    cx,
                )
            });

            let log_subscription = cx.subscribe_in(
                &log_table,
                window,
                move |this, table, event: &VirtualLogListEvent, window, cx| {
                    let keep_quick_find_focus = this.quick_find_input_has_focus(window, cx);
                    let source_row = match event {
                        VirtualLogListEvent::SelectRow(row_ix) => {
                            table.read(cx).delegate().settle_table_selection(*row_ix)
                        }
                        VirtualLogListEvent::ClearSelection => {
                            if table.read(cx).delegate().take_suppressed_table_clear() {
                                return;
                            }
                            table.read(cx).delegate().clear_row_selection();
                            table.read(cx).delegate().set_active_log_row(None);
                            None
                        }
                    };
                    this.selected_source_row = source_row;
                    this.active_log_region = LogRegion::Body;
                    if !keep_quick_find_focus {
                        this.log_viewer.focus_handle.focus(window, cx);
                    }
                    if let Some(tab) = this.documents.iter_mut().find(|tab| tab.id == document_id) {
                        tab.view.pending_restore_row = None;
                        tab.view.selection_table = SelectionTable::Log;
                        if source_row.is_some_and(|row| row + 1 < tab.document.source_line_count())
                        {
                            tab.view.auto_follow = false;
                        }
                    }
                    this.schedule_checkpoint(document_id, window, cx);
                    cx.notify();
                },
            );
            let result_subscription = cx.subscribe_in(
                &result_table,
                window,
                move |this, table, event: &VirtualLogListEvent, window, cx| {
                    let keep_quick_find_focus = this.quick_find_input_has_focus(window, cx);
                    let result_ix = match event {
                        VirtualLogListEvent::SelectRow(result_ix) => *result_ix,
                        VirtualLogListEvent::ClearSelection => {
                            if table.read(cx).delegate().take_suppressed_table_clear() {
                                return;
                            }
                            table.read(cx).delegate().clear_row_selection();
                            table.read(cx).delegate().set_active_log_row(None);
                            this.schedule_checkpoint(document_id, window, cx);
                            return;
                        }
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
                    this.documents[tab_ix].view.auto_follow = false;
                    if !this.select_and_center_log_source_row_atomically(
                        document_id,
                        source_row,
                        window,
                        cx,
                    ) {
                        return;
                    }
                    if !keep_quick_find_focus {
                        this.search_results_viewer.focus_handle.focus(window, cx);
                    }
                    this.documents[tab_ix].view.selection_table = SelectionTable::Results;
                    this.active_log_region = LogRegion::CurrentResults;
                    this.selected_source_row = Some(source_row);
                    this.schedule_checkpoint(document_id, window, cx);
                    cx.notify();
                },
            );
            let result_mode_subscription = cx.subscribe_in(
                &result_mode_select,
                window,
                move |this, _, event: &SelectEvent<Vec<ResultMode>>, window, cx| {
                    let SelectEvent::Confirm(Some(mode)) = event else {
                        return;
                    };
                    {
                        let Some(tab) = this.documents.iter_mut().find(|tab| tab.id == document_id)
                        else {
                            return;
                        };
                        if tab.result_mode == *mode {
                            return;
                        }
                        tab.result_mode = *mode;
                        if mode.includes_marks() && !tab.file.marked_rows.is_empty() {
                            tab.results_visible = true;
                        }
                    }
                    this.refresh_document_result_rows_atomically(document_id, window, cx);
                    if this
                        .active_document()
                        .is_some_and(|tab| tab.id == document_id)
                    {
                        this.refresh_active_document_surfaces_atomically(window, cx);
                    }
                    this.schedule_checkpoint(document_id, window, cx);
                    cx.notify();
                },
            );

            let results_visible = restored_results_visible(
                session.resume.current_search.results_visible,
                result_mode,
                !marked_rows.is_empty(),
            );
            let pending_log_jump = resolved_prepared_search_jump(
                None,
                self.pending_search_result_jump.as_ref(),
                &path,
                &document,
            );
            let pending_restore_row =
                opening_restore_source_row(pending_log_jump, session.selected_row);
            if let Some(selected_row) = pending_restore_row.and_then(|row| document.local_row(row))
            {
                log_table.update(cx, |table, cx| table.set_active_log_row(selected_row, cx));
            }
            let title: SharedString = custom_title
                .clone()
                .unwrap_or_else(|| document.file_name())
                .into();
            let log_viewport = {
                let table = log_table.read(cx);
                LogViewportState::new(
                    session.word_wrap,
                    table.viewport().clone(),
                    table.delegate().row_bounds_handle(),
                )
            };
            let result_viewport = {
                let table = result_table.read(cx);
                LogViewportState::new(
                    session.word_wrap,
                    table.viewport().clone(),
                    table.delegate().row_bounds_handle(),
                )
            };
            self.documents.push(DocumentTab {
                id: document_id,
                opened_at: Local::now().timestamp(),
                file: FileState {
                    title,
                    custom_title,
                    marked_rows,
                    pending_restore_marked_rows: if prepared.load_state != DocumentLoadState::Ready
                    {
                        restored_marked_rows
                    } else {
                        CompressedRows::default()
                    },
                    keyword_color_rules,
                    resolved_color_rules,
                },
                view: FileViewState {
                    auto_follow: false,
                    show_line_numbers: session.show_line_numbers,
                    show_row_separators: session.show_row_separators,
                    word_wrap: session.word_wrap,
                    selection_table: restored_selection_table(
                        session.resume.active_region,
                        results_visible,
                    ),
                    uses_default_view_options,
                    pending_restore_row: (prepared.load_state != DocumentLoadState::Ready)
                        .then_some(pending_restore_row)
                        .flatten(),
                    pending_resume,
                },
                document,
                session_base,
                log_table,
                result_table,
                log_viewport,
                result_viewport,
                search_query,
                search_result: prepared.search_result,
                search_matcher: prepared.search_matcher,
                result_mode,
                result_mode_select,
                _subscriptions: [
                    log_subscription,
                    result_subscription,
                    result_mode_subscription,
                ],
                search_revision: 0,
                result_replace_revision: 0,
                result_replace_task: None,
                result_replace_cancellation: None,
                results_visible,
                restoring_result_selection: false,
                load_state: prepared.load_state,
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
            if !defer_directory_group_activation {
                self.active_tab_id = workspace_tab_id;
            }
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
        let active_id = self.active_document().map(|tab| tab.id);
        let active_session_was_restored = active_id
            .is_some_and(|document_id| restored_session_document_ids.contains(&document_id));
        if should_sync_active_document_controls(
            final_phase,
            previous_active_id,
            active_id,
            active_session_was_restored,
        ) {
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

    pub(super) fn install_completed_documents(
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
        self.complete_pending_search_result_jump(window, cx);
        self.pending_directory_group_activation = None;
        self.persist_workspace_order(window, cx);
    }

    pub(super) fn apply_tab_resume(&mut self, document_ix: usize, cx: &mut Context<Self>) {
        if self
            .documents
            .get(document_ix)
            .is_none_or(|tab| tab.load_state != DocumentLoadState::Ready)
        {
            return;
        }
        let row_height = self.log_row_height();
        let resume = self.documents[document_ix]
            .view
            .pending_resume
            .take()
            .unwrap_or_else(|| self.documents[document_ix].session_base.resume.clone());
        {
            let tab = &mut self.documents[document_ix];
            tab.view.auto_follow = resume.viewer.auto_follow;
            tab.results_visible = restored_results_visible(
                resume.current_search.results_visible,
                tab.result_mode,
                !tab.file.marked_rows.is_empty(),
            );
            tab.view.selection_table =
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

    pub(super) fn complete_pending_search_result_jump(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(pending) = self.pending_search_result_jump.clone() else {
            return;
        };
        let Some(document_ix) = self
            .documents
            .iter()
            .position(|tab| paths_match(tab.document.path(), &pending.path))
        else {
            self.pending_search_result_jump = None;
            return;
        };
        // Preview is only an intermediate opening state. Keep the foreground result jump alive
        // until the same tab reaches Ready; otherwise its persisted row wins during the upgrade.
        if should_defer_search_result_jump(Some(self.documents[document_ix].load_state)) {
            return;
        }
        self.pending_search_result_jump = None;
        if !pending.matches(&self.documents[document_ix].document) {
            Self::notify_stale_search_result(window, cx);
            return;
        }
        // The newly opened tab was built from its file-owned search session. Install the active
        // workspace search's matcher and hit rows before selecting the target so the first
        // committed frame is already centered and highlighted like the result row that opened it.
        self.refresh_active_log_search_presentation(cx);
        if !self.activate_document_log_row_atomically(document_ix, pending.source_row, window, cx) {
            window.push_notification(
                crate::tr!(
                    "该搜索结果行在当前文件中已不存在，请重新搜索",
                    "That search result line no longer exists in the current file. Search again."
                ),
                cx,
            );
        }
    }

    pub(super) fn prepare_document_upgrade_jobs(
        &self,
        opened: &[(PathBuf, Result<PreparedDocument>)],
        window: &Window,
        cx: &App,
    ) -> Vec<DocumentUpgradeLoadJob> {
        let row_height = self.log_row_height();
        let window_visible_rows = (window.viewport_size().height / row_height.max(px(1.)))
            .ceil()
            .max(1.) as usize;
        opened
            .iter()
            .filter_map(|(path, prepared)| {
                let prepared = prepared.as_ref().ok()?;
                let tab = self.documents.iter().find(|tab| {
                    paths_match(tab.document.path(), path)
                        && tab.load_state != DocumentLoadState::Ready
                })?;
                if !should_upgrade_loading_document(
                    tab.load_state,
                    prepared.load_state,
                    prepared.session.is_some(),
                ) {
                    return None;
                }
                let mut marked_rows = tab.file.marked_rows.clone();
                marked_rows.extend(tab.file.pending_restore_marked_rows.iter());
                marked_rows.retain_below(prepared.document.source_line_count());
                let result_rows = compute_result_rows(
                    tab.result_mode,
                    Some(&prepared.search_result),
                    &marked_rows,
                );
                let log_anchor =
                    Self::capture_local_viewport_anchor(tab, WrappedRegion::Log, row_height, cx);
                let result_anchor = Self::capture_local_viewport_anchor(
                    tab,
                    WrappedRegion::Results,
                    row_height,
                    cx,
                );
                let log_jump = prepared_pending_search_jump(
                    self.pending_search_result_jump.as_ref(),
                    path,
                    &prepared.document,
                );
                let selected_source_row = log_jump.map(|jump| jump.source_row).or_else(|| {
                    tab.log_table
                        .read(cx)
                        .active_log_row()
                        .and_then(|row_ix| tab.log_table.read(cx).delegate().source_row(row_ix))
                        .or(tab.view.pending_restore_row)
                });
                let log_anchor_ix = log_jump
                    .map(|jump| jump.row_ix)
                    .or_else(|| {
                        log_anchor.as_ref().and_then(|anchor| match anchor.key {
                            LogRowKey::Row { source_row, .. } => {
                                prepared.document.local_row(source_row)
                            }
                            LogRowKey::FileGroup { .. } => None,
                        })
                    })
                    .or_else(|| {
                        selected_source_row.and_then(|row| prepared.document.local_row(row))
                    })
                    .or_else(|| log_anchor.as_ref().map(|anchor| anchor.fallback_ix))
                    .unwrap_or_default();
                let result_anchor_ix = result_anchor
                    .as_ref()
                    .and_then(|anchor| match anchor.key {
                        LogRowKey::Row { source_row, .. } => result_rows.position(source_row),
                        LogRowKey::FileGroup { .. } => None,
                    })
                    .or_else(|| selected_source_row.and_then(|row| result_rows.position(row)))
                    .or_else(|| result_anchor.as_ref().map(|anchor| anchor.fallback_ix))
                    .unwrap_or_default();
                let log_visible_rows = tab
                    .log_table
                    .read(cx)
                    .visible_range()
                    .rows()
                    .len()
                    .max(window_visible_rows);
                let log_range = if log_jump.is_some() {
                    centered_log_jump_preload_range(
                        log_anchor_ix,
                        prepared.document.line_count(),
                        log_visible_rows,
                    )
                } else {
                    search_scope_switch_preload_range(
                        log_anchor_ix,
                        log_anchor.as_ref().is_some_and(|anchor| anchor.at_end),
                        prepared.document.line_count(),
                        log_visible_rows,
                    )
                };
                let result_range = search_scope_switch_preload_range(
                    result_anchor_ix,
                    result_anchor.as_ref().is_some_and(|anchor| anchor.at_end),
                    result_rows.len(),
                    tab.result_table
                        .read(cx)
                        .visible_range()
                        .rows()
                        .len()
                        .max(window_visible_rows),
                );
                let log_word_wrap = tab.log_viewport.is_wrapped();
                let result_word_wrap = tab.result_viewport.is_wrapped();
                let log_measured_heights = if log_word_wrap {
                    let table = tab.log_table.read(cx);
                    tab.log_viewport
                        .wrapped_measured_heights_by_key(|row_ix| table.delegate().row_key(row_ix))
                } else {
                    BTreeMap::new()
                };
                let result_measured_heights = if result_word_wrap {
                    let table = tab.result_table.read(cx);
                    tab.result_viewport
                        .wrapped_measured_heights_by_key(|row_ix| table.delegate().row_key(row_ix))
                } else {
                    BTreeMap::new()
                };
                Some(DocumentUpgradeLoadJob {
                    path: path.clone(),
                    previous_document: tab.document.clone(),
                    document: prepared.document.clone(),
                    result_rows: result_rows.clone(),
                    log_request: tab
                        .log_table
                        .read(cx)
                        .delegate()
                        .stage_document_replacement(&prepared.document, None, log_range),
                    result_request: tab
                        .result_table
                        .read(cx)
                        .delegate()
                        .stage_document_replacement(
                            &prepared.document,
                            Some(&result_rows),
                            result_range,
                        ),
                    log_anchor,
                    result_anchor,
                    log_measured_heights,
                    result_measured_heights,
                    row_height,
                    log_word_wrap,
                    result_word_wrap,
                    log_jump,
                })
            })
            .collect()
    }

    pub(super) fn attach_document_upgrade_frames(
        opened: &mut [(PathBuf, Result<PreparedDocument>)],
        frames: Vec<PreparedDocumentUpgradeFrame>,
    ) {
        for frame in frames {
            let Some((_, prepared)) = opened.iter_mut().find(|(path, prepared)| {
                paths_match(path, &frame.path)
                    && prepared
                        .as_ref()
                        .is_ok_and(|prepared| Arc::ptr_eq(&prepared.document, &frame.document))
            }) else {
                continue;
            };
            if let Ok(prepared) = prepared {
                prepared.upgrade_frame = Some(frame);
            }
        }
    }

    pub(super) fn upgrade_loading_document(
        &mut self,
        document_ix: usize,
        mut prepared: PreparedDocument,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let upgrade_frame = prepared.upgrade_frame.take();
        let pending_log_jump = resolved_prepared_search_jump(
            upgrade_frame.as_ref().and_then(|frame| frame.log_jump),
            self.pending_search_result_jump.as_ref(),
            prepared.document.path(),
            &prepared.document,
        );
        let highlight_matches = self.app_settings.highlight_matches;
        let tab = &mut self.documents[document_ix];
        let previous_document = tab.document.clone();
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
            tab.file.custom_title = session
                .custom_title
                .as_deref()
                .map(str::trim)
                .filter(|title| !title.is_empty())
                .map(str::to_owned);
            tab.file.title = tab
                .file
                .custom_title
                .clone()
                .unwrap_or_else(|| prepared.document.file_name())
                .into();
            tab.search_query = SearchQuery {
                text: session.query_text.clone(),
                case_sensitive: prepared.search_case_sensitive,
                regex: prepared.search_regex,
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
            tab.file.pending_restore_marked_rows = session.marked_rows.clone();
            // Opening from a workspace-search result is an explicit foreground navigation.
            // Its selected result row outranks the file session's last closed row from the first
            // restored frame onward; ordinary opens still fall back to the persisted row.
            tab.view.pending_restore_row =
                opening_restore_source_row(pending_log_jump, session.selected_row);
            tab.view.pending_resume = Some(session.resume.clone());
            tab.file.keyword_color_rules = session.keyword_color_rules.clone();
            tab.file.resolved_color_rules = installable_color_rules(
                prepared.color_labels_snapshot.as_deref(),
                prepared.resolved_color_rules.clone(),
                &tab.file.keyword_color_rules,
                &self.color_labels,
            );
            tab.view.show_line_numbers = session.show_line_numbers;
            tab.view.show_row_separators = session.show_row_separators;
            tab.log_viewport.set_word_wrap(session.word_wrap);
            tab.result_viewport.set_word_wrap(session.word_wrap);
            tab.view.word_wrap = session.word_wrap;
            tab.view.uses_default_view_options = false;
            tab.results_visible = restored_results_visible(
                session.resume.current_search.results_visible,
                tab.result_mode,
                !tab.file.pending_restore_marked_rows.is_empty(),
            );
            tab.view.selection_table =
                restored_selection_table(session.resume.active_region, tab.results_visible);
            tab.refresh_view_options(cx);
            for table in [tab.log_table.clone(), tab.result_table.clone()] {
                table.update(cx, |table, cx| {
                    table
                        .delegate_mut()
                        .set_color_rules(tab.file.resolved_color_rules.clone());
                    table.refresh(cx);
                });
            }
        }
        tab.document = prepared.document;
        tab.search_result = prepared.search_result;
        tab.search_matcher = prepared.search_matcher;
        if prepared.load_state == DocumentLoadState::Ready {
            let pending_marks = std::mem::take(&mut tab.file.pending_restore_marked_rows);
            tab.file.marked_rows.extend(pending_marks.iter());
            tab.file
                .marked_rows
                .retain_below(tab.document.source_line_count());
        } else {
            tab.file.marked_rows = tab
                .file
                .pending_restore_marked_rows
                .iter()
                .filter(|row| tab.document.contains_source_row(*row))
                .collect();
        }
        let result_rows = tab.compute_result_rows();
        let upgrade_frame = upgrade_frame.filter(|frame| {
            Arc::ptr_eq(&frame.previous_document, &previous_document)
                && Arc::ptr_eq(&frame.document, &tab.document)
                && frame.result_rows == result_rows
        });
        let prepared_frame_installed = upgrade_frame.is_some();

        let marked_rows = tab.file.marked_rows.clone();
        tab.log_table.update(cx, |table, cx| {
            if let Some(frame) = upgrade_frame.as_ref() {
                table.delegate_mut().install_document_replacement(
                    tab.document.clone(),
                    None,
                    frame.log_lines.clone(),
                );
            } else {
                table.delegate_mut().replace_with_all(tab.document.clone());
            }
            table.delegate_mut().set_marked_rows(marked_rows.clone());
            table
                .delegate_mut()
                .set_matched_rows(tab.search_result.line_indices.clone());
            table.delegate_mut().set_search_matcher(
                highlight_matches
                    .then(|| tab.search_matcher.clone())
                    .flatten(),
            );
            if upgrade_frame.is_none() {
                table.refresh_log_rows(cx);
            } else {
                table.refresh(cx);
                cx.notify();
            }
        });
        tab.result_table.update(cx, |table, cx| {
            if let Some(frame) = upgrade_frame.as_ref() {
                table.delegate_mut().install_document_replacement(
                    tab.document.clone(),
                    Some(result_rows.clone()),
                    frame.result_lines.clone(),
                );
            } else {
                table
                    .delegate_mut()
                    .replace_with_rows(tab.document.clone(), result_rows.clone());
            }
            table.delegate_mut().set_marked_rows(marked_rows);
            table
                .delegate_mut()
                .set_matched_rows(tab.search_result.line_indices.clone());
            table.delegate_mut().set_search_matcher(
                highlight_matches
                    .then(|| tab.search_matcher.clone())
                    .flatten(),
            );
            if upgrade_frame.is_none() {
                table.refresh_log_rows(cx);
            } else {
                table.refresh(cx);
                cx.notify();
            }
        });

        if let Some(frame) = upgrade_frame.as_ref() {
            if frame.log_word_wrap {
                let table = tab.log_table.read(cx);
                tab.log_viewport.reset_wrapped_with_remapped_heights(
                    table.delegate().row_count(),
                    frame.row_height,
                    frame.log_measured_heights.clone(),
                    |key| table.delegate().row_ix_for_key(*key),
                );
            } else {
                tab.log_viewport.invalidate_wrapped();
            }
            if frame.result_word_wrap {
                let table = tab.result_table.read(cx);
                tab.result_viewport.reset_wrapped_with_remapped_heights(
                    table.delegate().row_count(),
                    frame.row_height,
                    frame.result_measured_heights.clone(),
                    |key| table.delegate().row_ix_for_key(*key),
                );
            } else {
                tab.result_viewport.invalidate_wrapped();
            }
            Self::restore_local_viewport_anchor(
                tab,
                WrappedRegion::Log,
                frame.log_anchor,
                frame.row_height,
                cx,
            );
            Self::restore_local_viewport_anchor(
                tab,
                WrappedRegion::Results,
                frame.result_anchor,
                frame.row_height,
                cx,
            );
        } else {
            tab.log_viewport.invalidate_wrapped();
            tab.result_viewport.invalidate_wrapped();
        }

        let restore_row = if prepared.load_state == DocumentLoadState::Ready {
            tab.view.pending_restore_row.take().or(selected_source_row)
        } else {
            tab.view.pending_restore_row.or(selected_source_row)
        };
        if let Some(row) = restore_row.and_then(|row| tab.document.local_row(row)) {
            tab.log_table
                .update(cx, |table, cx| table.set_active_log_row(row, cx));
        }
        tab.load_state = prepared.load_state;
        if prepared.load_state == DocumentLoadState::Ready {
            self.apply_tab_resume(document_ix, cx);
        }
        if let Some(log_jump) = pending_log_jump {
            // A result click owns the opening target. Commit it after the file session resume even
            // when visible-line staging was skipped or discarded, so a small/cached file cannot
            // fall back to its previously persisted selection.
            self.commit_prepared_log_jump(document_ix, log_jump, cx);
        }
        if self.active_ix == Some(document_ix) {
            if upgrade_frame.is_some() {
                self.refresh_prepared_active_document_surfaces_atomically(window, cx);
            } else {
                self.refresh_active_document_surfaces_atomically(window, cx);
            }
        }
        prepared_frame_installed
    }

    pub(super) fn reload_active(
        &mut self,
        _: &ReloadActive,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(active_ix) = self.active_ix else {
            return;
        };
        let document_id = self.documents[active_ix].id;
        self.reload_document(document_id, ReloadStrategy::Full, window, cx);
    }

    fn prepare_reload_replacement(
        &mut self,
        input: ReloadReplacementInput,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<ReloadReplacementPlan> {
        let ReloadReplacementInput {
            document_id,
            global_revision,
            global_search,
            revision,
            previous_document,
            document,
            search_result,
            query,
            search_matcher,
            results_visible,
            selected_source_row,
        } = input;
        let tab_ix = self
            .documents
            .iter()
            .position(|tab| tab.id == document_id)?;
        let tab = &self.documents[tab_ix];
        if tab.search_revision != revision
            || !Arc::ptr_eq(&tab.document, &previous_document)
            || self.global_search.revision != global_revision
            || self.searches.is_active()
        {
            return None;
        }
        let mut marked_rows = tab.file.marked_rows.clone();
        marked_rows.extend(tab.file.pending_restore_marked_rows.iter());
        marked_rows.retain_below(document.source_line_count());
        let result_rows = compute_result_rows(tab.result_mode, Some(&search_result), &marked_rows);
        let row_height = self.log_row_height();
        let log_word_wrap = tab.log_viewport.is_wrapped();
        let result_word_wrap = tab.result_viewport.is_wrapped();
        let follow_end = tab.view.auto_follow;
        let mut log_anchor =
            Self::capture_local_viewport_anchor(tab, WrappedRegion::Log, row_height, cx);
        if !follow_end && let Some(anchor) = log_anchor.as_mut() {
            anchor.at_end = false;
        }
        let result_anchor =
            Self::capture_local_viewport_anchor(tab, WrappedRegion::Results, row_height, cx);
        let window_visible_rows = (window.viewport_size().height / row_height.max(px(1.)))
            .ceil()
            .max(1.) as usize;
        let log_anchor_ix = if follow_end {
            document.line_count().saturating_sub(1)
        } else {
            log_anchor
                .as_ref()
                .and_then(|anchor| match anchor.key {
                    LogRowKey::Row { source_row, .. } => document.local_row(source_row),
                    LogRowKey::FileGroup { .. } => None,
                })
                .or_else(|| selected_source_row.and_then(|row| document.local_row(row)))
                .or_else(|| log_anchor.as_ref().map(|anchor| anchor.fallback_ix))
                .unwrap_or_default()
        };
        let log_visible_rows = tab.log_table.read(cx).visible_range().rows().len();
        let log_range = search_scope_switch_preload_range(
            log_anchor_ix,
            follow_end || log_anchor.as_ref().is_some_and(|anchor| anchor.at_end),
            document.line_count(),
            log_visible_rows.max(window_visible_rows),
        );
        let result_anchor_ix = result_anchor
            .as_ref()
            .and_then(|anchor| match anchor.key {
                LogRowKey::Row { source_row, .. } => result_rows.position(source_row),
                LogRowKey::FileGroup { .. } => None,
            })
            .or_else(|| selected_source_row.and_then(|source_row| result_rows.position(source_row)))
            .or_else(|| result_anchor.as_ref().map(|anchor| anchor.fallback_ix))
            .unwrap_or_default();
        let result_visible_rows = tab.result_table.read(cx).visible_range().rows().len();
        let result_range = search_scope_switch_preload_range(
            result_anchor_ix,
            result_anchor.as_ref().is_some_and(|anchor| anchor.at_end),
            result_rows.len(),
            result_visible_rows.max(window_visible_rows),
        );
        let log_request = tab
            .log_table
            .read(cx)
            .delegate()
            .stage_document_replacement(&document, None, log_range);
        let result_request = tab
            .result_table
            .read(cx)
            .delegate()
            .stage_document_replacement(&document, Some(&result_rows), result_range);
        Some(ReloadReplacementPlan {
            document_id,
            global_revision,
            global_search,
            revision,
            previous_document,
            document,
            search_result,
            query,
            search_matcher,
            marked_rows,
            result_rows,
            results_visible,
            follow_end,
            selected_source_row,
            log_request: Some(log_request),
            result_request: Some(result_request),
            log_anchor,
            result_anchor,
            row_height,
            log_word_wrap,
            result_word_wrap,
        })
    }

    fn finish_reload(&mut self, strategy: ReloadStrategy) {
        match strategy {
            ReloadStrategy::Full => {
                self.open_task = None;
                if matches!(self.activity, Activity::Opening) {
                    self.activity = Activity::Ready;
                }
            }
            ReloadStrategy::ExtendAppend => self.file_refresh_task = None,
        }
    }

    fn commit_reload_replacement(
        &mut self,
        prepared: PreparedReloadReplacement,
        strategy: ReloadStrategy,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let plan = prepared.plan;
        let Some(tab_ix) = self
            .documents
            .iter()
            .position(|tab| tab.id == plan.document_id)
        else {
            self.finish_reload(strategy);
            self.open_queued_external_paths_if_idle(window, cx);
            cx.notify();
            return;
        };
        if self.documents[tab_ix].search_revision != plan.revision
            || (matches!(strategy, ReloadStrategy::ExtendAppend)
                && (!window.is_window_active()
                    || self.active_tab_id != WorkspaceTabId::Document(plan.document_id)))
            || self.global_search.revision != plan.global_revision
            || self.searches.is_active()
            || plan.follow_end != self.documents[tab_ix].view.auto_follow
            || prepared.log_lines.has_unavailable_lines()
            || prepared.result_lines.has_unavailable_lines()
            || !Arc::ptr_eq(&self.documents[tab_ix].document, &plan.previous_document)
        {
            self.finish_reload(strategy);
            self.open_queued_external_paths_if_idle(window, cx);
            cx.notify();
            return;
        }
        let highlight_matches = self.app_settings.highlight_matches;
        let tab = &mut self.documents[tab_ix];
        if let Some(cancellation) = tab.result_replace_cancellation.take() {
            cancellation.store(true, Ordering::Release);
        }
        tab.result_replace_task.take();
        tab.result_replace_revision = tab.result_replace_revision.saturating_add(1);
        tab.document = plan.document;
        tab.search_query = plan.query;
        tab.search_result = plan.search_result;
        tab.search_matcher = plan.search_matcher;
        tab.file.pending_restore_marked_rows = CompressedRows::default();
        tab.file.marked_rows = plan.marked_rows;
        tab.log_table.update(cx, |table, cx| {
            table.delegate_mut().install_document_replacement(
                tab.document.clone(),
                None,
                prepared.log_lines,
            );
            table
                .delegate_mut()
                .set_marked_rows(tab.file.marked_rows.clone());
            table
                .delegate_mut()
                .set_matched_rows(tab.search_result.line_indices.clone());
            table.delegate_mut().set_search_matcher(
                highlight_matches
                    .then(|| tab.search_matcher.clone())
                    .flatten(),
            );
            if tab.document.line_count() > 0
                && (plan.follow_end || table.active_log_row().is_none())
            {
                let row = if plan.follow_end {
                    tab.document.line_count() - 1
                } else {
                    plan.selected_source_row
                        .unwrap_or_default()
                        .min(tab.document.line_count() - 1)
                };
                table.delegate().set_active_log_row(Some(row));
                table.delegate().settle_table_selection(row);
            } else if tab.document.line_count() == 0 {
                table.delegate().set_active_log_row(None);
                table.delegate().clear_row_selection();
            }
            table.refresh(cx);
            cx.notify();
        });
        tab.result_table.update(cx, |table, cx| {
            table.delegate_mut().install_document_replacement(
                tab.document.clone(),
                Some(plan.result_rows),
                prepared.result_lines,
            );
            table
                .delegate_mut()
                .set_marked_rows(tab.file.marked_rows.clone());
            table
                .delegate_mut()
                .set_matched_rows(tab.search_result.line_indices.clone());
            table.delegate_mut().set_search_matcher(
                highlight_matches
                    .then(|| tab.search_matcher.clone())
                    .flatten(),
            );
            // Source replacement already preserves result selection by source row.
            table.refresh(cx);
            cx.notify();
        });
        if plan.log_word_wrap {
            let table = tab.log_table.read(cx);
            tab.log_viewport.reset_wrapped_with_remapped_heights(
                table.delegate().row_count(),
                plan.row_height,
                BTreeMap::new(),
                |key| table.delegate().row_ix_for_key(*key),
            );
        } else {
            tab.log_viewport.invalidate_wrapped();
        }
        if plan.result_word_wrap {
            let table = tab.result_table.read(cx);
            tab.result_viewport.reset_wrapped_with_remapped_heights(
                table.delegate().row_count(),
                plan.row_height,
                BTreeMap::new(),
                |key| table.delegate().row_ix_for_key(*key),
            );
        } else {
            tab.result_viewport.invalidate_wrapped();
        }
        if plan.follow_end {
            tab.log_viewport.scroll_to_end();
        } else {
            Self::restore_local_viewport_anchor(
                tab,
                WrappedRegion::Log,
                plan.log_anchor,
                plan.row_height,
                cx,
            );
        }
        Self::restore_local_viewport_anchor(
            tab,
            WrappedRegion::Results,
            plan.result_anchor,
            plan.row_height,
            cx,
        );
        tab.results_visible = plan.results_visible;
        tab.load_state = DocumentLoadState::Ready;
        tab.view.pending_restore_row = None;
        if self
            .active_document()
            .is_some_and(|tab| tab.id == plan.document_id)
        {
            self.selected_source_row = self.documents[tab_ix].log_table.read(cx).active_log_row();
            self.refresh_prepared_active_document_surfaces_atomically(window, cx);
        }
        if let Some(global) = plan.global_search {
            self.install_reloaded_all_open_result(plan.document_id, global);
            self.schedule_workspace_search_state_save(window, cx);
        }
        self.refresh_global_result_rows(window, cx);
        self.refresh_active_log_search_presentation(cx);
        self.finish_reload(strategy);
        self.open_queued_external_paths_if_idle(window, cx);
        cx.notify();
    }

    pub(super) fn reload_document(
        &mut self,
        document_id: u64,
        strategy: ReloadStrategy,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.open_task.is_some() {
            return false;
        }
        if matches!(strategy, ReloadStrategy::ExtendAppend)
            && (self.file_refresh_task.is_some()
                || self.searches.is_active()
                || !window.is_window_active()
                || self.active_tab_id != WorkspaceTabId::Document(document_id))
        {
            return false;
        }
        let Some(document_ix) = self.documents.iter().position(|tab| tab.id == document_id) else {
            return false;
        };
        // Dropping the automatic task prevents it from publishing over a manual reload.
        self.file_refresh_task.take();
        self.cancel_search_for(document_id);
        let global_revision = self.global_search.revision;
        let global_search = self.all_open_result_for_reload(document_id);
        let tab = &mut self.documents[document_ix];
        tab.search_revision += 1;
        let revision = tab.search_revision;
        let previous_document = tab.document.clone();
        let query = tab.search_query.clone();
        let previous_result = tab.search_result.clone();
        let previous_search_complete = query.text.is_empty() || tab.search_matcher.is_some();
        let results_visible = tab.results_visible;
        let selected_source_row = {
            let table = tab.log_table.read(cx);
            table
                .active_log_row()
                .and_then(|row_ix| table.delegate().source_row(row_ix))
        };
        if matches!(strategy, ReloadStrategy::Full) {
            self.activity = Activity::Opening;
            cx.notify();
        }

        let task = cx.spawn_in(window, async move |this, cx| {
            let reload_source = previous_document.clone();
            let result = cx
                .background_spawn(async move {
                    let (document, refresh_kind) = match strategy {
                        ReloadStrategy::Full => (
                            LogDocument::open(reload_source.path())?,
                            DocumentRefreshKind::Rebuilt,
                        ),
                        ReloadStrategy::ExtendAppend => reload_source.refresh()?,
                    };
                    let document = Arc::new(document);
                    let search_matcher = SearchMatcher::new(&query)?;
                    let search_result = search_reloaded_document(
                        &document,
                        &reload_source,
                        if previous_search_complete {
                            refresh_kind
                        } else {
                            DocumentRefreshKind::Rebuilt
                        },
                        &previous_result,
                        &query,
                        search_matcher.as_ref(),
                    )?;
                    let global_search = global_search
                        .map(|mut global| -> Result<_> {
                            let kind = if global.completed
                                && global.document.same_source_snapshot(&reload_source)
                            {
                                refresh_kind
                            } else {
                                DocumentRefreshKind::Rebuilt
                            };
                            global.result = search_reloaded_document(
                                &document,
                                &global.document,
                                kind,
                                &global.result,
                                &global.query,
                                global.matcher.as_ref(),
                            )?;
                            global.document = document.clone();
                            Ok(global)
                        })
                        .transpose()?;
                    Ok::<_, anyhow::Error>((
                        document,
                        search_result,
                        query,
                        search_matcher,
                        global_search,
                    ))
                })
                .await;
            let (document, search_result, query, search_matcher, global_search) = match result {
                Ok(prepared) => prepared,
                Err(error) => {
                    _ = this.update_in(cx, |this, window, cx| {
                        // A rotation can temporarily remove/lock the path. Keep the last
                        // frame and follow preference; monitoring retries on the next round.
                        if matches!(strategy, ReloadStrategy::Full) {
                            let message: SharedString = error.to_string().into();
                            window.push_notification(message, cx);
                            this.activity = Activity::Error;
                        }
                        this.finish_reload(strategy);
                        this.open_queued_external_paths_if_idle(window, cx);
                        cx.notify();
                    });
                    return;
                }
            };

            let plan = this
                .update_in(cx, |this, window, cx| {
                    if matches!(strategy, ReloadStrategy::ExtendAppend)
                        && (!window.is_window_active()
                            || this.active_tab_id != WorkspaceTabId::Document(document_id))
                    {
                        return None;
                    }
                    this.prepare_reload_replacement(
                        ReloadReplacementInput {
                            document_id,
                            global_revision,
                            global_search,
                            revision,
                            previous_document,
                            document,
                            search_result,
                            query,
                            search_matcher,
                            results_visible,
                            selected_source_row,
                        },
                        window,
                        cx,
                    )
                })
                .ok()
                .flatten();
            let Some(mut plan) = plan else {
                _ = this.update_in(cx, |this, window, cx| {
                    this.finish_reload(strategy);
                    this.open_queued_external_paths_if_idle(window, cx);
                    cx.notify();
                });
                return;
            };
            let document = plan.document.clone();
            let log_request = plan
                .log_request
                .take()
                .expect("a reload plan has a log frame request");
            let result_request = plan
                .result_request
                .take()
                .expect("a reload plan has a result frame request");
            let (log_lines, result_lines) = cx
                .background_spawn(async move {
                    let mut reader = LinePreviewReader::default();
                    let log_lines = log_request.load(|source_row, max_bytes| {
                        reader.line_preview(&document, *source_row, max_bytes)
                    });
                    let result_lines = result_request.load(|source_row, max_bytes| {
                        reader.line_preview(&document, *source_row, max_bytes)
                    });
                    (log_lines, result_lines)
                })
                .await;
            _ = this.update_in(cx, |this, window, cx| {
                this.commit_reload_replacement(
                    PreparedReloadReplacement {
                        plan,
                        log_lines,
                        result_lines,
                    },
                    strategy,
                    window,
                    cx,
                );
            });
        });
        match strategy {
            ReloadStrategy::Full => self.open_task = Some(task),
            ReloadStrategy::ExtendAppend => self.file_refresh_task = Some(task),
        }
        true
    }
}
