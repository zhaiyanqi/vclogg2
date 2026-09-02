use super::*;

impl Workspace {
    /// Installs the active search session's matcher and hit rows into the shared upper log
    /// projection. Global and directory searches therefore highlight the opened file with the
    /// same matcher as the lower result projection.
    pub(super) fn refresh_active_log_search_presentation(&mut self, cx: &mut Context<Self>) {
        let Some(active_ix) = self.active_ix else {
            return;
        };
        let tab = &self.documents[active_ix];
        let (matched_rows, matcher, session_color_rules) = match self.global_search.scope {
            SearchScope::CurrentFile => (
                tab.search_result.line_indices.clone(),
                tab.search_matcher.clone(),
                Arc::default(),
            ),
            scope @ (SearchScope::AllOpenFiles | SearchScope::Directory)
                if self.global_search.result_scope == Some(scope) =>
            {
                let result = match scope {
                    SearchScope::AllOpenFiles => self.global_search.results.get(&tab.id),
                    SearchScope::Directory => self.global_search.results.values().find(|result| {
                        result_snapshot_matches_document(
                            &result.path,
                            &result.document,
                            &tab.document,
                        )
                    }),
                    SearchScope::CurrentFile => unreachable!(),
                };
                (
                    result
                        .map(|result| result.search_result.line_indices.clone())
                        .unwrap_or_default(),
                    self.global_search.matcher.clone(),
                    match scope {
                        SearchScope::AllOpenFiles => self
                            .global_search
                            .all_open_context
                            .resolved_color_rules
                            .clone(),
                        SearchScope::Directory => self
                            .global_search
                            .directory_context
                            .resolved_color_rules
                            .clone(),
                        SearchScope::CurrentFile => unreachable!(),
                    },
                )
            }
            SearchScope::AllOpenFiles | SearchScope::Directory => {
                (CompressedRows::default(), None, Arc::default())
            }
        };
        let matcher = self
            .app_settings
            .highlight_matches
            .then_some(matcher)
            .flatten();
        let color_rules =
            ResolvedColorRules::layered(tab.file.resolved_color_rules.clone(), session_color_rules);
        let table = self.documents[active_ix].log_table.clone();
        table.update(cx, |table, cx| {
            table.delegate_mut().set_matched_rows(matched_rows);
            table.delegate_mut().set_search_matcher(matcher);
            table.delegate_mut().set_color_rules(color_rules);
            table.refresh(cx);
            cx.notify();
        });
    }

    pub(super) fn open_global_search_files_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.documents.is_empty() {
            return;
        }
        let files = self
            .documents
            .iter()
            .map(|tab| GlobalSearchFileOption {
                document_id: tab.id,
                title: tab.file.title.clone(),
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

    pub(super) fn open_directory_search_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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

    pub(super) fn apply_directory_search_options(
        &mut self,
        options: DirectorySearchOptions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.global_search.directory_options == options {
            return;
        }
        let previous_directory = self
            .global_search
            .directory_options
            .directory
            .as_deref()
            .map(normalized_path_match_key);
        let next_directory = options.directory.as_deref().map(normalized_path_match_key);
        let directory_changed = previous_directory != next_directory;
        if self.global_search.scope == SearchScope::Directory
            && self.searches.has_target(SearchTarget::Directory)
        {
            self.cancel_search();
        }
        if directory_changed {
            self.capture_retained_global_context(SearchScope::Directory, cx);
            self.remember_current_directory_session();
            if let Some(directory) = options.directory.as_deref()
                && let Some(session) = self.view_state.directory_session(directory)
            {
                self.install_directory_session(session, window, cx);
                self.schedule_workspace_search_state_save(window, cx);
                cx.notify();
                return;
            }
        }
        self.global_search.directory_options = options;
        if directory_changed {
            self.global_search.directory_query = SearchQuery {
                text: String::new(),
                case_sensitive: self.app_settings.default_case_sensitive,
                regex: self.app_settings.default_use_regex,
                max_results: self.app_settings.search_result_limit(),
            };
            self.case_sensitive = self.global_search.directory_query.case_sensitive;
            self.regex = self.global_search.directory_query.regex;
            self.query
                .update(cx, |query, cx| query.set_value("", window, cx));
        }
        self.global_search.pending_directory_restore = None;
        self.global_search.directory_context = SearchSessionState::default();
        self.global_search.clear_directory_document_ids();
        if self.global_search.result_scope == Some(SearchScope::Directory) {
            self.global_search.revision = self.global_search.revision.saturating_add(1);
            self.global_search.results_visible = false;
            self.global_search.results.clear();
            self.global_search.matcher = None;
            self.global_search.result_scope = None;
            self.refresh_global_result_rows(window, cx);
            self.fallback_from_hidden_global_results();
        }
        self.schedule_workspace_search_state_save(window, cx);
        cx.notify();
    }

    pub(super) fn install_directory_session(
        &mut self,
        session: PersistedDirectorySearchSession,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let search_result_limit = self.app_settings.search_result_limit();
        self.global_search.directory_query =
            Self::restored_search_query(&session.context.query, search_result_limit);
        self.global_search.directory_options = Self::restored_directory_options(session.options);
        let keyword_color_rules = session.context.keyword_color_rules.clone();
        let resolved_color_rules = resolve_color_rules(&keyword_color_rules, &self.color_labels);
        self.global_search.directory_context = SearchSessionState {
            query: self.global_search.directory_query.clone(),
            keyword_color_rules,
            resolved_color_rules,
            result_mode: ResultMode::from_database(session.context.result_mode),
            results_visible: session.context.results_visible,
            word_wrap: session.context.word_wrap,
            active: session.context.active,
            ..SearchSessionState::default()
        };
        self.global_search.pending_directory_restore =
            session.context.results_visible.then_some(session.context);
        self.global_search.revision = self.global_search.revision.saturating_add(1);
        self.global_search.results.clear();
        self.global_search.matcher = None;
        self.global_search.result_scope = None;
        self.global_search.clear_directory_document_ids();
        self.restore_retained_global_context(SearchScope::Directory, window, cx);
        self.case_sensitive = self.global_search.directory_query.case_sensitive;
        self.regex = self.global_search.directory_query.regex;
        let text = self.global_search.directory_query.text.clone();
        self.query
            .update(cx, |query, cx| query.set_value(text, window, cx));
        self.maybe_restore_persisted_search(window, cx);
    }

    pub(super) fn apply_global_selected_documents(
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
        let invalidated_all_open_results = self.invalidate_all_open_results();
        if invalidated_all_open_results.is_none() {
            self.global_search.revision = self.global_search.revision.saturating_add(1);
        }
        self.global_search.selected_documents = selected;
        let preferences = self
            .documents
            .iter()
            .map(|tab| {
                let selected = self.global_search.selected_documents.contains(&tab.id);
                let path = tab.document.path().to_path_buf();
                self.global_search.set_preference(path.clone(), selected);
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
        self.refresh_global_result_rows(window, cx);
        if invalidated_all_open_results == Some(true) {
            window.push_notification(
                crate::tr!(
                    "参与搜索的文件已改变，请重新执行全部打开文件搜索",
                    "The searched files changed. Run the all-open-files search again."
                ),
                cx,
            );
        }
        self.maybe_restore_persisted_search(window, cx);
        self.schedule_workspace_search_state_save(window, cx);
        cx.notify();
    }

    fn global_result_groups_for_context(
        &self,
        scope: SearchScope,
        context: &SearchSessionState,
    ) -> Vec<GlobalSearchGroup> {
        match scope {
            SearchScope::AllOpenFiles => self
                .documents
                .iter()
                .filter(|tab| {
                    self.global_search.selected_documents.contains(&tab.id)
                        && (!context.initialized || context.results.get(&tab.id).is_some())
                })
                .map(|tab| {
                    let result = context.results.get(&tab.id);
                    let search_result = result.map(|result| &result.search_result);
                    GlobalSearchGroup {
                        source: crate::global_search_table::GlobalSearchGroupSource {
                            document_id: tab.id,
                            title: tab.file.title.clone(),
                            path: result
                                .map(|result| result.path.clone())
                                .unwrap_or_else(|| tab.document.path().to_path_buf()),
                            document: result
                                .map(|result| result.document.clone())
                                .unwrap_or_else(|| tab.document.clone()),
                        },
                        projection: crate::global_search_table::GlobalSearchGroupProjection {
                            rows: compute_result_rows(
                                context.result_mode,
                                search_result,
                                &tab.file.marked_rows,
                            ),
                        },
                        presentation: crate::global_search_table::GlobalSearchGroupPresentation {
                            matched_rows: search_result
                                .map(|result| result.line_indices.clone())
                                .unwrap_or_default(),
                            marked_rows: tab.file.marked_rows.clone(),
                            truncated: search_result.is_some_and(|result| result.truncated)
                                && context.result_mode.includes_matches(),
                            failure: result.and_then(|result| result.failure.clone()),
                            color_rules: ResolvedColorRules::layered(
                                tab.file.resolved_color_rules.clone(),
                                context.resolved_color_rules.clone(),
                            ),
                        },
                    }
                })
                .collect::<Vec<_>>(),
            SearchScope::Directory if context.initialized => {
                let open_documents_by_path = self
                    .documents
                    .iter()
                    .map(|tab| (path_match_key(tab.document.path()), tab))
                    .collect::<BTreeMap<_, _>>();
                context
                    .results
                    .iter()
                    .filter_map(|(document_id, result)| {
                        let open_tab = path_match_map_get(&open_documents_by_path, &result.path)
                            .copied()
                            .filter(|tab| {
                                result_snapshot_matches_document(
                                    &result.path,
                                    &result.document,
                                    &tab.document,
                                )
                            });
                        let marked_rows = open_tab
                            .map(|tab| tab.file.marked_rows.clone())
                            .unwrap_or_default();
                        let rows = compute_result_rows(
                            context.result_mode,
                            Some(&result.search_result),
                            &marked_rows,
                        );
                        (!rows.is_empty() || result.failure.is_some()).then(|| GlobalSearchGroup {
                            source: crate::global_search_table::GlobalSearchGroupSource {
                                document_id: *document_id,
                                title: open_tab
                                    .map(|tab| tab.file.title.clone())
                                    .unwrap_or_else(|| result.title.clone()),
                                path: result.path.clone(),
                                document: open_tab
                                    .map(|tab| tab.document.clone())
                                    .unwrap_or_else(|| result.document.clone()),
                            },
                            projection: crate::global_search_table::GlobalSearchGroupProjection {
                                rows,
                            },
                            presentation:
                                crate::global_search_table::GlobalSearchGroupPresentation {
                                    matched_rows: result.search_result.line_indices.clone(),
                                    marked_rows,
                                    truncated: result.search_result.truncated
                                        && context.result_mode.includes_matches(),
                                    failure: result.failure.clone(),
                                    color_rules: ResolvedColorRules::layered(
                                        open_tab
                                            .map(|tab| tab.file.resolved_color_rules.clone())
                                            .unwrap_or_else(Arc::default),
                                        context.resolved_color_rules.clone(),
                                    ),
                                },
                        })
                    })
                    .collect::<Vec<_>>()
            }
            SearchScope::CurrentFile | SearchScope::Directory => Vec::new(),
        }
    }

    fn active_global_result_context(&self) -> SearchSessionState {
        let retained = match self.global_search.scope {
            SearchScope::AllOpenFiles => &self.global_search.all_open_context,
            SearchScope::Directory => &self.global_search.directory_context,
            SearchScope::CurrentFile => return SearchSessionState::default(),
        };
        SearchSessionState {
            query: match self.global_search.scope {
                SearchScope::AllOpenFiles => self.global_search.query.clone(),
                SearchScope::Directory => self.global_search.directory_query.clone(),
                SearchScope::CurrentFile => SearchQuery::default(),
            },
            keyword_color_rules: retained.keyword_color_rules.clone(),
            resolved_color_rules: retained.resolved_color_rules.clone(),
            initialized: self.global_search.result_scope == Some(self.global_search.scope),
            results: self.global_search.results.clone(),
            matcher: self.global_search.matcher.clone(),
            result_mode: self.global_search.result_mode,
            results_visible: self.global_search.results_visible,
            ..SearchSessionState::default()
        }
    }

    pub(super) fn refresh_global_result_rows(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.refresh_global_result_rows_with_viewport(true, None, window, cx);
    }

    fn refresh_global_result_rows_with_viewport(
        &mut self,
        preserve_viewport: bool,
        restore_context: Option<SearchSessionState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let context = self.active_global_result_context();
        let groups = self.global_result_groups_for_context(self.global_search.scope, &context);

        let matcher = self.global_result_matcher();
        let virtual_content_changed = !self
            .global_table
            .read(cx)
            .delegate()
            .has_same_virtual_content(&groups);
        if !virtual_content_changed {
            self.install_global_result_groups(groups, matcher, cx);
            return;
        }

        if let Some(cancellation) = self.global_result_replace_cancellation.take() {
            cancellation.store(true, Ordering::Release);
        }
        self.global_result_replace_task.take();
        self.global_result_replace_revision = self.global_result_replace_revision.saturating_add(1);

        if groups.is_empty() && restore_context.is_none() {
            self.install_global_result_groups(groups, matcher, cx);
            self.global_viewport.invalidate_wrapped();
            self.refresh_global_result_surface_atomically(window, cx);
            return;
        }

        let word_wrap = self.global_viewport.is_wrapped();
        let row_height = self.log_row_height();
        let viewport_anchor = restore_context
            .as_ref()
            .and_then(|context| context.viewport)
            .or_else(|| {
                preserve_viewport
                    .then(|| self.capture_global_viewport_anchor(row_height, cx))
                    .flatten()
            });
        let measured_heights = {
            let table = self.global_table.read(cx);
            if word_wrap {
                self.global_viewport
                    .wrapped_measured_heights_by_key(|row_ix| table.delegate().row_key(row_ix))
            } else {
                BTreeMap::new()
            }
        };
        let collapsed_document_ids = restore_context.as_ref().map_or_else(
            || {
                self.global_table
                    .read(cx)
                    .delegate()
                    .collapsed_document_ids()
            },
            |context| context.collapsed_document_ids.clone(),
        );
        let mut replacement = GlobalSearchTableDelegate::new();
        replacement.set_groups(groups.clone());
        replacement.restore_collapsed_document_ids(&collapsed_document_ids);
        let anchor_ix = viewport_anchor
            .as_ref()
            .and_then(|anchor| replacement.nearest_row_ix_for_key(anchor.key))
            .or_else(|| viewport_anchor.as_ref().map(|anchor| anchor.fallback_ix))
            .unwrap_or_default();
        let table_visible_rows = self.global_table.read(cx).visible_range().rows().len();
        let window_visible_rows = (window.viewport_size().height / row_height.max(px(1.)))
            .ceil()
            .max(1.) as usize;
        let preload_range = search_scope_switch_preload_range(
            anchor_ix,
            viewport_anchor.as_ref().is_some_and(|anchor| anchor.at_end),
            replacement.rows_len(),
            table_visible_rows.max(window_visible_rows),
        );
        let table = self.global_table.clone();
        let (expected_content_revision, expected_layout_revision, request) = {
            let table = table.read(cx);
            let delegate = table.delegate();
            (
                delegate.content_revision(),
                delegate.layout_revision(),
                delegate.stage_groups_replacement(&replacement, preload_range),
            )
        };
        let documents = replacement.staged_visible_documents(&request);
        let cancellation = Arc::new(AtomicBool::new(false));
        self.global_result_replace_cancellation = Some(cancellation.clone());
        let revision = self.global_result_replace_revision;
        self.global_result_replace_task = Some(cx.spawn_in(window, async move |this, cx| {
            let staged = cx
                .background_spawn(async move {
                    let mut readers = BTreeMap::<u64, LinePreviewReader>::new();
                    request.load_cancellable(
                        &cancellation,
                        |(document_id, source_row), max_bytes| {
                            let document = documents.get(document_id)?;
                            readers.entry(*document_id).or_default().line_preview(
                                document,
                                *source_row,
                                max_bytes,
                            )
                        },
                    )
                })
                .await;
            _ = this.update_in(cx, |this, window, cx| {
                if this.global_result_replace_revision != revision {
                    return;
                }
                let current_is_valid = {
                    let current = this.global_table.read(cx);
                    current.delegate().content_revision() == expected_content_revision
                        && current.delegate().layout_revision() == expected_layout_revision
                };
                if !current_is_valid {
                    this.refresh_global_result_rows(window, cx);
                    return;
                }
                this.commit_global_result_replacement(
                    PreparedGlobalResultReplacement {
                        expected_content_revision,
                        expected_layout_revision,
                        groups,
                        matcher,
                        staged,
                        viewport_anchor,
                        measured_heights,
                        row_height,
                        word_wrap,
                        restore_context,
                    },
                    window,
                    cx,
                );
            });
        }));
    }

    fn commit_global_result_replacement(
        &mut self,
        prepared: PreparedGlobalResultReplacement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let valid = {
            let table = self.global_table.read(cx);
            table.delegate().content_revision() == prepared.expected_content_revision
                && table.delegate().layout_revision() == prepared.expected_layout_revision
        };
        if !valid {
            self.refresh_global_result_rows(window, cx);
            return;
        }
        self.global_search.restoring_selection = prepared
            .restore_context
            .as_ref()
            .is_none_or(|context| context.selected_row.is_some());
        let restore_context = prepared.restore_context.clone();
        let active_restored = self.global_table.update(cx, |table, cx| {
            if let Some(context) = restore_context.as_ref() {
                table
                    .delegate_mut()
                    .restore_collapsed_document_ids(&context.collapsed_document_ids);
            }
            table.delegate_mut().install_groups_replacement(
                prepared.groups,
                prepared.matcher,
                prepared.staged,
            );
            if let Some(context) = restore_context.as_ref() {
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
            }
            let active_restored = table.sync_active_log_row(cx);
            table.refresh(cx);
            cx.notify();
            active_restored
        });
        if !active_restored {
            self.global_search.restoring_selection = false;
        }
        if prepared.word_wrap {
            let table = self.global_table.read(cx);
            self.global_viewport.reset_wrapped_with_remapped_heights(
                table.delegate().rows_len(),
                prepared.row_height,
                prepared.measured_heights,
                |key| table.delegate().row_ix_for_key(*key),
            );
        } else {
            self.global_viewport.invalidate_wrapped();
        }
        self.restore_global_viewport_anchor(prepared.viewport_anchor, prepared.row_height, cx);
        if prepared.viewport_anchor.is_none() {
            self.global_viewport.place_at_top(0, prepared.row_height);
        }
        if let Some(context) = restore_context {
            if !context.word_wrap {
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
        self.global_result_replace_task = None;
        self.global_result_replace_cancellation = None;
        self.bind_active_display_tables(cx);
        Self::refresh_log_surfaces_atomically(
            [self.search_results_viewer.surface.clone()],
            window,
            cx,
        );
    }

    pub(super) fn global_result_matcher(&self) -> Option<SearchMatcher> {
        (self.global_search.result_mode.includes_matches() && self.app_settings.highlight_matches)
            .then(|| self.global_search.matcher.clone())
            .flatten()
    }

    pub(super) fn install_global_result_groups(
        &mut self,
        groups: Vec<GlobalSearchGroup>,
        matcher: Option<SearchMatcher>,
        cx: &mut Context<Self>,
    ) {
        self.global_search.restoring_selection = true;
        let active_restored = self.global_table.update(cx, |table, cx| {
            table.delegate_mut().set_groups(groups);
            table.delegate_mut().set_search_matcher(matcher);
            let active_restored = table.sync_active_log_row(cx);
            table.refresh(cx);
            cx.notify();
            active_restored
        });
        if !active_restored {
            self.global_search.restoring_selection = false;
        }
    }

    pub(super) fn reset_search_history_navigation(&mut self) {
        self.search_history_ix = None;
        self.search_history_draft = None;
    }

    pub(super) fn close_search_autocomplete(&mut self) {
        self.search_autocomplete_mode = SearchAutocompleteMode::Closed;
        self.search_suggestion_ix = None;
    }

    pub(super) fn reset_search_suggestion_scroll(&self) {
        let base_handle = {
            let mut state = self.search_suggestion_scroll.0.borrow_mut();
            state.deferred_scroll_to_item = None;
            state.base_handle.clone()
        };
        base_handle.set_offset(point(px(0.), px(0.)));
    }

    pub(super) fn search_autocomplete_suggestions(&self, cx: &App) -> Vec<SearchSuggestion> {
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

    pub(super) fn accept_active_search_suggestion(
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

    pub(super) fn refresh_search_autocomplete(&mut self, cx: &mut Context<Self>) {
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

    pub(super) fn accept_search_suggestion(
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

    pub(super) fn toggle_search_history_popup(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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

    pub(super) fn navigate_search_autocomplete_by_key(
        &mut self,
        key: &str,
        cx: &mut Context<Self>,
    ) -> bool {
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

    pub(super) fn set_query_from_search_history(
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

    pub(super) fn navigate_search_history_by_wheel(
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

    pub(super) fn choose_predefined_filter(
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
            self.set_active_search_options(self.case_sensitive, true, window, cx);
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

    pub(super) fn apply_search_history(&mut self, history: Vec<String>, cx: &mut Context<Self>) {
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

    pub(super) fn replace_search_history(
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

    pub(super) fn remove_search_history_entries(
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

    pub(super) fn record_search_history(
        &mut self,
        query: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if query.is_empty() {
            return;
        }
        let history = std::iter::once(query.to_string())
            .chain(self.search_history.iter().cloned())
            .collect();
        self.replace_search_history(history, window, cx);
    }

    pub(super) fn persisted_search_query(query: &SearchQuery) -> PersistedSearchQuery {
        PersistedSearchQuery {
            text: query.text.clone(),
            case_sensitive: query.case_sensitive,
            regex: query.regex,
        }
    }

    pub(super) fn restored_search_query(
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

    pub(super) fn persisted_directory_options(
        options: &DirectorySearchOptions,
    ) -> PersistedDirectorySearchOptions {
        PersistedDirectorySearchOptions {
            directory: options.directory.as_deref().map(encode_persisted_path),
            file_type: 0,
            file_type_filter_enabled: Some(options.file_type_filter_enabled),
            file_type_patterns: Some(options.file_type_patterns.clone()),
            include_subdirectories: options.include_subdirectories,
            include_hidden_directories: options.include_hidden_directories,
        }
    }

    pub(super) fn restored_directory_options(
        options: PersistedDirectorySearchOptions,
    ) -> DirectorySearchOptions {
        let (legacy_enabled, legacy_patterns) =
            DirectorySearchOptions::from_legacy_file_type(options.file_type);
        DirectorySearchOptions {
            directory: options.directory.as_deref().map(decode_persisted_path),
            file_type_filter_enabled: options.file_type_filter_enabled.unwrap_or(legacy_enabled),
            file_type_patterns: options.file_type_patterns.unwrap_or(legacy_patterns),
            include_subdirectories: options.include_subdirectories,
            include_hidden_directories: options.include_hidden_directories,
        }
    }

    pub(super) fn current_directory_session(&self) -> Option<PersistedDirectorySearchSession> {
        let directory = self
            .global_search
            .directory_options
            .directory
            .as_deref()
            .map(encode_persisted_path)?;
        Some(PersistedDirectorySearchSession {
            directory,
            options: Self::persisted_directory_options(&self.global_search.directory_options),
            context: self.persisted_global_context(
                SearchScope::Directory,
                &self.global_search.directory_context,
                self.global_search.pending_directory_restore.as_ref(),
            ),
            last_used: 0,
        })
    }

    pub(super) fn remember_current_directory_session(&mut self) {
        if let Some(session) = self.current_directory_session() {
            self.view_state.remember_directory_session(session);
        }
    }

    pub(super) fn global_context_path<'a>(
        &'a self,
        context: &'a SearchSessionState,
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

    pub(super) fn persisted_global_context(
        &self,
        scope: SearchScope,
        context: &SearchSessionState,
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
            persisted.keyword_color_rules = context.keyword_color_rules.clone();
            persisted.active = context.active;
            return persisted;
        }

        let collapsed_paths = context
            .collapsed_document_ids
            .iter()
            .filter_map(|document_id| self.global_context_path(context, *document_id))
            .map(encode_persisted_path)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let selection = context
            .selection
            .iter()
            .filter_map(|(document_id, rows)| {
                self.global_context_path(context, *document_id)
                    .map(|path| PersistedPathSelection::new(encode_persisted_path(path), rows))
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
                    path: encode_persisted_path(path),
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
            keyword_color_rules: context.keyword_color_rules.clone(),
            collapsed_paths,
            selection,
            selected_row,
            viewport,
            active: context.active,
            ..PersistedGlobalSearchContext::default()
        }
    }

    pub(super) fn workspace_search_state(&self) -> WorkspaceSearchState {
        let directory = self.persisted_global_context(
            SearchScope::Directory,
            &self.global_search.directory_context,
            self.global_search.pending_directory_restore.as_ref(),
        );
        let directory_options =
            Self::persisted_directory_options(&self.global_search.directory_options);
        let active_directory = directory_options.directory.clone();
        let mut directories = self.view_state.directory_sessions();
        if let Some(current) = self.current_directory_session() {
            let current_key = normalized_path_match_key(&decode_persisted_path(&current.directory));
            directories.retain(|session| {
                normalized_path_match_key(&decode_persisted_path(&session.directory)) != current_key
            });
            let mut current = current;
            current.last_used = directories
                .iter()
                .map(|session| session.last_used)
                .max()
                .unwrap_or_default()
                .saturating_add(1);
            directories.push(current);
        }
        let mut state = WorkspaceSearchState {
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
            directory,
            directory_options,
            active_directory,
            directories,
            ..WorkspaceSearchState::default()
        };
        state.normalize_directory_sessions();
        state
    }

    pub(super) fn queue_workspace_search_state_save(
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

    pub(super) fn schedule_workspace_search_state_save(
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

    pub(super) fn apply_persisted_workspace_search_state(
        &mut self,
        state: WorkspaceSearchState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let search_result_limit = self.app_settings.search_result_limit();
        self.global_search.query =
            Self::restored_search_query(&state.all_open.query, search_result_limit);
        self.view_state
            .restore_directory_sessions(state.directories.clone());
        let active_directory_session = state
            .active_directory
            .as_deref()
            .map(decode_persisted_path)
            .and_then(|directory| self.view_state.directory_session(&directory));
        let (directory_context, directory_options) = active_directory_session.map_or_else(
            || (state.directory.clone(), state.directory_options.clone()),
            |session| (session.context, session.options),
        );
        self.global_search.directory_query =
            Self::restored_search_query(&directory_context.query, search_result_limit);
        self.global_search.directory_options = Self::restored_directory_options(directory_options);
        let all_open_keyword_color_rules = state.all_open.keyword_color_rules.clone();
        let all_open_resolved_color_rules =
            resolve_color_rules(&all_open_keyword_color_rules, &self.color_labels);
        self.global_search.all_open_context = SearchSessionState {
            query: self.global_search.query.clone(),
            keyword_color_rules: all_open_keyword_color_rules,
            resolved_color_rules: all_open_resolved_color_rules,
            result_mode: ResultMode::from_database(state.all_open.result_mode),
            results_visible: state.all_open.results_visible,
            word_wrap: state.all_open.word_wrap,
            active: state.all_open.active,
            ..SearchSessionState::default()
        };
        let directory_keyword_color_rules = directory_context.keyword_color_rules.clone();
        let directory_resolved_color_rules =
            resolve_color_rules(&directory_keyword_color_rules, &self.color_labels);
        self.global_search.directory_context = SearchSessionState {
            query: self.global_search.directory_query.clone(),
            keyword_color_rules: directory_keyword_color_rules,
            resolved_color_rules: directory_resolved_color_rules,
            result_mode: ResultMode::from_database(directory_context.result_mode),
            results_visible: directory_context.results_visible,
            word_wrap: directory_context.word_wrap,
            active: directory_context.active,
            ..SearchSessionState::default()
        };
        self.global_search.pending_all_open_restore =
            state.all_open.results_visible.then_some(state.all_open);
        self.global_search.pending_directory_restore = directory_context
            .results_visible
            .then_some(directory_context);
        self.global_search.scope = match state.active_scope {
            PersistedSearchScope::CurrentFile => SearchScope::CurrentFile,
            PersistedSearchScope::AllOpenFiles => SearchScope::AllOpenFiles,
            PersistedSearchScope::Directory => SearchScope::Directory,
        };
        self.view_state.active_search = match self.global_search.scope {
            SearchScope::CurrentFile => self
                .active_document()
                .map(|tab| SearchSessionKey::CurrentFile(tab.id)),
            SearchScope::AllOpenFiles => Some(SearchSessionKey::AllOpenFiles),
            SearchScope::Directory => self
                .global_search
                .directory_options
                .directory
                .as_deref()
                .map(normalized_path_match_key)
                .map(SearchSessionKey::Directory),
        };
        if matches!(
            self.global_search.scope,
            SearchScope::AllOpenFiles | SearchScope::Directory
        ) {
            self.restore_retained_global_context(self.global_search.scope, window, cx);
        }
        let (text, case_sensitive, regex) = match self.global_search.scope {
            SearchScope::CurrentFile => self.active_document().map_or_else(
                || {
                    (
                        String::new(),
                        self.app_settings.default_case_sensitive,
                        self.app_settings.default_use_regex,
                    )
                },
                |tab| {
                    (
                        tab.search_query.text.clone(),
                        tab.search_query.case_sensitive,
                        tab.search_query.regex,
                    )
                },
            ),
            SearchScope::AllOpenFiles => (
                self.global_search.query.text.clone(),
                self.global_search.query.case_sensitive,
                self.global_search.query.regex,
            ),
            SearchScope::Directory => (
                self.global_search.directory_query.text.clone(),
                self.global_search.directory_query.case_sensitive,
                self.global_search.directory_query.regex,
            ),
        };
        self.case_sensitive = case_sensitive;
        self.regex = regex;
        self.query
            .update(cx, |query, cx| query.set_value(text, window, cx));
    }

    pub(super) fn global_document_id_for_path(&self, path: &str) -> Option<u64> {
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

    pub(super) fn persisted_path_matches(actual: &std::path::Path, persisted: &str) -> bool {
        paths_match(actual, &decode_persisted_path(persisted))
    }

    pub(super) fn restore_persisted_global_presentation(
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
                Some((document_id, selection.decoded_rows()))
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
        let context = SearchSessionState {
            query: Self::restored_search_query(
                &persisted.query,
                self.app_settings.search_result_limit(),
            ),
            keyword_color_rules: persisted.keyword_color_rules.clone(),
            resolved_color_rules: resolve_color_rules(
                &persisted.keyword_color_rules,
                &self.color_labels,
            ),
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
            visible_lines: None,
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

    pub(super) fn maybe_restore_persisted_search(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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

    pub(super) fn capture_retained_global_context(&mut self, scope: SearchScope, cx: &App) {
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
        let retained_color_rules = match scope {
            SearchScope::AllOpenFiles => (
                self.global_search
                    .all_open_context
                    .keyword_color_rules
                    .clone(),
                self.global_search
                    .all_open_context
                    .resolved_color_rules
                    .clone(),
            ),
            SearchScope::Directory => (
                self.global_search
                    .directory_context
                    .keyword_color_rules
                    .clone(),
                self.global_search
                    .directory_context
                    .resolved_color_rules
                    .clone(),
            ),
            SearchScope::CurrentFile => return,
        };
        let context = SearchSessionState {
            query: match scope {
                SearchScope::AllOpenFiles => self.global_search.query.clone(),
                SearchScope::Directory => self.global_search.directory_query.clone(),
                SearchScope::CurrentFile => return,
            },
            keyword_color_rules: retained_color_rules.0,
            resolved_color_rules: retained_color_rules.1,
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
            visible_lines: Some(
                self.global_table
                    .read(cx)
                    .delegate()
                    .visible_line_snapshot(),
            ),
        };
        match scope {
            SearchScope::AllOpenFiles => self.global_search.all_open_context = context,
            SearchScope::Directory => self.global_search.directory_context = context,
            SearchScope::CurrentFile => {}
        }
    }

    pub(super) fn restore_retained_global_context(
        &mut self,
        scope: SearchScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let context = match scope {
            SearchScope::AllOpenFiles => {
                let retained = &mut self.global_search.all_open_context;
                let snapshot = retained.visible_lines.take();
                let mut context = retained.clone();
                context.visible_lines = snapshot;
                context
            }
            SearchScope::Directory => {
                let retained = &mut self.global_search.directory_context;
                let snapshot = retained.visible_lines.take();
                let mut context = retained.clone();
                context.visible_lines = snapshot;
                context
            }
            SearchScope::CurrentFile => return,
        };
        match scope {
            SearchScope::AllOpenFiles => self.global_search.query = context.query.clone(),
            SearchScope::Directory => self.global_search.directory_query = context.query.clone(),
            SearchScope::CurrentFile => return,
        }
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
        let groups = self.global_result_groups_for_context(scope, &context);
        let matcher = self.global_result_matcher();
        self.global_table.update(cx, |table, cx| {
            if let Some(snapshot) = context.visible_lines.clone() {
                table.delegate_mut().install_scope_snapshot(
                    groups,
                    matcher,
                    &context.collapsed_document_ids,
                    snapshot,
                );
            } else {
                table.delegate_mut().set_groups(groups);
                table
                    .delegate_mut()
                    .restore_collapsed_document_ids(&context.collapsed_document_ids);
                table.delegate_mut().set_search_matcher(matcher);
            }
            table.delegate().restore_selection(&context.selection);
            let selected_ix = context
                .selected_row
                .and_then(|row| table.delegate().row_ix(row));
            // Session restoration is presentation state, not a user command. Updating the
            // delegate directly keeps the selected row visible without emitting SelectRow,
            // which is reserved for explicit navigation from the result table.
            table.delegate().set_active_log_row(selected_ix);
            table.refresh(cx);
            cx.notify();
        });
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

    pub(super) fn set_search_scope(
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
        if self.global_search.scope == SearchScope::Directory {
            self.remember_current_directory_session();
        }

        self.commit_search_scope(next_scope, window, cx);
    }

    fn commit_search_scope(
        &mut self,
        next_scope: SearchScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.global_search.scope = next_scope;
        self.view_state.active_search = match next_scope {
            SearchScope::CurrentFile => self
                .active_document()
                .map(|tab| SearchSessionKey::CurrentFile(tab.id)),
            SearchScope::AllOpenFiles => Some(SearchSessionKey::AllOpenFiles),
            SearchScope::Directory => self
                .global_search
                .directory_options
                .directory
                .as_deref()
                .map(normalized_path_match_key)
                .map(SearchSessionKey::Directory),
        };
        if matches!(
            next_scope,
            SearchScope::AllOpenFiles | SearchScope::Directory
        ) {
            self.restore_retained_global_context(next_scope, window, cx);
        } else if self.active_log_region == LogRegion::GlobalResults {
            self.active_log_region = self
                .active_document()
                .filter(|tab| {
                    tab.results_visible && tab.view.selection_table == SelectionTable::Results
                })
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
        let (case_sensitive, regex) = match next_scope {
            SearchScope::CurrentFile => self
                .active_document()
                .map(|tab| (tab.search_query.case_sensitive, tab.search_query.regex))
                .unwrap_or((
                    self.app_settings.default_case_sensitive,
                    self.app_settings.default_use_regex,
                )),
            SearchScope::AllOpenFiles => (
                self.global_search.query.case_sensitive,
                self.global_search.query.regex,
            ),
            SearchScope::Directory => (
                self.global_search.directory_query.case_sensitive,
                self.global_search.directory_query.regex,
            ),
        };
        self.case_sensitive = case_sensitive;
        self.regex = regex;
        if self.query.read(cx).value().as_ref() != text {
            self.query
                .update(cx, |state, cx| state.set_value(text, window, cx));
        }
        self.schedule_workspace_search_state_save(window, cx);
        self.close_search_autocomplete();
        if !self.query.focus_handle(cx).is_focused(window) {
            self.query.focus_handle(cx).focus(window, cx);
        }
        self.bind_active_display_tables(cx);
        Self::refresh_log_surfaces_atomically(
            [
                self.log_viewer.surface.clone(),
                self.search_results_viewer.surface.clone(),
            ],
            window,
            cx,
        );
        cx.notify();
    }

    /// Updates only the active search session. Application defaults seed sessions when they are
    /// created, but changing one scope must not silently rewrite every other scope.
    pub(super) fn set_active_search_options(
        &mut self,
        case_sensitive: bool,
        regex: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.case_sensitive == case_sensitive && self.regex == regex {
            return;
        }
        self.cancel_search();
        self.case_sensitive = case_sensitive;
        self.regex = regex;
        let checkpoint = match self.global_search.scope {
            SearchScope::CurrentFile => self.active_ix.map(|active_ix| {
                let tab = &mut self.documents[active_ix];
                tab.search_query.case_sensitive = case_sensitive;
                tab.search_query.regex = regex;
                tab.id
            }),
            SearchScope::AllOpenFiles => {
                self.global_search.query.case_sensitive = case_sensitive;
                self.global_search.query.regex = regex;
                None
            }
            SearchScope::Directory => {
                self.global_search.directory_query.case_sensitive = case_sensitive;
                self.global_search.directory_query.regex = regex;
                None
            }
        };
        if let Some(document_id) = checkpoint {
            self.schedule_checkpoint(document_id, window, cx);
        } else {
            self.schedule_workspace_search_state_save(window, cx);
        }
        cx.notify();
    }

    pub(super) fn jump_to_global_result(
        &mut self,
        document_id: u64,
        source_row: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let result = self.global_search.results.get(&document_id);
        let directory_jump = (self.global_search.result_scope == Some(SearchScope::Directory))
            .then(|| {
                result.map(|result| PendingDirectoryResultJump {
                    path: result.path.clone(),
                    source_row,
                    expected_document: result.document.clone(),
                })
            })
            .flatten();
        let document_ix = self.open_document_ix_for_global_result(document_id);
        let Some(document_ix) = document_ix else {
            let Some(pending) = directory_jump else {
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
            let path = pending.path.clone();
            self.pending_directory_result_jump = Some(pending);
            self.begin_open_paths(vec![path], window, cx);
            return;
        };
        if let Some(pending) = directory_jump {
            if self.documents[document_ix].load_state != DocumentLoadState::Ready {
                self.pending_directory_result_jump = Some(pending);
                self.activate_workspace_tab(
                    WorkspaceTabId::Document(self.documents[document_ix].id),
                    window,
                    cx,
                );
                return;
            }
            if !pending.matches(&self.documents[document_ix].document) {
                Self::notify_stale_directory_result(window, cx);
                return;
            }
        }
        if !self.activate_document_log_row_atomically(document_ix, source_row, window, cx) {
            window.push_notification(
                crate::tr!(
                    "该结果行在当前文件中已不存在，请重新搜索",
                    "That result line no longer exists in the current file. Search again."
                ),
                cx,
            );
        }
    }

    pub(super) fn notify_stale_directory_result(window: &mut Window, cx: &mut App) {
        window.push_notification(
            crate::tr!(
                "该目录结果对应的文件内容已改变，请重新搜索",
                "The file for that directory result has changed. Search again."
            ),
            cx,
        );
    }

    pub(super) fn activate_global_group(
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
        let document_ix = self.open_document_ix_for_global_result(document_id);
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
            if self.global_search.result_scope == Some(SearchScope::Directory) {
                self.pending_directory_group_activation = Some(path.clone());
            }
            self.begin_open_paths(vec![path], window, cx);
            return;
        };
        if self.global_search.result_scope == Some(SearchScope::Directory)
            && self.documents[document_ix].load_state != DocumentLoadState::Ready
            && result_path.is_some()
        {
            self.pending_directory_group_activation = result_path;
            return;
        }
        self.activate_workspace_tab(
            WorkspaceTabId::Document(self.documents[document_ix].id),
            window,
            cx,
        );
    }

    pub(super) fn start_search_action(
        &mut self,
        _: &StartSearch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.start_search(window, cx);
    }

    pub(super) fn start_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_search_autocomplete();
        match self.global_search.scope {
            SearchScope::CurrentFile => self.start_current_search(window, cx),
            SearchScope::AllOpenFiles => self.start_global_search(window, cx),
            SearchScope::Directory => self.start_directory_search(window, cx),
        }
    }

    pub(super) fn start_current_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
        let document_id = self.documents[active_ix].id;
        self.cancel_search();
        let cancellation = SearchCancellation::default();
        let tab = &mut self.documents[active_ix];
        tab.search_revision += 1;
        let revision = tab.search_revision;
        let document = tab.document.clone();
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
                let Some(tab_ix) = this.documents.iter().position(|tab| tab.id == document_id)
                else {
                    return;
                };
                let tab = &mut this.documents[tab_ix];
                if tab.search_revision != revision {
                    return;
                }
                let results_changed = match result {
                    Ok((SearchRun::Completed(result), search_matcher)) => {
                        tab.search_query = query;
                        tab.search_result = result;
                        tab.search_matcher = search_matcher;
                        tab.refresh_search_matcher(highlight_matches, cx);
                        tab.results_visible = true;
                        this.activity = Activity::Ready;
                        this.schedule_checkpoint(document_id, window, cx);
                        true
                    }
                    Ok((SearchRun::Cancelled, _)) => {
                        this.activity = Activity::Ready;
                        false
                    }
                    Ok((SearchRun::SourceChanged, _)) => {
                        window.push_notification("搜索期间文件内容已改变，请重新加载后重试。", cx);
                        this.activity = Activity::Error;
                        false
                    }
                    Err(error) => {
                        let message: SharedString = error.to_string().into();
                        window.push_notification(message.clone(), cx);
                        this.activity = Activity::Error;
                        false
                    }
                };
                if results_changed {
                    this.refresh_document_result_rows_atomically(document_id, window, cx);
                }
                this.searches.finish(target, revision);
                cx.notify();
            });
        });
        self.searches.set_task(task);
    }

    pub(super) fn install_completed_global_search(
        &mut self,
        completed: CompletedGlobalSearch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let preserve_viewport = completed.preserve_viewport;
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

        let pending_restore = match completed.scope {
            SearchScope::AllOpenFiles => self.global_search.pending_all_open_restore.clone(),
            SearchScope::Directory => self.global_search.pending_directory_restore.clone(),
            SearchScope::CurrentFile => None,
        };
        if let Some(persisted) = pending_restore {
            self.restore_persisted_global_presentation(completed.scope, persisted, window, cx);
        } else {
            self.refresh_global_result_rows_with_viewport(preserve_viewport, None, window, cx);
        }
        self.refresh_active_log_search_presentation(cx);
        self.activity = Activity::Ready;
        self.schedule_workspace_search_state_save(window, cx);
    }

    pub(super) fn start_global_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
        let preserve_viewport = self.global_search.results_visible;
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
                    tab.file.title.clone(),
                    tab.document.path().to_path_buf(),
                    tab.document.clone(),
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
                            let run = search_with_compiled_matcher(
                                &target.3,
                                matcher_for_search,
                                max_results,
                                &cancellation,
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
                            let open_document_ids = this
                                .documents
                                .iter()
                                .map(|tab| tab.id)
                                .collect::<BTreeSet<_>>();
                            let results = outcomes
                                .into_iter()
                                .filter_map(|(target, run)| {
                                    let (document_id, title, path, document) = target;
                                    if !open_document_ids.contains(&document_id) {
                                        return None;
                                    }
                                    let (search_result, failure) = match run {
                                        Ok(SearchRun::Completed(result)) => (result, None),
                                        Ok(SearchRun::SourceChanged) => (
                                            SearchResult::default(),
                                            Some(
                                                "搜索期间文件内容已改变，请重新加载后重试。".into(),
                                            ),
                                        ),
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
                                    preserve_viewport,
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

    pub(super) fn start_directory_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
        let preserve_viewport = self.global_search.results_visible;
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
                    let Some(enumeration) = enumerate_directory_search_paths(
                        &options,
                        &cancellation_for_search,
                    )? else {
                        return Ok::<_, anyhow::Error>((
                            true,
                            Vec::new(),
                            matcher,
                            0,
                            0,
                            0,
                        ));
                    };
                    let file_count = enumeration.paths.len();
                    let unreadable_directory_count = enumeration.unreadable_directory_count;
                    let max_results = query_for_search.max_results;
                    let scan_paths = directory_search_scan_paths(
                        enumeration.paths,
                        matcher.is_some(),
                        &open_document_paths,
                    );
                    let outcomes = prepare_paths_bounded_while(
                        scan_paths,
                        || !cancellation_for_search.is_cancelled(),
                        |path| -> Result<Option<(Arc<LogDocument>, SearchResult)>> {
                        if cancellation_for_search.is_cancelled() {
                            return Ok(None);
                        }
                        let opened = if let Some(cache_dir) = crate::app_paths::index_cache_dir() {
                            LogDocument::open_with_index_cache_and_search_cancellable(
                                path,
                                cache_dir,
                                matcher
                                    .as_ref()
                                    .expect("directory scan paths require a compiled matcher"),
                                max_results,
                                &cancellation_for_search,
                            )?
                        } else {
                            LogDocument::open_cancellable(path, &cancellation_for_search)?.map(
                                |document| {
                                    let run = search_with_compiled_matcher(
                                        &document,
                                        matcher.as_ref(),
                                        max_results,
                                        &cancellation_for_search,
                                    );
                                    (document, None, run)
                                },
                            )
                        };
                        let Some((document, pending_index_cache, run)) = opened else {
                            return Ok(None);
                        };
                        let document = Arc::new(document);
                        match run {
                            SearchRun::Completed(search_result)
                                if !search_result.line_indices.is_empty()
                                    || path_match_set_contains(&open_document_paths, path) =>
                            {
                                let document = Arc::new(
                                    document.project_source_rows(&search_result.line_indices),
                                );
                                document.release_source_handle();
                                if !cancellation_for_search.is_cancelled()
                                    && let Some(cache_write) = pending_index_cache
                                {
                                    _ = cache_write.persist();
                                }
                                Ok(Some((document, search_result)))
                            }
                            SearchRun::Completed(_) => {
                                if !cancellation_for_search.is_cancelled()
                                    && let Some(cache_write) = pending_index_cache
                                {
                                    _ = cache_write.persist();
                                }
                                Ok(None)
                            }
                            SearchRun::SourceChanged => Err(anyhow::anyhow!(
                                "搜索期间文件内容已改变，请重新加载后重试：{}",
                                path.display()
                            )),
                            SearchRun::Cancelled => Ok(None),
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
                                preserve_viewport,
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

    pub(super) fn clear_search_action(
        &mut self,
        _: &ClearSearch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.clear_search(window, cx);
    }

    pub(super) fn cancel_search_action(
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

    pub(super) fn clear_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.global_search.scope {
            SearchScope::CurrentFile => self.clear_current_search(window, cx),
            SearchScope::AllOpenFiles => self.clear_global_search(window, cx),
            SearchScope::Directory => self.clear_directory_search(window, cx),
        }
    }

    pub(super) fn clear_current_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(active_ix) = self.active_ix else {
            return;
        };
        let document_id = self.documents[active_ix].id;
        self.cancel_search_for(document_id);
        let highlight_matches = self.app_settings.highlight_matches;
        let case_sensitive = self.app_settings.default_case_sensitive;
        let regex = self.app_settings.default_use_regex;
        let max_results = self.app_settings.search_result_limit();
        {
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
            tab.view.selection_table = SelectionTable::Log;
            tab.refresh_search_matcher(highlight_matches, cx);
        }
        self.refresh_document_result_rows_atomically(document_id, window, cx);
        self.case_sensitive = case_sensitive;
        self.regex = regex;
        if self.active_log_region == LogRegion::CurrentResults {
            self.active_log_region = LogRegion::Body;
        }
        self.reset_search_history_navigation();
        self.close_search_autocomplete();
        self.query
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.activity = Activity::Ready;
        self.refresh_active_document_surfaces_atomically(window, cx);
        self.schedule_checkpoint(document_id, window, cx);
        cx.notify();
    }

    pub(super) fn invalidate_all_open_results_for_reload(
        &mut self,
        document_id: u64,
    ) -> Option<bool> {
        let installed = self.global_search.result_scope == Some(SearchScope::AllOpenFiles)
            && self.global_search.results.get(&document_id).is_some();
        let retained = self
            .global_search
            .all_open_context
            .results
            .get(&document_id)
            .is_some();
        if !installed && !retained {
            return None;
        }

        self.invalidate_all_open_results()
    }

    pub(super) fn invalidate_all_open_results(&mut self) -> Option<bool> {
        let installed = self.global_search.result_scope == Some(SearchScope::AllOpenFiles);
        let retained = self.global_search.all_open_context.initialized;
        if !installed && !retained {
            return None;
        }

        let visible = installed
            && self.global_search.scope == SearchScope::AllOpenFiles
            && self.global_search.results_visible;
        self.global_search.revision = self.global_search.revision.saturating_add(1);
        if installed {
            self.global_search.results.clear();
            self.global_search.matcher = None;
            self.global_search.result_scope = None;
            self.global_search.results_visible = false;
        }
        self.global_search.all_open_context.invalidate_results();
        self.global_search.pending_all_open_restore = None;
        self.fallback_from_hidden_global_results();
        Some(visible)
    }

    pub(super) fn fallback_from_hidden_global_results(&mut self) {
        if self.active_log_region != LogRegion::GlobalResults || self.global_search.results_visible
        {
            return;
        }
        self.active_log_region = self
            .active_document()
            .filter(|tab| {
                tab.results_visible && tab.view.selection_table == SelectionTable::Results
            })
            .map(|_| LogRegion::CurrentResults)
            .unwrap_or(LogRegion::Body);
    }

    pub(super) fn clear_global_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
        self.global_search.all_open_context = SearchSessionState::default();
        self.case_sensitive = self.global_search.query.case_sensitive;
        self.regex = self.global_search.query.regex;
        self.refresh_global_result_rows(window, cx);
        self.fallback_from_hidden_global_results();
        self.reset_search_history_navigation();
        self.close_search_autocomplete();
        self.query
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.activity = Activity::Ready;
        self.schedule_workspace_search_state_save(window, cx);
        cx.notify();
    }

    pub(super) fn clear_directory_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
        self.global_search.directory_context = SearchSessionState::default();
        self.case_sensitive = self.global_search.directory_query.case_sensitive;
        self.regex = self.global_search.directory_query.regex;
        self.global_search.clear_directory_document_ids();
        self.refresh_global_result_rows(window, cx);
        self.fallback_from_hidden_global_results();
        self.reset_search_history_navigation();
        self.close_search_autocomplete();
        self.query
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.activity = Activity::Ready;
        self.schedule_workspace_search_state_save(window, cx);
        cx.notify();
    }

    pub(super) fn cancel_search(&mut self) -> bool {
        let was_active = self.searches.cancel();
        if was_active && matches!(self.activity, Activity::Searching) {
            self.activity = Activity::Ready;
        }
        was_active
    }

    pub(super) fn cancel_search_for(&mut self, document_id: u64) {
        if self.searches.cancel_for_document(document_id)
            && matches!(self.activity, Activity::Searching)
        {
            self.activity = Activity::Ready;
        }
    }
}
