use super::*;

impl Workspace {
    pub(super) fn cancel_pending_tab_activation(&mut self) {
        self.tab_activation_revision = self.tab_activation_revision.saturating_add(1);
        self.tab_activation_task = None;
    }

    pub(super) fn tab_frame_visible_range(
        &self,
        tab_ix: usize,
        region: WrappedRegion,
        window: &Window,
        cx: &App,
    ) -> Range<usize> {
        let tab = &self.documents[tab_ix];
        let (table, viewport) = if region == WrappedRegion::Results {
            (&tab.result_table, &tab.result_viewport)
        } else {
            (&tab.log_table, &tab.log_viewport)
        };
        let row_count = table.read(cx).delegate().row_count();
        if row_count == 0 {
            return 0..0;
        }

        let bounds = self
            .row_drag_bounds
            .get(&(tab.id, region))
            .or_else(|| {
                (region == WrappedRegion::Results)
                    .then(|| self.row_drag_bounds.get(&(tab.id, WrappedRegion::Log)))
                    .flatten()
            })
            .copied();
        let row_height = self.log_row_height();
        if viewport.is_wrapped() {
            let viewport_height = viewport
                .wrapped_viewport_height()
                .max(bounds.map_or(px(0.), |bounds| bounds.size.height))
                .max(window.viewport_size().height);
            return wrapped_viewport_measurement_range(
                viewport.wrapped_first_visible_row(),
                viewport_height,
                row_height,
                row_count,
            );
        }

        let visible_range = table.read(cx).visible_range().rows().clone();
        let first_visible = viewport.first_visible(row_count, row_height);
        let viewport_height = bounds
            .map_or(px(0.), |bounds| bounds.size.height)
            .max(window.viewport_size().height);
        let visible_count = (viewport_height / row_height.max(px(1.))).ceil().max(1.) as usize;
        let visible_start = visible_range.start.min(row_count);
        let visible_end = visible_range.end.min(row_count);
        let preload_end = first_visible.saturating_add(visible_count).min(row_count);
        if visible_start < visible_end
            && visible_start <= preload_end
            && first_visible <= visible_end
        {
            visible_start.min(first_visible)..visible_end.max(preload_end)
        } else {
            first_visible..preload_end
        }
    }

    pub(super) fn commit_workspace_tab_activation(
        &mut self,
        tab_id: WorkspaceTabId,
        prepared_frame: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.tabs.contains(&tab_id) {
            return;
        }
        let active_tab_changed = self.active_tab_id != tab_id;
        if active_tab_changed && let Some(active) = self.active_document() {
            let path = active.document.path().to_path_buf();
            let base = active.session_base.clone();
            let state = self.file_session_state(active, cx);
            self.save_file_session(path, base, state, window, cx);
        }
        if active_tab_changed && let Some(document_id) = self.active_document().map(|tab| tab.id) {
            self.clear_document_visible_lines(document_id, cx);
        }
        self.active_tab_id = tab_id;
        self.sync_active_document_ix();
        self.pending_document_tab_reveal.set(None);
        self.sync_active_document(window, cx);
        self.refresh_active_log_search_presentation(cx);
        if active_tab_changed {
            if prepared_frame {
                self.refresh_prepared_active_document_surfaces_atomically(window, cx);
            } else {
                self.refresh_active_document_surfaces_atomically(window, cx);
            }
        }
        cx.notify();
    }

    pub(super) fn activate_workspace_tab(
        &mut self,
        tab_id: WorkspaceTabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.activate_workspace_tab_with_log_jump(tab_id, None, window, cx);
    }

    fn activate_workspace_tab_with_log_jump(
        &mut self,
        tab_id: WorkspaceTabId,
        log_jump: Option<PreparedLogJump>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.tabs.contains(&tab_id) {
            return;
        }
        self.cancel_pending_tab_activation();
        if self.active_tab_id == tab_id {
            if let Some(log_jump) = log_jump {
                if let Some(document_ix) = tab_id.document_id().and_then(|document_id| {
                    self.documents.iter().position(|tab| tab.id == document_id)
                }) {
                    let _ = self.activate_document_log_row_atomically(
                        document_ix,
                        log_jump.source_row,
                        window,
                        cx,
                    );
                }
            } else {
                self.commit_workspace_tab_activation(tab_id, false, window, cx);
            }
            return;
        }
        if tab_id.document_id().is_none() {
            self.commit_workspace_tab_activation(tab_id, false, window, cx);
            return;
        }

        let Some(tab_ix) = tab_id
            .document_id()
            .and_then(|document_id| self.documents.iter().position(|tab| tab.id == document_id))
        else {
            return;
        };
        let log_range = if let Some(log_jump) = log_jump {
            let table = self.documents[tab_ix].log_table.read(cx);
            let row_count = table.delegate().row_count();
            let table_visible_rows = table.visible_range().rows().len();
            let measured_visible_rows = self
                .row_drag_bounds
                .get(&(self.documents[tab_ix].id, WrappedRegion::Log))
                .map(|bounds| (bounds.size.height / self.log_row_height()).ceil().max(1.) as usize)
                .unwrap_or_default();
            let window_visible_rows = (window.viewport_size().height
                / self.log_row_height().max(px(1.)))
            .ceil()
            .max(1.) as usize;
            tab_switch_log_jump_preload_range(
                log_jump.row_ix,
                row_count,
                table_visible_rows,
                measured_visible_rows,
                window_visible_rows,
            )
        } else {
            self.tab_frame_visible_range(tab_ix, WrappedRegion::Log, window, cx)
        };
        let result_range = self.tab_frame_visible_range(tab_ix, WrappedRegion::Results, window, cx);
        if log_jump.is_some() {
            let tab = &mut self.documents[tab_ix];
            tab.log_jump_revision = tab.log_jump_revision.saturating_add(1);
            tab.log_jump_task.take();
        }
        let tab = &self.documents[tab_ix];
        let document_id = tab.id;
        let document = tab.document.clone();
        let log_table = tab.log_table.clone();
        let result_table = tab.result_table.clone();
        let (log_revision, log_request) = {
            let table = log_table.read(cx);
            (
                table.delegate().visible_line_revision(),
                table.delegate().stage_visible_rows(log_range),
            )
        };
        let (result_revision, result_request) = {
            let table = result_table.read(cx);
            (
                table.delegate().visible_line_revision(),
                table.delegate().stage_visible_rows(result_range),
            )
        };
        if log_request.is_none() && result_request.is_none() {
            if let Some(log_jump) = log_jump {
                self.commit_prepared_log_jump(tab_ix, log_jump, cx);
            }
            self.commit_workspace_tab_activation(tab_id, false, window, cx);
            if log_jump.is_some() {
                self.schedule_checkpoint(document_id, window, cx);
            }
            return;
        }

        let source_tab_id = self.active_tab_id;
        let activation_revision = self.tab_activation_revision;
        self.tab_activation_task = Some(cx.spawn_in(window, async move |this, cx| {
            let prepared = cx
                .background_spawn(async move {
                    let mut reader = LinePreviewReader::default();
                    let log_lines = log_request.map(|request| {
                        request.load(|source_row, max_bytes| {
                            reader.line_preview(&document, *source_row, max_bytes)
                        })
                    });
                    let result_lines = result_request.map(|request| {
                        request.load(|source_row, max_bytes| {
                            reader.line_preview(&document, *source_row, max_bytes)
                        })
                    });
                    PreparedTabFrame {
                        document_id,
                        document,
                        log_revision,
                        result_revision,
                        log_jump,
                        log_lines,
                        result_lines,
                    }
                })
                .await;
            _ = this.update_in(cx, |this, window, cx| {
                if this.tab_activation_revision != activation_revision {
                    return;
                }
                this.tab_activation_task = None;
                if this.active_tab_id != source_tab_id {
                    return;
                }
                let Some(tab_ix) = this
                    .documents
                    .iter()
                    .position(|tab| tab.id == prepared.document_id)
                else {
                    return;
                };
                let frame_is_current = {
                    let tab = &this.documents[tab_ix];
                    Arc::ptr_eq(&tab.document, &prepared.document)
                        && tab.log_table.read(cx).delegate().visible_line_revision()
                            == prepared.log_revision
                        && tab.result_table.read(cx).delegate().visible_line_revision()
                            == prepared.result_revision
                };
                if !frame_is_current {
                    this.activate_workspace_tab_with_log_jump(
                        tab_id,
                        prepared.log_jump,
                        window,
                        cx,
                    );
                    return;
                }

                if let Some(lines) = prepared.log_lines {
                    let table = this.documents[tab_ix].log_table.clone();
                    table.update(cx, |table, cx| {
                        table.delegate().install_staged_visible_lines(lines);
                        table.refresh(cx);
                        cx.notify();
                    });
                }
                if let Some(lines) = prepared.result_lines {
                    let table = this.documents[tab_ix].result_table.clone();
                    table.update(cx, |table, cx| {
                        table.delegate().install_staged_visible_lines(lines);
                        table.refresh(cx);
                        cx.notify();
                    });
                }
                if let Some(log_jump) = prepared.log_jump {
                    this.commit_prepared_log_jump(tab_ix, log_jump, cx);
                }
                this.tab_activation_revision = this.tab_activation_revision.saturating_add(1);
                this.commit_workspace_tab_activation(tab_id, true, window, cx);
                if prepared.log_jump.is_some() {
                    this.schedule_checkpoint(prepared.document_id, window, cx);
                }
            });
        }));
    }

    pub(super) fn commit_prepared_log_jump(
        &mut self,
        tab_ix: usize,
        log_jump: PreparedLogJump,
        cx: &mut Context<Self>,
    ) {
        let tab = &mut self.documents[tab_ix];
        tab.view.auto_follow = false;
        tab.view.selection_table = SelectionTable::Log;
        tab.log_table.update(cx, |table, cx| {
            table.delegate().set_active_log_row(Some(log_jump.row_ix));
            table.delegate().settle_table_selection(log_jump.row_ix);
            cx.notify();
        });
        tab.log_viewport.center_row(log_jump.row_ix);
        self.selected_source_row = Some(log_jump.source_row);
    }

    pub(super) fn activate_document_log_row_atomically(
        &mut self,
        document_ix: usize,
        source_row: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(tab) = self.documents.get(document_ix) else {
            return false;
        };
        let Some(row_ix) = tab.document.local_row(source_row) else {
            return false;
        };
        let document_id = tab.id;
        if self.active_tab_id == WorkspaceTabId::Document(document_id) {
            self.cancel_pending_tab_activation();
            let tab = &mut self.documents[document_ix];
            tab.view.auto_follow = false;
            tab.view.selection_table = SelectionTable::Log;
            let selected = self.select_and_center_log_source_row_atomically(
                document_id,
                source_row,
                window,
                cx,
            );
            if selected {
                self.selected_source_row = Some(source_row);
            }
            return selected;
        }

        self.activate_workspace_tab_with_log_jump(
            WorkspaceTabId::Document(document_id),
            Some(PreparedLogJump { source_row, row_ix }),
            window,
            cx,
        );
        true
    }

    pub(super) fn activate_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.documents.len() {
            return;
        }
        let tab_id = WorkspaceTabId::Document(self.documents[ix].id);
        self.cancel_pending_tab_activation();
        self.commit_workspace_tab_activation(tab_id, false, window, cx);
    }

    pub(super) fn reveal_pending_document_tab(&self) {
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

    pub(super) fn scroll_document_tabs_from_wheel(
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

    pub(super) fn sync_active_document(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (title, query, search_options, selected_row) = self
            .active_document()
            .map(|tab| {
                (
                    format!("{} — VCLogg2", tab.file.title),
                    tab.search_query.text.clone(),
                    (tab.search_query.case_sensitive, tab.search_query.regex),
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
                    (
                        self.app_settings.default_case_sensitive,
                        self.app_settings.default_use_regex,
                    ),
                    None,
                )
            });

        window.set_window_title(&title);
        if self.global_search.scope == SearchScope::CurrentFile {
            self.reset_search_history_navigation();
            self.view_state.active_search = self
                .active_document()
                .map(|tab| SearchSessionKey::CurrentFile(tab.id));
            (self.case_sensitive, self.regex) = search_options;
            self.query
                .update(cx, |state, cx| state.set_value(query, window, cx));
        }
        self.selected_source_row = selected_row;
    }

    pub(super) fn close_active_tab(
        &mut self,
        _: &CloseActiveTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_close_workspace_tabs(BTreeSet::from([self.active_tab_id]), window, cx);
    }

    pub(super) fn close_tab_by_id(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        if !self.tabs.contains(&WorkspaceTabId::Document(id)) {
            return;
        }
        self.close_workspace_tabs(BTreeSet::from([WorkspaceTabId::Document(id)]), window, cx);
    }

    pub(super) fn close_tab_group(
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

    pub(super) fn request_close_workspace_tabs(
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
                .map(|tab| tab.file.title.to_string())
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

    pub(super) fn close_workspace_tabs(
        &mut self,
        ids: BTreeSet<WorkspaceTabId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if ids.is_empty() {
            return;
        }
        self.cancel_pending_tab_activation();
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

        for document_id in &document_ids {
            self.persistence.checkpoint_tasks.remove(*document_id);
        }

        if self
            .searches
            .is_affected_by_removed_documents(&document_ids)
        {
            self.cancel_search();
        }
        for (path, base, session) in sessions {
            self.save_file_session(path, base, session, window, cx);
        }

        self.tabs.retain(|tab_id| !ids.contains(tab_id));
        self.documents.retain(|tab| !document_ids.contains(&tab.id));
        self.row_drag_bounds
            .retain(|(tab_id, _), _| !document_ids.contains(tab_id));
        self.visible_line_tasks
            .retain(|(document_id, _), _| *document_id == 0 || !document_ids.contains(document_id));
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
        // Completed workspace-wide results own immutable sparse source snapshots and therefore
        // remain usable after their tabs close. A later explicit search rebuilds the source set.
        self.reorder_documents_to_match_tabs();
        if !document_ids.is_empty() {
            self.global_search.revision = self.global_search.revision.saturating_add(1);
            self.refresh_global_result_rows(window, cx);
        }
        self.sync_active_document(window, cx);
        if previous_active_id != self.active_tab_id {
            self.refresh_active_document_surfaces_atomically(window, cx);
        }
        self.persist_workspace_order(window, cx);
        self.schedule_workspace_search_state_save(window, cx);
        self.maybe_restore_persisted_search(window, cx);
        cx.notify();
    }

    pub(super) fn reorder_tab(
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
        self.refresh_global_result_rows(window, cx);
        self.persist_workspace_order(window, cx);
        cx.notify();
    }

    pub(super) fn persist_workspace_order(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let store = self.persistence.store.clone();
        let sessions = self
            .documents
            .iter()
            .filter(|tab| !path_match_set_contains(&self.transient_paths, tab.document.path()))
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
            .filter(|tab| !path_match_set_contains(&self.transient_paths, tab.document.path()))
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

    pub(super) fn copy_tab_file_path(
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

    pub(super) fn reveal_tab_file(
        &mut self,
        document_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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

    pub(super) fn copy_tab_to_new_window(
        &mut self,
        document_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.transfer_tab_to_new_window(document_id, TabTransferMode::Copy, None, window, cx);
    }

    pub(super) fn move_tab_to_new_window(
        &mut self,
        document_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.transfer_tab_to_new_window(document_id, TabTransferMode::Move, None, window, cx);
    }

    pub(super) fn transfer_tab_to_new_window(
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
        let transient = path_match_set_contains(&self.transient_paths, &path);
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

    pub(super) fn receive_transferred_tab(
        &mut self,
        initial: InitialDocument,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> TabTransferReception {
        if let Some(existing_ix) = self
            .documents
            .iter()
            .position(|tab| paths_match(tab.document.path(), &initial.path))
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

    pub(super) fn transfer_tab_to_previous_window(
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

    pub(super) fn transfer_tab_to_window_target(
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
        let file_name = tab.file.title.clone();
        let session = self.file_session_state(tab, cx);
        let transient = path_match_set_contains(&self.transient_paths, &path);
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

    pub(super) fn open_rename_tab_dialog(
        &mut self,
        document_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(current_title) = self
            .documents
            .iter()
            .find(|tab| tab.id == document_id)
            .map(|tab| tab.file.title.to_string())
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

    pub(super) fn rename_tab(
        &mut self,
        document_id: u64,
        title: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.documents.iter_mut().find(|tab| tab.id == document_id) else {
            return;
        };
        if tab.file.title.as_ref() == title {
            return;
        }
        tab.file.title = title.clone().into();
        tab.file.custom_title = Some(title.clone());
        self.global_table.update(cx, |table, cx| {
            table
                .delegate_mut()
                .update_group_title(document_id, title.clone().into());
            table.refresh(cx);
        });
        // Directory results use a search-snapshot document id, so rebuild their active
        // projection from FileState instead of relying only on the open-tab id update above.
        self.refresh_global_result_rows(window, cx);
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

    pub(super) fn restore_tab_title(
        &mut self,
        document_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.documents.iter_mut().find(|tab| tab.id == document_id) else {
            return;
        };
        if tab.file.custom_title.is_none() {
            return;
        }
        let original_title = tab.document.file_name();
        tab.file.title = original_title.clone().into();
        tab.file.custom_title = None;
        self.global_table.update(cx, |table, cx| {
            table
                .delegate_mut()
                .update_group_title(document_id, original_title.clone().into());
            table.refresh(cx);
        });
        self.refresh_global_result_rows(window, cx);
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

    pub(super) fn build_tab_menu(
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

    pub(super) fn build_new_tab_menu(
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
}
