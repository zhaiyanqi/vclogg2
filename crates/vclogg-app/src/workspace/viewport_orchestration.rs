use super::*;

impl Workspace {
    pub(super) fn refresh_document_result_rows_atomically(
        &mut self,
        document_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab_ix) = self.documents.iter().position(|tab| tab.id == document_id) else {
            return;
        };
        let rows = self.documents[tab_ix].compute_result_rows();
        let previous_rows = self.documents[tab_ix]
            .result_table
            .read(cx)
            .delegate()
            .projected_rows()
            .cloned()
            .unwrap_or_default();
        if previous_rows == rows {
            self.documents[tab_ix].install_result_rows(rows, cx);
            return;
        }

        let row_height = self.log_row_height();
        let word_wrap = self.documents[tab_ix].result_viewport.is_wrapped();
        let row_height =
            if word_wrap && self.documents[tab_ix].result_viewport.wrapped_base_height() > px(0.) {
                self.documents[tab_ix].result_viewport.wrapped_base_height()
            } else {
                row_height
            };
        let viewport_anchor = Self::capture_local_viewport_anchor(
            &self.documents[tab_ix],
            WrappedRegion::Results,
            row_height,
            cx,
        );
        let measured_heights = if word_wrap {
            let table = self.documents[tab_ix].result_table.read(cx);
            self.documents[tab_ix]
                .result_viewport
                .wrapped_measured_heights_by_key(|row_ix| table.delegate().row_key(row_ix))
        } else {
            BTreeMap::new()
        };
        let anchor_ix = viewport_anchor
            .as_ref()
            .and_then(|anchor| match anchor.key {
                LogRowKey::Row { source_row, .. } => rows.position(source_row),
                LogRowKey::FileGroup { .. } => None,
            })
            .or_else(|| viewport_anchor.as_ref().map(|anchor| anchor.fallback_ix))
            .unwrap_or_default();
        let table_visible_rows = self.documents[tab_ix]
            .result_table
            .read(cx)
            .visible_range()
            .rows()
            .len();
        let measured_visible_rows = self
            .row_drag_bounds
            .get(&(document_id, WrappedRegion::Results))
            .map(|bounds| (bounds.size.height / row_height.max(px(1.))).ceil().max(1.) as usize)
            .unwrap_or_default();
        let window_visible_rows = (window.viewport_size().height / row_height.max(px(1.)))
            .ceil()
            .max(1.) as usize;
        let preload_range = search_scope_switch_preload_range(
            anchor_ix,
            viewport_anchor.as_ref().is_some_and(|anchor| anchor.at_end),
            rows.len(),
            table_visible_rows
                .max(measured_visible_rows)
                .max(window_visible_rows),
        );
        let table = self.documents[tab_ix].result_table.clone();
        let request = table
            .read(cx)
            .delegate()
            .stage_row_projection_replacement(&rows, preload_range);
        let document = self.documents[tab_ix].document.clone();
        let matched_rows = self.documents[tab_ix].search_result.line_indices.clone();
        let marked_rows = self.documents[tab_ix].file.marked_rows.clone();
        let matcher = self
            .app_settings
            .highlight_matches
            .then(|| self.documents[tab_ix].search_matcher.clone())
            .flatten();
        let cancellation = Arc::new(AtomicBool::new(false));
        let revision = {
            let tab = &mut self.documents[tab_ix];
            if let Some(previous) = tab
                .result_replace_cancellation
                .replace(cancellation.clone())
            {
                previous.store(true, Ordering::Release);
            }
            tab.result_replace_task.take();
            tab.result_replace_revision = tab.result_replace_revision.saturating_add(1);
            tab.result_replace_revision
        };
        let expected_document = document.clone();
        let task = cx.spawn_in(window, async move |this, cx| {
            let staged = cx
                .background_spawn(async move {
                    let mut reader = LinePreviewReader::default();
                    request.load_cancellable(&cancellation, |source_row, max_bytes| {
                        reader.line_preview(&document, *source_row, max_bytes)
                    })
                })
                .await;
            _ = this.update_in(cx, |this, window, cx| {
                let Some(tab_ix) = this.documents.iter().position(|tab| tab.id == document_id)
                else {
                    return;
                };
                let tab = &this.documents[tab_ix];
                if tab.result_replace_revision != revision
                    || !Arc::ptr_eq(&tab.document, &expected_document)
                    || tab.result_table.read(cx).delegate().projected_rows() != Some(&previous_rows)
                {
                    return;
                }
                this.commit_local_result_replacement(
                    PreparedLocalResultReplacement {
                        document_id,
                        document: expected_document,
                        previous_rows,
                        rows,
                        matched_rows,
                        marked_rows,
                        matcher,
                        staged,
                        viewport_anchor,
                        measured_heights,
                        row_height,
                        word_wrap,
                    },
                    window,
                    cx,
                );
            });
        });
        self.documents[tab_ix].result_replace_task = Some(task);
    }

    fn commit_local_result_replacement(
        &mut self,
        prepared: PreparedLocalResultReplacement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab_ix) = self
            .documents
            .iter()
            .position(|tab| tab.id == prepared.document_id)
        else {
            return;
        };
        let tab = &mut self.documents[tab_ix];
        if !Arc::ptr_eq(&tab.document, &prepared.document)
            || tab.result_table.read(cx).delegate().projected_rows()
                != Some(&prepared.previous_rows)
        {
            return;
        }
        tab.restoring_result_selection = true;
        let active_restored = tab.result_table.update(cx, |table, cx| {
            if tab.view.auto_follow {
                table.delegate().set_active_log_row(None);
            }
            table.delegate_mut().set_matched_rows(prepared.matched_rows);
            table.delegate_mut().set_marked_rows(prepared.marked_rows);
            table.delegate_mut().set_search_matcher(prepared.matcher);
            table
                .delegate_mut()
                .install_row_projection_replacement(prepared.rows, prepared.staged);
            let active_restored = table.sync_active_log_row(cx);
            table.refresh(cx);
            cx.notify();
            active_restored
        });
        if !active_restored {
            tab.restoring_result_selection = false;
        }
        if prepared.word_wrap {
            let table = tab.result_table.read(cx);
            tab.result_viewport.reset_wrapped_with_remapped_heights(
                table.delegate().row_count(),
                prepared.row_height,
                prepared.measured_heights,
                |key| table.delegate().row_ix_for_key(*key),
            );
        } else {
            tab.result_viewport.invalidate_wrapped();
        }
        Self::restore_local_viewport_anchor(
            tab,
            WrappedRegion::Results,
            prepared.viewport_anchor,
            prepared.row_height,
            cx,
        );
        tab.result_replace_task = None;
        tab.result_replace_cancellation = None;
        self.refresh_prepared_local_result_surface_atomically(prepared.document_id, window, cx);
    }

    fn refresh_prepared_local_result_surface_atomically(
        &mut self,
        document_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.global_search.scope != SearchScope::CurrentFile
            || self
                .active_document()
                .is_none_or(|tab| tab.id != document_id)
        {
            return;
        }
        self.bind_active_display_tables(cx);
        Self::refresh_log_surfaces_atomically(
            [self.search_results_viewer.surface.clone()],
            window,
            cx,
        );
    }

    /// Rebinds the stable display hosts to the table entities backing the active projections.
    /// This keeps ordinary table notifications repainting the shared surface after a tab or
    /// search-session switch.
    pub(super) fn bind_active_display_tables(&mut self, cx: &mut Context<Self>) {
        let Some(tab_ix) = self.active_ix else {
            return;
        };
        self.refresh_active_log_search_presentation(cx);
        let log_table = self.documents[tab_ix].log_table.clone();
        self.log_viewer
            .surface
            .update(cx, |surface, cx| surface.bind_table(&log_table, cx));
        match self.global_search.scope {
            SearchScope::CurrentFile => {
                let result_table = self.documents[tab_ix].result_table.clone();
                self.search_results_viewer
                    .surface
                    .update(cx, |surface, cx| surface.bind_table(&result_table, cx));
            }
            SearchScope::AllOpenFiles | SearchScope::Directory => {
                let result_table = self.global_table.clone();
                self.search_results_viewer
                    .surface
                    .update(cx, |surface, cx| surface.bind_table(&result_table, cx));
            }
        }
    }

    /// 正文、当前结果和全局结果共用的一行日志高度。
    ///
    /// 固定行高表格把行高交给布局引擎对齐到设备像素，换行列表则要自己按同一规则对齐，
    /// 否则同一条不换行的日志在两种模式下画出来不一样高。
    pub(super) fn log_row_height(&self) -> Pixels {
        snap_to_device_pixels(
            log_line_height(
                self.app_settings.log_font_size,
                self.app_settings.log_line_spacing,
            ),
            self.scale_factor,
        )
    }

    pub(super) fn wrapped_layout_key(
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

    pub(super) fn invalidate_log_scroll_frame(&mut self, key: (u64, WrappedRegion)) {
        self.pending_log_scroll_frames.clear(key);
    }

    pub(super) fn apply_pending_log_scroll_target(
        &mut self,
        document_id: u64,
        region: WrappedRegion,
        row_height: Pixels,
        cx: &mut Context<Self>,
    ) {
        let key = if region == WrappedRegion::GlobalResults {
            (0, region)
        } else {
            (document_id, region)
        };
        let target = if region == WrappedRegion::GlobalResults {
            take_pending_log_scroll_target(
                &mut self.pending_log_scroll_frames,
                key,
                &self.global_viewport,
            )
        } else {
            let Some(tab_ix) = self.documents.iter().position(|tab| tab.id == document_id) else {
                return;
            };
            let viewport = if region == WrappedRegion::Results {
                &self.documents[tab_ix].result_viewport
            } else {
                &self.documents[tab_ix].log_viewport
            };
            take_pending_log_scroll_target(&mut self.pending_log_scroll_frames, key, viewport)
        };
        let Some(target) = target else {
            return;
        };
        if region == WrappedRegion::GlobalResults {
            self.apply_global_scroll_target(target, row_height, cx);
        } else {
            self.apply_local_scroll_target(document_id, region, target, row_height, cx);
        }
    }

    pub(super) fn apply_local_scroll_target(
        &mut self,
        document_id: u64,
        region: WrappedRegion,
        target: LogScrollFrameTarget,
        row_height: Pixels,
        cx: &App,
    ) {
        let Some(tab) = self.documents.iter().find(|tab| tab.id == document_id) else {
            return;
        };
        let (table, viewport) = if region == WrappedRegion::Results {
            (&tab.result_table, &tab.result_viewport)
        } else {
            (&tab.log_table, &tab.log_viewport)
        };
        viewport.commit_scroll_frame_target(
            target,
            table.read(cx).delegate().row_count(),
            row_height,
        );
    }

    pub(super) fn apply_global_scroll_target(
        &mut self,
        target: LogScrollFrameTarget,
        row_height: Pixels,
        cx: &App,
    ) {
        self.global_group_toggle_task.take();
        self.global_group_toggle_revision = self.global_group_toggle_revision.saturating_add(1);
        self.global_viewport.commit_scroll_frame_target(
            target,
            self.global_table.read(cx).delegate().rows_len(),
            row_height,
        );
    }

    pub(super) fn select_and_center_log_source_row_atomically(
        &mut self,
        document_id: u64,
        source_row: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(tab_ix) = self.documents.iter().position(|tab| tab.id == document_id) else {
            return false;
        };
        self.invalidate_log_scroll_frame((document_id, WrappedRegion::Log));
        self.documents[tab_ix]
            .log_viewport
            .take_pending_scrollbar_offset();
        let selected = self.documents[tab_ix].select_and_center_log_source_row(source_row, cx);
        if selected {
            self.schedule_checkpoint(document_id, window, cx);
            cx.notify();
        }
        selected
    }

    pub(super) fn refresh_visible_line_owner(
        &mut self,
        document_id: u64,
        region: WrappedRegion,
        cx: &mut Context<Self>,
    ) {
        self.invalidate_log_scroll_frame((document_id, region));
        if region == WrappedRegion::GlobalResults {
            self.global_table.update(cx, |table, cx| {
                table.reacquire_visible_log_rows(cx);
            });
            return;
        }
        let Some(tab) = self.documents.iter().find(|tab| tab.id == document_id) else {
            return;
        };
        let table = if region == WrappedRegion::Results {
            tab.result_table.clone()
        } else {
            tab.log_table.clone()
        };
        table.update(cx, |table, cx| {
            table.reacquire_visible_log_rows(cx);
        });
    }

    pub(super) fn refresh_log_surfaces_atomically(
        surfaces: impl IntoIterator<Item = Entity<LogRegionSurface>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for surface in surfaces {
            surface.update(cx, |_, cx| cx.notify());
        }
        // Renderer ownership and every surface that consumes it must invalidate before the next
        // frame so retained subtrees cannot outlive a mode or active-tab transition.
        window.refresh();
    }

    pub(super) fn refresh_active_document_surfaces_atomically(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab_ix) = self.active_ix else {
            return;
        };
        self.bind_active_display_tables(cx);
        let document_id = self.documents[tab_ix].id;
        let surfaces = [
            self.log_viewer.surface.clone(),
            self.search_results_viewer.surface.clone(),
        ];
        for region in [WrappedRegion::Log, WrappedRegion::Results] {
            self.refresh_visible_line_owner(document_id, region, cx);
        }
        Self::refresh_log_surfaces_atomically(surfaces, window, cx);
    }

    pub(super) fn refresh_prepared_active_document_surfaces_atomically(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active_ix.is_none() {
            return;
        }
        self.bind_active_display_tables(cx);
        let surfaces = [
            self.log_viewer.surface.clone(),
            self.search_results_viewer.surface.clone(),
        ];
        Self::refresh_log_surfaces_atomically(surfaces, window, cx);
    }

    pub(super) fn refresh_global_result_surface_atomically(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.bind_active_display_tables(cx);
        self.refresh_visible_line_owner(0, WrappedRegion::GlobalResults, cx);
        Self::refresh_log_surfaces_atomically(
            [self.search_results_viewer.surface.clone()],
            window,
            cx,
        );
    }

    pub(super) fn commit_global_group_toggle(
        &mut self,
        prepared: PreparedGlobalGroupToggle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let applied = self.global_table.update(cx, |table, cx| {
            if !table
                .delegate_mut()
                .apply_group_toggle(prepared.plan, prepared.staged)
            {
                return false;
            }
            table.delegate().clear_row_selection();
            table.clear_selection(cx);
            table.refresh(cx);
            cx.notify();
            true
        });
        if !applied {
            return false;
        }
        if self.global_viewport.is_wrapped() {
            self.prime_global_wrapped_group_toggle(
                prepared.anchor,
                prepared.measured_heights,
                prepared.row_height,
                cx,
            );
        } else {
            self.global_viewport.invalidate_wrapped();
            self.position_global_row_viewport_anchor(prepared.anchor, prepared.row_height, cx);
        }
        Self::refresh_log_surfaces_atomically(
            [self.search_results_viewer.surface.clone()],
            window,
            cx,
        );
        self.schedule_workspace_search_state_save(window, cx);
        cx.notify();
        true
    }

    pub(super) fn prepare_global_group_toggle(
        &mut self,
        document_id: u64,
        anchor: Option<RowViewportAnchor<LogRowKey>>,
        measured_heights: BTreeMap<LogRowKey, Pixels>,
        row_height: Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let scrollbar_key = (0, WrappedRegion::GlobalResults);
        self.global_viewport.take_pending_scrollbar_offset();
        self.invalidate_log_scroll_frame(scrollbar_key);
        let table = self.global_table.clone();
        let (plan, request, documents) = {
            let table = table.read(cx);
            let delegate = table.delegate();
            let Some(plan) = delegate.plan_group_toggle(document_id) else {
                return;
            };
            let row_count = delegate.group_toggle_rows_len(&plan);
            let visible_range = if self.global_viewport.is_wrapped() {
                let viewport_height = self.global_viewport.wrapped_viewport_height();
                let visible_count =
                    (viewport_height / row_height.max(px(1.))).ceil().max(1.) as usize;
                let anchor_ix = anchor
                    .as_ref()
                    .and_then(|anchor| delegate.group_toggle_row_ix_for_key(&plan, anchor.key))
                    .unwrap_or_else(|| {
                        anchor
                            .as_ref()
                            .map_or(0, |anchor| anchor.fallback_ix)
                            .min(row_count.saturating_sub(1))
                    });
                anchor_ix.saturating_sub(visible_count.saturating_add(2))
                    ..anchor_ix
                        .saturating_add(visible_count)
                        .saturating_add(2)
                        .min(row_count)
            } else {
                scrollbar_preload_range(
                    self.global_viewport.committed_scroll_offset(),
                    row_count,
                    self.global_viewport.committed_viewport_height(),
                    row_height,
                )
            };
            let request = delegate.stage_group_toggle_visible_rows(&plan, visible_range);
            let documents = request
                .as_ref()
                .map(|request| delegate.staged_visible_documents(request))
                .unwrap_or_default();
            (plan, request, documents)
        };

        self.global_group_toggle_task.take();
        self.global_group_toggle_revision = self.global_group_toggle_revision.saturating_add(1);
        let revision = self.global_group_toggle_revision;
        let Some(request) = request else {
            self.commit_global_group_toggle(
                PreparedGlobalGroupToggle {
                    plan,
                    staged: None,
                    anchor,
                    measured_heights,
                    row_height,
                },
                window,
                cx,
            );
            return;
        };

        self.global_group_toggle_task = Some(cx.spawn_in(window, async move |this, cx| {
            let staged = cx
                .background_spawn(async move {
                    let mut readers = BTreeMap::<u64, LinePreviewReader>::new();
                    request.load(|(document_id, source_row), max_bytes| {
                        let document = documents.get(document_id)?;
                        readers.entry(*document_id).or_default().line_preview(
                            document,
                            *source_row,
                            max_bytes,
                        )
                    })
                })
                .await;
            _ = this.update_in(cx, |this, window, cx| {
                if this.global_group_toggle_revision != revision
                    || this.global_table.entity_id() != table.entity_id()
                {
                    return;
                }
                this.commit_global_group_toggle(
                    PreparedGlobalGroupToggle {
                        plan,
                        staged: Some(staged),
                        anchor,
                        measured_heights,
                        row_height,
                    },
                    window,
                    cx,
                );
            });
        }));
    }

    pub(super) fn clear_document_visible_lines(
        &mut self,
        document_id: u64,
        cx: &mut Context<Self>,
    ) {
        self.invalidate_log_scroll_frame((document_id, WrappedRegion::Log));
        self.invalidate_log_scroll_frame((document_id, WrappedRegion::Results));
        let Some(tab) = self.documents.iter().find(|tab| tab.id == document_id) else {
            return;
        };
        for table in [tab.log_table.clone(), tab.result_table.clone()] {
            table.update(cx, |table, _| {
                table.delegate_mut().clear_visible_lines();
            });
        }
    }

    pub(super) fn prime_global_wrapped_group_toggle(
        &mut self,
        anchor: Option<RowViewportAnchor<LogRowKey>>,
        measured_heights: BTreeMap<LogRowKey, Pixels>,
        base_height: Pixels,
        cx: &mut Context<Self>,
    ) {
        let count = self.global_table.read(cx).delegate().rows_len();
        {
            let table = self.global_table.read(cx);
            self.global_viewport.reset_wrapped_with_remapped_heights(
                count,
                base_height,
                measured_heights,
                |key| table.delegate().row_ix_for_key(*key),
            );
        }
        if count == 0 {
            return;
        }

        self.position_global_row_viewport_anchor(anchor, base_height, cx);
    }

    pub(super) fn toggle_word_wrap(
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
        if self.global_search.scope.owns_global_word_wrap() {
            self.apply_pending_log_scroll_target(0, WrappedRegion::GlobalResults, row_height, cx);
        }
        if let Some(active_ix) = self.active_ix {
            let document_id = self.documents[active_ix].id;
            for region in [WrappedRegion::Log, WrappedRegion::Results] {
                self.apply_pending_log_scroll_target(document_id, region, row_height, cx);
            }
        }
        let mut refreshed_surfaces = Vec::with_capacity(3);

        if self.global_search.scope.owns_global_word_wrap()
            && self.global_viewport.is_wrapped() != enabled
        {
            let anchor = self.capture_global_row_viewport_anchor(row_height, cx);
            self.refresh_visible_line_owner(0, WrappedRegion::GlobalResults, cx);
            self.global_viewport.set_word_wrap(enabled);
            self.position_global_row_viewport_anchor(anchor, row_height, cx);
            refreshed_surfaces.push(self.search_results_viewer.surface.clone());
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
                let document_id = self.documents[active_ix].id;
                self.refresh_visible_line_owner(document_id, WrappedRegion::Log, cx);
                self.refresh_visible_line_owner(document_id, WrappedRegion::Results, cx);
                let document_id = {
                    let tab = &mut self.documents[active_ix];
                    tab.log_viewport.set_word_wrap(enabled);
                    tab.result_viewport.set_word_wrap(enabled);
                    tab.view.word_wrap = enabled;
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
                refreshed_surfaces.extend([
                    self.log_viewer.surface.clone(),
                    self.search_results_viewer.surface.clone(),
                ]);
                self.schedule_checkpoint(document_id, window, cx);
            }
        }

        if !refreshed_surfaces.is_empty() {
            Self::refresh_log_surfaces_atomically(refreshed_surfaces, window, cx);
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

    pub(super) fn active_navigation_region(&self) -> Option<(u64, WrappedRegion)> {
        let tab = self.active_document()?;
        if self.active_log_region == LogRegion::GlobalResults && self.global_search.results_visible
        {
            return Some((tab.id, WrappedRegion::GlobalResults));
        }
        Some((
            tab.id,
            if tab.view.selection_table == SelectionTable::Results && tab.results_visible {
                WrappedRegion::Results
            } else {
                WrappedRegion::Log
            },
        ))
    }

    pub(super) fn navigate_log_rows(
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

    pub(super) fn select_wrapped_up(
        &mut self,
        _: &SelectUp,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.navigate_log_rows(-1, false, None, cx);
    }

    pub(super) fn select_wrapped_down(
        &mut self,
        _: &SelectDown,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.navigate_log_rows(1, false, None, cx);
    }

    pub(super) fn select_wrapped_page_up(
        &mut self,
        _: &SelectPageUp,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.navigate_log_rows(-1, true, None, cx);
    }

    pub(super) fn select_wrapped_page_down(
        &mut self,
        _: &SelectPageDown,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.navigate_log_rows(1, true, None, cx);
    }

    pub(super) fn select_wrapped_first(
        &mut self,
        _: &SelectFirst,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.navigate_log_rows(-1, false, Some(false), cx);
    }

    pub(super) fn select_wrapped_last(
        &mut self,
        _: &SelectLast,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.navigate_log_rows(1, false, Some(true), cx);
    }
}
