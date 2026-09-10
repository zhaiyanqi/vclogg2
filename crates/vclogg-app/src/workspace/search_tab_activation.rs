//! Prepare the next result window off-thread, then publish it with its search-tab identity.
use super::search_tabs::{SearchTabId, SearchTabOwner};
use super::*;

pub(super) enum PreparedSearchTabFrame {
    File {
        rows: CompressedRows,
        selection: Vec<(usize, usize)>,
        active: Option<usize>,
        lines: StagedVisibleLineLoadResult<usize>,
    },
    Global {
        groups: Vec<GlobalSearchGroup>,
        lines: StagedVisibleLineLoadResult<(u64, usize)>,
    },
}

impl Workspace {
    pub(super) fn cancel_search_tab_activation(&mut self) {
        self.search_tabs.activation_revision = self.search_tabs.activation_revision.wrapping_add(1);
        if let Some(cancel) = self.search_tabs.activation_cancellation.take() {
            cancel.store(true, Ordering::Release);
        }
        self.search_tabs.activation_task = None;
    }

    pub(super) fn prepare_search_tab_activation(
        &mut self,
        owner: SearchTabOwner,
        id: SearchTabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cancel_search_tab_activation();
        let Some(state) = self.search_tabs.state(owner, id).cloned() else {
            return;
        };
        // Empty/restoring tabs have no completed frame to read yet.
        if state.needs_restore || !state.saved.context.results_visible {
            self.commit_search_tab_activation(owner, id, window, cx);
            return;
        }
        let local = match owner {
            SearchTabOwner::File(document_id) => {
                let Some(tab) = self.documents.iter().find(|tab| tab.id == document_id) else {
                    return;
                };
                let Some((document, result, _)) = state
                    .local_result
                    .as_ref()
                    .filter(|(document, _, _)| Arc::ptr_eq(document, &tab.document))
                else {
                    self.commit_search_tab_activation(owner, id, window, cx);
                    return;
                };
                Some((
                    document_id,
                    document.clone(),
                    result.clone(),
                    tab.file.marked_rows.clone(),
                ))
            }
            _ => None,
        };
        let groups = local
            .is_none()
            .then(|| self.global_result_groups_for_context(owner.scope(), &state.context));
        let source = self.search_tabs.installed;
        let revision = self.search_tabs.activation_revision;
        let target_revision = state.revision;
        let expected_results = state.context.results.clone();
        let expected_local = local
            .as_ref()
            .map(|(id, document, _, marks)| (*id, document.clone(), marks.clone()));
        let expected_documents = groups.as_ref().map(|_| {
            self.documents
                .iter()
                .map(|tab| {
                    (
                        tab.id,
                        tab.document.clone(),
                        tab.file.marked_rows.clone(),
                        tab.file.resolved_color_rules.clone(),
                        tab.file.title.clone(),
                    )
                })
                .collect::<Vec<_>>()
        });
        let row_height = self.log_row_height();
        let window_size = window.viewport_size();
        let visible_rows = (window_size.height / row_height.max(px(1.))).ceil().max(1.) as usize;
        let cancellation = Arc::new(AtomicBool::new(false));
        self.search_tabs.activation_cancellation = Some(cancellation.clone());
        self.search_tabs.activation_task = Some(cx.spawn_in(window, async move |this, cx| {
            let worker_cancel = cancellation.clone();
            let frame = cx
                .background_spawn(async move {
                    if let Some((document_id, document, result, marks)) = local {
                        let rows = compute_result_rows(
                            ResultMode::from_database(state.saved.context.result_mode),
                            Some(&result),
                            &marks,
                        );
                        let selection = state
                            .saved
                            .context
                            .selection
                            .first()
                            .map(PersistedPathSelection::decoded_rows)
                            .unwrap_or_default();
                        let selection = rows.position_ranges_for_subset(&selection);
                        let active = state
                            .saved
                            .local
                            .selected_source_row
                            .and_then(|row| rows.position(row));
                        let bookmark = state.saved.local.viewport.unwrap_or_default();
                        let anchor = rows
                            .nearest_position(bookmark.anchor_source_row)
                            .unwrap_or_default();
                        let range = search_scope_switch_preload_range(
                            anchor,
                            bookmark.at_end,
                            rows.len(),
                            visible_rows,
                        );
                        let delegate = LogTableDelegate::projected(
                            document_id,
                            document.clone(),
                            rows.clone(),
                        );
                        let request = delegate.stage_row_projection_replacement(&rows, range);
                        let mut reader = LinePreviewReader::for_viewport();
                        let lines = request.load_cancellable(&worker_cancel, |row, max_bytes| {
                            reader.line_preview(&document, *row, max_bytes)
                        });
                        PreparedSearchTabFrame::File {
                            rows,
                            selection,
                            active,
                            lines,
                        }
                    } else {
                        let groups = groups.unwrap_or_default();
                        let mut replacement = GlobalSearchTableDelegate::new();
                        replacement.set_groups(groups.clone());
                        replacement
                            .restore_collapsed_document_ids(&state.context.collapsed_document_ids);
                        let anchor = state
                            .context
                            .viewport
                            .and_then(|anchor| {
                                replacement
                                    .nearest_row_ix_for_key(anchor.key)
                                    .or(Some(anchor.fallback_ix))
                            })
                            .unwrap_or_default();
                        let range = search_scope_switch_preload_range(
                            anchor,
                            state.context.viewport.is_some_and(|anchor| anchor.at_end),
                            replacement.rows_len(),
                            visible_rows,
                        );
                        let request = replacement.stage_groups_replacement(&replacement, range);
                        let documents = replacement.staged_visible_documents(&request);
                        let mut readers = BTreeMap::<u64, LinePreviewReader>::new();
                        let lines = request.load_cancellable(
                            &worker_cancel,
                            |(document_id, row), max_bytes| {
                                readers
                                    .entry(*document_id)
                                    .or_insert_with(LinePreviewReader::for_viewport)
                                    .line_preview(documents.get(document_id)?, *row, max_bytes)
                            },
                        );
                        PreparedSearchTabFrame::Global { groups, lines }
                    }
                })
                .await;
            _ = this.update_in(cx, |this, window, cx| {
                if cancellation.load(Ordering::Acquire)
                    || this.search_tabs.activation_revision != revision
                {
                    return;
                }
                this.search_tabs.activation_task = None;
                this.search_tabs.activation_cancellation = None;
                if this.search_tabs.installed != source || this.search_tab_owner() != Some(owner) {
                    return;
                }
                let Some(target) = this.search_tabs.state(owner, id) else {
                    return;
                };
                let valid_local = expected_local.as_ref().is_none_or(|(id, document, marks)| {
                    this.documents.iter().any(|tab| {
                        tab.id == *id
                            && Arc::ptr_eq(&tab.document, document)
                            && &tab.file.marked_rows == marks
                    })
                });
                let valid_documents = expected_documents.as_ref().is_none_or(|documents| {
                    documents.len() == this.documents.len()
                        && documents.iter().zip(&this.documents).all(
                            |((id, document, marks, colors, title), tab)| {
                                tab.id == *id
                                    && Arc::ptr_eq(document, &tab.document)
                                    && marks == &tab.file.marked_rows
                                    && Arc::ptr_eq(colors, &tab.file.resolved_color_rules)
                                    && title == &tab.file.title
                            },
                        )
                });
                if target.revision != target_revision
                    || !target.context.results.is_same_snapshot(&expected_results)
                    || !valid_local
                    || !valid_documents
                    || this.log_row_height() != row_height
                    || window.viewport_size() != window_size
                {
                    this.prepare_search_tab_activation(owner, id, window, cx);
                    return;
                }
                this.search_tabs.prepared_frame = Some(frame);
                this.commit_search_tab_activation(owner, id, window, cx);
            });
        }));
    }
}
