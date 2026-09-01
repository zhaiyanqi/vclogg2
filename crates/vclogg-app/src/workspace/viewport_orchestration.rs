use super::*;

impl Workspace {
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

    pub(super) fn measure_wrapped_line_height(
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

    pub(super) fn prepare_pending_log_scroll_frame(
        &mut self,
        document_id: u64,
        region: WrappedRegion,
        row_height: Pixels,
        window: &Window,
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
            self.prepare_global_scroll_frame(target, row_height, window, cx);
        } else {
            self.prepare_local_scroll_frame(document_id, region, target, row_height, window, cx);
        }
    }

    pub(super) fn prepare_local_scroll_frame(
        &mut self,
        document_id: u64,
        region: WrappedRegion,
        target: LogScrollFrameTarget,
        row_height: Pixels,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab_ix) = self.documents.iter().position(|tab| tab.id == document_id) else {
            return;
        };
        let table = if region == WrappedRegion::Results {
            self.documents[tab_ix].result_table.clone()
        } else {
            self.documents[tab_ix].log_table.clone()
        };
        let (wrapped, viewport_height) = {
            let viewport = if region == WrappedRegion::Results {
                &self.documents[tab_ix].result_viewport
            } else {
                &self.documents[tab_ix].log_viewport
            };
            let wrapped = viewport.is_wrapped();
            let viewport_height = if wrapped {
                viewport.wrapped_viewport_height()
            } else {
                viewport
                    .fixed
                    .scroll_handle
                    .0
                    .borrow()
                    .base_handle
                    .bounds()
                    .size
                    .height
            };
            (wrapped, viewport_height)
        };
        let (row_count, document, request) = {
            let table = table.read(cx);
            let delegate = table.delegate();
            let row_count = delegate.row_count();
            let viewport = if region == WrappedRegion::Results {
                &self.documents[tab_ix].result_viewport
            } else {
                &self.documents[tab_ix].log_viewport
            };
            let range =
                viewport.scroll_frame_preload_range(target, row_count, viewport_height, row_height);
            (
                row_count,
                delegate.visible_document(),
                delegate.stage_visible_rows(range),
            )
        };
        if let Some(request) = request {
            // klogg resolves the final scrollbar position in paintEvent and performs one
            // contiguous visible-window read before presenting the pixmap. Our line reader keeps
            // verified source blocks hot, so doing the staged window synchronously here provides
            // the same one-frame contract without exposing an empty virtual list.
            let mut reader = LinePreviewReader::default();
            let staged = request.load(|source_row, max_bytes| {
                reader.line_preview(&document, *source_row, max_bytes)
            });
            table.update(cx, |table, cx| {
                table.delegate().install_staged_visible_lines(staged);
                table.refresh(cx);
                cx.notify();
            });
        }
        let viewport = if region == WrappedRegion::Results {
            &self.documents[tab_ix].result_viewport
        } else {
            &self.documents[tab_ix].log_viewport
        };
        viewport.commit_scroll_frame_target(target, row_count, row_height);
        if wrapped {
            self.prime_local_wrapped_frame(tab_ix, region, row_height, false, window, cx);
        }
    }

    pub(super) fn prepare_global_scroll_frame(
        &mut self,
        target: LogScrollFrameTarget,
        row_height: Pixels,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        self.global_group_toggle_task.take();
        self.global_group_toggle_revision = self.global_group_toggle_revision.saturating_add(1);
        let wrapped = self.global_viewport.is_wrapped();
        let viewport_height = if wrapped {
            self.global_viewport.wrapped_viewport_height()
        } else {
            self.global_viewport
                .fixed
                .scroll_handle
                .0
                .borrow()
                .base_handle
                .bounds()
                .size
                .height
        };
        let table = self.global_table.clone();
        let (row_count, request, documents) = {
            let table = table.read(cx);
            let delegate = table.delegate();
            let row_count = delegate.rows_len();
            let range = self.global_viewport.scroll_frame_preload_range(
                target,
                row_count,
                viewport_height,
                row_height,
            );
            let request = delegate.stage_visible_rows(range);
            let documents = request
                .as_ref()
                .map(|request| delegate.staged_visible_documents(request))
                .unwrap_or_default();
            (row_count, request, documents)
        };
        if let Some(request) = request {
            let mut readers = BTreeMap::<u64, LinePreviewReader>::new();
            let staged = request.load(|(document_id, source_row), max_bytes| {
                let document = documents.get(document_id)?;
                readers.entry(*document_id).or_default().line_preview(
                    document,
                    *source_row,
                    max_bytes,
                )
            });
            table.update(cx, |table, cx| {
                table.delegate().install_staged_visible_lines(staged);
                table.refresh(cx);
                cx.notify();
            });
        }
        self.global_viewport
            .commit_scroll_frame_target(target, row_count, row_height);
        if wrapped {
            self.prime_global_wrapped_frame(row_height, false, window, cx);
        }
    }

    pub(super) fn schedule_local_visible_lines(
        &mut self,
        document_id: u64,
        region: WrappedRegion,
        visible_range: Range<usize>,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.documents.iter().find(|tab| tab.id == document_id) else {
            return;
        };
        let table = if region == WrappedRegion::Results {
            tab.result_table.clone()
        } else {
            tab.log_table.clone()
        };
        let (request, document) = {
            let table = table.read(cx);
            let delegate = table.delegate();
            let Some(request) = delegate.request_visible_rows(visible_range) else {
                return;
            };
            (request, delegate.visible_document())
        };
        let task = cx.spawn(async move |this, cx| {
            let loaded = cx
                .background_spawn(async move {
                    let mut reader = LinePreviewReader::default();
                    request.load(|source_row, max_bytes| {
                        reader.line_preview(&document, *source_row, max_bytes)
                    })
                })
                .await;
            _ = this.update(cx, |_, cx| {
                table.update(cx, |table, cx| {
                    if table.delegate().install_visible_lines(loaded) {
                        cx.notify();
                    }
                });
                cx.notify();
            });
        });
        self.visible_line_tasks.insert((document_id, region), task);
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
        let Some(row_ix) = self.documents[tab_ix].document.local_row(source_row) else {
            return false;
        };
        let table = self.documents[tab_ix].log_table.clone();
        let document = self.documents[tab_ix].document.clone();
        let row_count = table.read(cx).delegate().row_count();
        let table_visible_rows = table.read(cx).visible_range().rows().len();
        let measured_visible_rows = self
            .row_drag_bounds
            .get(&(document_id, WrappedRegion::Log))
            .map(|bounds| (bounds.size.height / self.log_row_height()).ceil().max(1.) as usize)
            .unwrap_or_default();
        let preload_range = centered_log_jump_preload_range(
            row_ix,
            row_count,
            table_visible_rows.max(measured_visible_rows),
        );
        let staged_request = table.read(cx).delegate().stage_visible_rows(preload_range);

        let revision = {
            let tab = &mut self.documents[tab_ix];
            tab.log_jump_revision = tab.log_jump_revision.saturating_add(1);
            tab.log_jump_task.take();
            tab.log_jump_revision
        };
        let Some(staged_request) = staged_request else {
            return self.documents[tab_ix].select_and_center_log_source_row(source_row, cx);
        };

        let expected_document = document.clone();
        let task = cx.spawn_in(window, async move |this, cx| {
            let staged = cx
                .background_spawn(async move {
                    let mut reader = LinePreviewReader::default();
                    staged_request.load(|source_row, max_bytes| {
                        reader.line_preview(&document, *source_row, max_bytes)
                    })
                })
                .await;
            _ = this.update_in(cx, |this, window, cx| {
                let Some(tab_ix) = this.documents.iter().position(|tab| tab.id == document_id)
                else {
                    return;
                };
                let tab = &mut this.documents[tab_ix];
                if tab.log_jump_revision != revision
                    || !Arc::ptr_eq(&tab.document, &expected_document)
                {
                    return;
                }
                table.update(cx, |table, cx| {
                    table.delegate().install_staged_visible_lines(staged);
                    table.delegate().set_active_log_row(Some(row_ix));
                    table.delegate().settle_table_selection(row_ix);
                    cx.notify();
                });
                tab.log_viewport.center_row(row_ix);
                this.schedule_checkpoint(document_id, window, cx);
                cx.notify();
            });
        });
        self.documents[tab_ix].log_jump_task = Some(task);
        true
    }

    pub(super) fn switch_visible_line_owner(
        &mut self,
        document_id: u64,
        region: WrappedRegion,
        mode: LogViewportMode,
        cx: &mut Context<Self>,
    ) {
        self.invalidate_log_scroll_frame((document_id, region));
        self.visible_line_tasks.remove(&(document_id, region));
        if region == WrappedRegion::GlobalResults {
            self.global_table.update(cx, |table, cx| {
                if mode == LogViewportMode::Fixed {
                    table.reacquire_visible_log_rows(cx);
                } else {
                    table.delegate_mut().reset_visible_line_owner();
                }
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
            if mode == LogViewportMode::Fixed {
                table.reacquire_visible_log_rows(cx);
            } else {
                table.delegate_mut().reset_visible_line_owner();
            }
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
        let row_height = self.log_row_height();
        let (document_id, log_mode, result_mode, surfaces) = {
            let tab = &self.documents[tab_ix];
            (
                tab.id,
                if tab.log_viewport.is_wrapped() {
                    LogViewportMode::Wrapped
                } else {
                    LogViewportMode::Fixed
                },
                if tab.result_viewport.is_wrapped() {
                    LogViewportMode::Wrapped
                } else {
                    LogViewportMode::Fixed
                },
                [tab.log_surface.clone(), tab.result_surface.clone()],
            )
        };

        for (region, mode) in [
            (WrappedRegion::Log, log_mode),
            (WrappedRegion::Results, result_mode),
        ] {
            self.switch_visible_line_owner(document_id, region, mode, cx);
            if mode == LogViewportMode::Wrapped {
                self.prime_local_wrapped_frame(tab_ix, region, row_height, false, window, cx);
            }
        }
        Self::refresh_log_surfaces_atomically(surfaces, window, cx);
    }

    pub(super) fn refresh_prepared_active_document_surfaces_atomically(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab_ix) = self.active_ix else {
            return;
        };
        let row_height = self.log_row_height();
        let (log_wrapped, result_wrapped, surfaces) = {
            let tab = &self.documents[tab_ix];
            (
                tab.log_viewport.is_wrapped(),
                tab.result_viewport.is_wrapped(),
                [tab.log_surface.clone(), tab.result_surface.clone()],
            )
        };

        // The staged windows already own complete first-frame data. Do not run the ordinary
        // owner handoff here: a fixed table with no previous layout would otherwise replace the
        // staged range with its old empty visible range before the new tab is painted.
        for (region, wrapped) in [
            (WrappedRegion::Log, log_wrapped),
            (WrappedRegion::Results, result_wrapped),
        ] {
            if wrapped {
                self.prime_local_wrapped_frame_with_minimum_height(
                    tab_ix,
                    region,
                    row_height,
                    WrappedFramePrimeOptions {
                        minimum_viewport_height: window.viewport_size().height,
                        reset_for_mode_switch: false,
                    },
                    window,
                    cx,
                );
            }
        }
        Self::refresh_log_surfaces_atomically(surfaces, window, cx);
    }

    pub(super) fn refresh_global_result_surface_atomically(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mode = if self.global_viewport.is_wrapped() {
            LogViewportMode::Wrapped
        } else {
            LogViewportMode::Fixed
        };
        self.switch_visible_line_owner(0, WrappedRegion::GlobalResults, mode, cx);
        if mode == LogViewportMode::Wrapped {
            self.prime_global_wrapped_frame(self.log_row_height(), false, window, cx);
        }
        Self::refresh_log_surfaces_atomically([self.global_surface.clone()], window, cx);
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
                window,
                cx,
            );
        }
        Self::refresh_log_surfaces_atomically([self.global_surface.clone()], window, cx);
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
                    self.global_viewport
                        .fixed
                        .scroll_handle
                        .0
                        .borrow()
                        .base_handle
                        .bounds()
                        .size
                        .height,
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
        self.visible_line_tasks
            .remove(&(document_id, WrappedRegion::Log));
        self.visible_line_tasks
            .remove(&(document_id, WrappedRegion::Results));
        self.pending_log_scroll_frames
            .clear((document_id, WrappedRegion::Log));
        self.pending_log_scroll_frames
            .clear((document_id, WrappedRegion::Results));
        let Some(tab) = self.documents.iter().find(|tab| tab.id == document_id) else {
            return;
        };
        for table in [tab.log_table.clone(), tab.result_table.clone()] {
            table.update(cx, |table, _| {
                table.delegate_mut().clear_visible_lines();
            });
        }
    }

    pub(super) fn schedule_global_visible_lines(
        &mut self,
        visible_range: Range<usize>,
        cx: &mut Context<Self>,
    ) {
        let table = self.global_table.clone();
        let (request, documents) = {
            let table = table.read(cx);
            let delegate = table.delegate();
            let Some(request) = delegate.request_visible_rows(visible_range) else {
                return;
            };
            let documents = delegate.visible_documents(&request);
            (request, documents)
        };
        let task = cx.spawn(async move |this, cx| {
            let loaded = cx
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
            _ = this.update(cx, |_, cx| {
                table.update(cx, |table, cx| {
                    if table.delegate().install_visible_lines(loaded) {
                        cx.notify();
                    }
                });
                cx.notify();
            });
        });
        self.visible_line_tasks
            .insert((0, WrappedRegion::GlobalResults), task);
    }

    pub(super) fn prime_local_wrapped_frame(
        &mut self,
        tab_ix: usize,
        region: WrappedRegion,
        base_height: Pixels,
        reset_for_mode_switch: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        self.prime_local_wrapped_frame_with_minimum_height(
            tab_ix,
            region,
            base_height,
            WrappedFramePrimeOptions {
                minimum_viewport_height: px(0.),
                reset_for_mode_switch,
            },
            window,
            cx,
        );
    }

    pub(super) fn prime_local_wrapped_frame_with_minimum_height(
        &mut self,
        tab_ix: usize,
        region: WrappedRegion,
        base_height: Pixels,
        options: WrappedFramePrimeOptions,
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
        let count = table.read(cx).delegate().row_count();
        let range = {
            let wrapped = if region == WrappedRegion::Results {
                &self.documents[tab_ix].result_viewport
            } else {
                &self.documents[tab_ix].log_viewport
            };
            let viewport_height = wrapped
                .committed_viewport_height()
                .max(bounds.map_or(px(0.), |bounds| bounds.size.height))
                .max(options.minimum_viewport_height);
            wrapped.prospective_wrapped_measurement_range(count, viewport_height, base_height)
        };
        self.schedule_local_visible_lines(document_id, region, range.clone(), cx);
        let (content_revision, outer_width, font_size, font_family, rows) = {
            let table = table.read(cx);
            let delegate = table.delegate();
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
        if options.reset_for_mode_switch {
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

    pub(super) fn prime_global_wrapped_frame(
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
        let count = self.global_table.read(cx).delegate().rows_len();
        let viewport_height = self
            .global_viewport
            .committed_viewport_height()
            .max(bounds.map_or(px(0.), |bounds| bounds.size.height));
        let range = self.global_viewport.prospective_wrapped_measurement_range(
            count,
            viewport_height,
            base_height,
        );
        self.schedule_global_visible_lines(range.clone(), cx);
        let (content_revision, outer_width, font_size, font_family, rows) = {
            let table = self.global_table.read(cx);
            let delegate = table.delegate();
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

    pub(super) fn prime_global_wrapped_group_toggle(
        &mut self,
        anchor: Option<RowViewportAnchor<LogRowKey>>,
        measured_heights: BTreeMap<LogRowKey, Pixels>,
        base_height: Pixels,
        window: &Window,
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

        let first_visible = self.global_viewport.wrapped_first_visible_row();
        let visible_range = wrapped_viewport_measurement_range(
            first_visible,
            self.global_viewport.wrapped_viewport_height(),
            base_height,
            count,
        );
        self.schedule_global_visible_lines(visible_range.clone(), cx);
        let (outer_width, font_size, font_family, rows) = {
            let table = self.global_table.read(cx);
            let delegate = table.delegate();
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

    pub(super) fn prime_wrapped_first_frame(
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
            self.prepare_pending_log_scroll_frame(
                0,
                WrappedRegion::GlobalResults,
                row_height,
                window,
                cx,
            );
        }
        if let Some(active_ix) = self.active_ix {
            let document_id = self.documents[active_ix].id;
            for region in [WrappedRegion::Log, WrappedRegion::Results] {
                self.prepare_pending_log_scroll_frame(document_id, region, row_height, window, cx);
            }
        }
        let target_mode = if enabled {
            LogViewportMode::Wrapped
        } else {
            LogViewportMode::Fixed
        };
        let mut refreshed_surfaces = Vec::with_capacity(3);

        if self.global_search.scope.owns_global_word_wrap()
            && self.global_viewport.is_wrapped() != enabled
        {
            let anchor = self.capture_global_row_viewport_anchor(row_height, cx);
            self.switch_visible_line_owner(0, WrappedRegion::GlobalResults, target_mode, cx);
            if enabled {
                self.prime_global_wrapped_frame(row_height, true, window, cx);
            }
            self.global_viewport.set_word_wrap(enabled);
            self.position_global_row_viewport_anchor(anchor, row_height, cx);
            refreshed_surfaces.push(self.global_surface.clone());
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
                self.switch_visible_line_owner(document_id, WrappedRegion::Log, target_mode, cx);
                self.switch_visible_line_owner(
                    document_id,
                    WrappedRegion::Results,
                    target_mode,
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
                refreshed_surfaces.extend([tab.log_surface.clone(), tab.result_surface.clone()]);
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
            if tab.selection_table == SelectionTable::Results && tab.results_visible {
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
