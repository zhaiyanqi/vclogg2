use super::*;

impl Workspace {
    pub(super) fn viewport_anchor_retains_end(
        viewport_is_at_end: bool,
        preferred_row: Option<usize>,
        anchor_row: usize,
    ) -> bool {
        // A visible active row is an explicit navigation anchor. Restoring the bottom instead
        // would discard that row's screen-relative position when the projection changes.
        viewport_is_at_end && preferred_row != Some(anchor_row)
    }

    pub(super) fn highlight_styles(
        highlights: &[(Range<usize>, TextHighlight)],
        cx: &App,
    ) -> Vec<(Range<usize>, HighlightStyle)> {
        highlights
            .iter()
            .cloned()
            .map(|(range, highlight)| (range, text_highlight_style(highlight, cx)))
            .collect()
    }

    pub(super) fn log_region_surface(
        &self,
        _document_id: u64,
        region: WrappedRegion,
    ) -> Option<Entity<LogRegionSurface>> {
        Some(match region {
            WrappedRegion::Log => self.log_viewer.surface.clone(),
            WrappedRegion::Results | WrappedRegion::GlobalResults => {
                self.search_results_viewer.surface.clone()
            }
        })
    }

    pub(super) fn is_text_selection_origin_in_log_region(&self, position: Point<Pixels>) -> bool {
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

    pub(super) fn select_wrapped_log_row(
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
            self.search_results_viewer.focus_handle.clone()
        } else {
            self.log_viewer.focus_handle.clone()
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

    pub(super) fn handle_row_drag_move(
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

    pub(super) fn end_row_drag_selection(
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
        let changed_row_selection = self.row_drag_selection.is_some_and(|drag| {
            drag.document_id == document_id && drag.region == region && drag.changed_row_selection()
        });
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
            if changed_row_selection {
                self.schedule_log_region_state_save(document_id, region, window, cx);
            }
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
        self.schedule_log_region_state_save(document_id, region, window, cx);
    }

    pub(super) fn end_all_row_drag_selection(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.row_drag_selection.is_some() {
            self.advance_row_drag_selection(cx);
        }
        // Result replacement clears the delegate's pointer anchor before MouseUp. Keep the
        // workspace-owned drag target so its text-selection suppression is still released.
        let active_drag = self.row_drag_selection;
        let changed_global_row_selection = active_drag.is_some_and(|drag| {
            drag.region == WrappedRegion::GlobalResults && drag.changed_row_selection()
        });
        let clear_text_selection = self
            .row_drag_selection
            .is_some_and(|drag| drag.mode == RowDragMode::Lines);
        self.row_drag_selection = None;
        if clear_text_selection {
            TextSelection::clear(window, cx);
        }
        let mut ended_selection = false;
        let mut ended_document_selections = BTreeSet::new();
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
                ended_document_selections.insert(tab.id);
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
        for document_id in ended_document_selections {
            self.schedule_checkpoint(document_id, window, cx);
        }
        if ended_global_selection && changed_global_row_selection {
            self.schedule_workspace_search_state_save(window, cx);
        }
    }

    pub(super) fn schedule_row_drag_frame(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    pub(super) fn advance_row_drag_selection(&mut self, cx: &mut Context<Self>) -> bool {
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
        tab.view.auto_follow = false;
        tab.view.selection_table = if drag.region == WrappedRegion::Results {
            SelectionTable::Results
        } else {
            SelectionTable::Log
        };
        self.selected_source_row = source_row;
        keep_scrolling && scroll_changed
    }

    /// Runs from the viewport's prepaint observer before the wrapped-list child is prepainted.
    /// Priming the shared measurements here makes the child's first visible frame self-consistent.
    pub(super) fn update_wrapped_layout(
        &mut self,
        document_id: u64,
        region: WrappedRegion,
        width: Pixels,
        viewport_height: Pixels,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        if width <= px(0.) || viewport_height <= px(0.) {
            return;
        }
        let base_height = self.log_row_height();
        let horizontal_padding = log_cell_horizontal_padding(cx);
        let text_width = (width - horizontal_padding * 2.).max(px(0.));
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
                let (count, font_size, font_family, key, preferred) = {
                    let table = table.read(cx);
                    let delegate = table.delegate();
                    let font_size = delegate.log_font_size();
                    let font_family = delegate.resolved_font_family(cx);
                    (
                        delegate.row_count(),
                        font_size,
                        font_family.clone(),
                        Self::wrapped_layout_key(
                            delegate.content_revision(),
                            width,
                            font_size,
                            font_family,
                            base_height,
                            window.rem_size(),
                            horizontal_padding,
                        ),
                        table.active_log_row(),
                    )
                };
                let viewport = if region == WrappedRegion::Results {
                    &self.documents[tab_ix].result_viewport
                } else {
                    &self.documents[tab_ix].log_viewport
                };
                let changed =
                    viewport.invalidate_wrapped_layout_preserving_position(key, preferred);
                viewport.wrapped_sizes(count, base_height);
                let range = wrapped_viewport_measurement_range(
                    viewport.wrapped_first_visible_row(),
                    viewport_height,
                    base_height,
                    count,
                );
                let unknown_rows = range
                    .filter(|row_ix| !viewport.has_known_wrapped_row_height(*row_ix))
                    .collect::<Vec<_>>();
                let rows = {
                    let table = table.read(cx);
                    unknown_rows
                        .into_iter()
                        .filter_map(|row_ix| {
                            table
                                .delegate()
                                .wrapped_row(row_ix)
                                .map(|row| (row_ix, row.text.display().clone()))
                        })
                        .collect::<Vec<_>>()
                };
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
                viewport.prime_wrapped_measured_heights(count, base_height, heights);
                changed
            }
            WrappedRegion::GlobalResults => {
                if !self.global_viewport.is_wrapped() {
                    return;
                }
                let (count, font_size, font_family, key, preferred) = {
                    let table = self.global_table.read(cx);
                    let delegate = table.delegate();
                    let font_size = delegate.log_font_size();
                    let font_family = delegate.resolved_font_family(cx);
                    (
                        delegate.rows_len(),
                        font_size,
                        font_family.clone(),
                        Self::wrapped_layout_key(
                            delegate.content_revision(),
                            width,
                            font_size,
                            font_family,
                            base_height,
                            window.rem_size(),
                            horizontal_padding,
                        ),
                        table.active_log_row(),
                    )
                };
                let changed = self
                    .global_viewport
                    .invalidate_wrapped_layout_preserving_position(key, preferred);
                self.global_viewport.wrapped_sizes(count, base_height);
                let range = wrapped_viewport_measurement_range(
                    self.global_viewport.wrapped_first_visible_row(),
                    viewport_height,
                    base_height,
                    count,
                );
                let unknown_rows = range
                    .filter(|row_ix| !self.global_viewport.has_known_wrapped_row_height(*row_ix))
                    .collect::<Vec<_>>();
                let rows = {
                    let table = self.global_table.read(cx);
                    unknown_rows
                        .into_iter()
                        .filter_map(|row_ix| match table.delegate().wrapped_row(row_ix)? {
                            WrappedGlobalRow::Match { text, .. } => {
                                Some((row_ix, text.display().clone()))
                            }
                            WrappedGlobalRow::Group { .. } => None,
                        })
                        .collect::<Vec<_>>()
                };
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
                changed
            }
        };
        if changed && let Some(surface) = self.log_region_surface(document_id, region) {
            surface.update(cx, |_, cx| cx.notify());
        }
    }

    pub(super) fn advance_global_row_drag_selection(
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

    pub(super) fn render_wrapped_log_rows(
        &mut self,
        document_id: u64,
        region: WrappedRegion,
        visible_range: Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<VirtualLogRow<LogRowKey>> {
        let _performance_scope = crate::ui_performance::scope("Workspace::render_wrapped_log_rows");
        let Some(tab_ix) = self.documents.iter().position(|tab| tab.id == document_id) else {
            return Vec::new();
        };
        let table = if region == WrappedRegion::Results {
            self.documents[tab_ix].result_table.clone()
        } else {
            self.documents[tab_ix].log_table.clone()
        };
        table.update(cx, |table, _| {
            table.set_visible_range(visible_range.clone())
        });
        self.schedule_local_visible_lines(document_id, region, visible_range.clone(), cx);
        let (
            show_line_numbers,
            show_row_separators,
            show_line_number_row_separators,
            line_number_width,
            line_number_text_color,
            line_number_background_color,
            log_text_color,
            font_size,
            font_family,
            max_line_columns,
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
                delegate.log_text_color(cx),
                delegate.log_font_size(),
                delegate.resolved_font_family(cx),
                delegate.max_line_columns(),
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
        let viewport = if region == WrappedRegion::Results {
            &self.documents[tab_ix].result_viewport
        } else {
            &self.documents[tab_ix].log_viewport
        };
        let word_wrap = viewport.is_wrapped();
        let horizontal_offset = viewport.horizontal_offset();
        let message_width =
            message_column_width(max_line_columns, font_family.clone(), font_size, cx);
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
                let row_key = table.read(cx).delegate().stable_row_key(row_ix)?;
                let row = table.read(cx).delegate().row(row_ix)?;
                let selected_above =
                    row_ix > 0 && table.read(cx).delegate().is_row_selected(row_ix - 1);
                let selected_below = row_ix + 1 < table.read(cx).delegate().row_count()
                    && table.read(cx).delegate().is_row_selected(row_ix + 1);
                let source_row = row.source_row;
                let source_unavailable = row.source_unavailable;
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
                let log_level_style = (!source_unavailable)
                    .then_some(row.log_level_style)
                    .flatten();
                let row_bounds = rendered_row_bounds.clone();
                let line = SelectableLogText::new(
                    selection,
                    source_row as u64,
                    row.text,
                    styled_text,
                    ui_theme::text_selection_highlight(cx),
                )
                .suppress_selection(suppress_text_selection)
                .word_boundary_characters(self.app_settings.word_boundary_characters.clone());
                Some(VirtualLogRow::new(
                    row_ix,
                    row_key,
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
                        .when(!word_wrap, |row| row.h(base_height).overflow_hidden())
                        .flex()
                        .items_start()
                        .when_some(log_level_style, |row, style| {
                            row.bg(style.background)
                                .child(log_level_accent_overlay(style.foreground))
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
                                .when(word_wrap, |cell| cell.whitespace_normal())
                                .when(!word_wrap, |cell| cell.whitespace_nowrap())
                                .px(log_cell_horizontal_padding(cx))
                                .text_color(
                                    log_level_style
                                        .map_or(log_text_color, |style| style.foreground),
                                )
                                .text_size(px(font_size as f32))
                                .line_height(base_height)
                                .font_family(font_family.clone())
                                .when(source_unavailable, |cell| {
                                    cell.text_color(cx.theme().danger)
                                })
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
                                .child(
                                    div()
                                        .relative()
                                        .when(!word_wrap, |content| {
                                            content.left(-horizontal_offset).w(message_width)
                                        })
                                        .child(line),
                                ),
                        )
                        .child(log_fixed_column_divider_overlay(fixed_columns_width, cx)),
                ))
            })
            .collect()
    }

    pub(super) fn render_wrapped_log_table(
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
        let count = VirtualLogListDelegate::row_count(delegate);
        let base_height = snap_to_device_pixels(delegate.minimum_row_height(), self.scale_factor);
        if count == 0 {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_2()
                .text_color(cx.theme().muted_foreground)
                .child(delegate.empty_message())
                .into_any_element();
        }
        let fixed_columns_width = line_marker_column_width()
            + if delegate.show_line_numbers() {
                px(delegate.line_number_width() as f32)
            } else {
                px(0.)
            };
        let content_width = if (if region == WrappedRegion::Results {
            &tab.result_viewport
        } else {
            &tab.log_viewport
        })
        .is_wrapped()
        {
            px(0.)
        } else {
            delegate.unwrapped_content_width(cx)
        };
        let wrapped = if region == WrappedRegion::Results {
            &tab.result_viewport
        } else {
            &tab.log_viewport
        };
        let word_wrap = wrapped.is_wrapped();
        if wrapped.wrapped_base_height() != base_height {
            wrapped.ensure_wrapped_measurement_anchor(table.read(cx).active_log_row());
        }
        wrapped.wrapped_sizes(count, base_height);
        let list_scroll = wrapped.wrapped_scroll_handle();
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
                    .key_context("VirtualLogList")
                    .child(crate::ui_performance::element(
                        "VirtualLogList::request_layout",
                        "VirtualLogList::prepaint",
                        "VirtualLogList::paint",
                        v_virtual_log_list(
                            surface,
                            list_id,
                            list_scroll,
                            count,
                            base_height,
                            content_width,
                            move |_, range, window, cx| {
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
                            },
                        )
                        .size_full()
                        .when(!word_wrap, |list| list.pb(Scrollbar::width())),
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
            )
            .when(!word_wrap, |container| {
                container.child(
                    div()
                        .absolute()
                        .left(fixed_columns_width)
                        .right(Scrollbar::width())
                        .bottom_0()
                        .h(Scrollbar::width())
                        .bg(scrollbar_background)
                        .child(
                            persistent_log_scrollbar(
                                Scrollbar::horizontal(&logical_scroll)
                                    .id(format!(
                                        "log-horizontal-scrollbar-{document_id}-{}",
                                        region as u8
                                    ))
                                    .viewport_from_layout(),
                                scrollbar_background,
                            )
                            .max_fps(60),
                        ),
                )
            });
        crate::ui_performance::element(
            "WrappedLogTable::request_layout",
            "WrappedLogTable::prepaint",
            "WrappedLogTable::paint",
            element,
        )
        .into_any_element()
    }

    pub(super) fn select_wrapped_global_row(
        &mut self,
        row_ix: usize,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.search_results_viewer.focus_handle.focus(window, cx);
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

    pub(super) fn prepare_wrapped_log_context(
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
            self.search_results_viewer.focus_handle.clone()
        } else {
            self.log_viewer.focus_handle.clone()
        };
        focus.focus(window, cx);
        tab.view.selection_table = if region == WrappedRegion::Results {
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

    pub(super) fn context_color_target(
        &self,
        selected_text: Option<&str>,
        cx: &App,
    ) -> std::result::Result<(usize, ColorKeywordTarget), String> {
        if self.active_log_region == LogRegion::GlobalResults {
            let selected_groups = self
                .global_table
                .read(cx)
                .delegate()
                .selected_match_groups();
            let Some((document_id, rows)) = selected_groups.first() else {
                return Err(crate::tr!("请先选择日志行", "Select log lines first").to_string());
            };
            if let Some(text) = selected_text.map(str::trim).filter(|text| !text.is_empty()) {
                let target_ix = self
                    .presentation_document_ix_for_global_result(*document_id)
                    .or(self.active_ix)
                    .ok_or_else(|| {
                        crate::tr!(
                            "当前没有可承载颜色规则的日志文件",
                            "There is no open log file to own the color rule"
                        )
                        .to_string()
                    })?;
                let tab = &self.documents[target_ix];
                return Ok((
                    target_ix,
                    ColorKeywordTarget {
                        document_id: tab.id,
                        document: tab.document.clone(),
                        selection: ColorKeywordSelection::Text(text.to_string()),
                    },
                ));
            }
            if selected_groups.len() > 1 {
                return Err(crate::tr!(
                    "颜色标签一次只能应用到同一文件的全局结果",
                    "A color label can be applied only to global results from one file at a time"
                )
                .to_string());
            }
            let active_ix = self
                .presentation_document_ix_for_global_result(*document_id)
                .ok_or_else(|| {
                    if self.global_search.scope == SearchScope::Directory {
                        crate::tr!(
                            "请重新搜索，或打开与该结果内容一致的文件后再应用颜色标签",
                            "Search again, or open the same file snapshot before applying a color label"
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
            let tab = &self.documents[active_ix];
            let selection = ColorKeywordSelection::Rows(rows.clone());
            return Ok((
                active_ix,
                ColorKeywordTarget {
                    document_id: tab.id,
                    document: tab.document.clone(),
                    selection,
                },
            ));
        }
        let active_ix = self.active_ix.ok_or_else(|| {
            crate::tr!("当前没有活动日志文件", "There is no active log file").to_string()
        })?;
        let tab = &self.documents[active_ix];
        let selection = selected_text
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map_or_else(
                || ColorKeywordSelection::Rows(tab.selected_source_rows_compressed(cx)),
                |text| ColorKeywordSelection::Text(text.to_string()),
            );
        Ok((
            active_ix,
            ColorKeywordTarget {
                document_id: tab.id,
                document: tab.document.clone(),
                selection,
            },
        ))
    }

    pub(super) fn start_color_rule_action(
        &mut self,
        target: ColorKeywordTarget,
        action: ColorRuleAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cancel_color_rule_action();
        let revision = self.color_rule_revision;
        let collect_keywords = !matches!(
            &action,
            ColorRuleAction::Apply {
                clear_all: true,
                ..
            }
        );
        let Some(tab) = self.documents.iter().find(|tab| {
            tab.id == target.document_id && Arc::ptr_eq(&tab.document, &target.document)
        }) else {
            window.push_notification(
                crate::tr!(
                    "目标日志已刷新或关闭，请重新选择",
                    "The target log was refreshed or closed. Select it again."
                ),
                cx,
            );
            return;
        };
        let rules = tab.file.keyword_color_rules.clone();
        let propagation_scope = matches!(target.selection, ColorKeywordSelection::Text(ref text) if !text.trim().is_empty())
            .then_some(self.global_search.scope)
            .filter(|scope| {
                matches!(scope, SearchScope::AllOpenFiles | SearchScope::Directory)
                    && self.global_search.result_scope == Some(*scope)
            });
        let propagation_targets = propagation_scope.map_or_else(Vec::new, |scope| {
            self.documents
                .iter()
                .filter(|candidate| candidate.id != target.document_id)
                .filter(|candidate| match scope {
                    SearchScope::AllOpenFiles => {
                        self.global_search.results.get(&candidate.id).is_some()
                    }
                    SearchScope::Directory => self.global_search.results.values().any(|result| {
                        result_snapshot_matches_document(
                            &result.path,
                            &result.document,
                            &candidate.document,
                        )
                    }),
                    SearchScope::CurrentFile => false,
                })
                .map(|candidate| ColorRulePropagationTarget {
                    document_id: candidate.id,
                    document: candidate.document.clone(),
                    expected_rules: candidate.file.keyword_color_rules.clone(),
                })
                .collect()
        });
        let session_target = propagation_scope.map(|scope| {
            let expected_rules = match scope {
                SearchScope::AllOpenFiles => self
                    .global_search
                    .all_open_context
                    .keyword_color_rules
                    .clone(),
                SearchScope::Directory => self
                    .global_search
                    .directory_context
                    .keyword_color_rules
                    .clone(),
                SearchScope::CurrentFile => Vec::new(),
            };
            ColorRuleSessionTarget {
                scope,
                expected_revision: self.global_search.revision,
                expected_rules,
            }
        });
        let labels = self.color_labels.clone();
        let last_color_label_id = self.last_color_label_id.clone();
        let cancellation = SearchCancellation::default();
        self.color_rule_cancellation = Some(cancellation.clone());
        self.color_rule_task = Some(cx.spawn_in(window, async move |this, cx| {
            let prepared = cx
                .background_spawn(async move {
                    prepare_color_rule_update(
                        ColorRuleUpdateInput {
                            target,
                            collect_keywords,
                            action,
                            rules,
                            labels,
                            last_color_label_id,
                            propagation_targets,
                            session_target,
                        },
                        &cancellation,
                    )
                })
                .await;
            _ = this.update_in(cx, |this, window, cx| {
                if this.color_rule_revision != revision {
                    return;
                }
                this.color_rule_task = None;
                this.color_rule_cancellation = None;
                let prepared = match prepared {
                    DocumentLineTask::Completed(prepared) => prepared,
                    DocumentLineTask::Cancelled => return,
                    DocumentLineTask::SourceUnavailable => {
                        window.push_notification(
                            crate::tr!(
                                "所选日志的文件内容已改变，请重新加载后再应用颜色标签",
                                "The selected log file changed. Reload it before applying a color label."
                            ),
                            cx,
                        );
                        return;
                    }
                };
                this.finish_color_rule_update(prepared, window, cx);
            });
        }));
    }

    pub(super) fn cancel_color_rule_action(&mut self) {
        self.color_rule_revision = self.color_rule_revision.saturating_add(1);
        if let Some(cancellation) = self.color_rule_cancellation.take() {
            cancellation.cancel();
        }
        self.color_rule_task = None;
    }

    pub(super) fn finish_color_rule_update(
        &mut self,
        prepared: PreparedColorRuleUpdate,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(active_ix) = self.documents.iter().position(|tab| {
            tab.id == prepared.document_id && Arc::ptr_eq(&tab.document, &prepared.document)
        }) else {
            window.push_notification(
                crate::tr!(
                    "目标日志已刷新或关闭，请重新选择",
                    "The target log was refreshed or closed. Select it again."
                ),
                cx,
            );
            return;
        };
        if self.documents[active_ix].file.keyword_color_rules != prepared.expected_rules
            || self.color_labels != prepared.expected_labels
        {
            window.push_notification(
                crate::tr!(
                    "颜色设置已发生变化，请重新应用",
                    "Color settings changed. Apply the selection again."
                ),
                cx,
            );
            return;
        }
        let propagated_file_indices = prepared
            .propagated_files
            .iter()
            .map(|propagated| {
                self.documents.iter().position(|tab| {
                    tab.id == propagated.document_id
                        && Arc::ptr_eq(&tab.document, &propagated.document)
                        && tab.file.keyword_color_rules == propagated.expected_rules
                })
            })
            .collect::<Option<Vec<_>>>();
        let session_is_current = prepared.search_session.as_ref().is_none_or(|session| {
            if self.global_search.scope != session.scope
                || self.global_search.result_scope != Some(session.scope)
                || self.global_search.revision != session.expected_revision
            {
                return false;
            }
            let current_rules = match session.scope {
                SearchScope::AllOpenFiles => {
                    &self.global_search.all_open_context.keyword_color_rules
                }
                SearchScope::Directory => &self.global_search.directory_context.keyword_color_rules,
                SearchScope::CurrentFile => return false,
            };
            current_rules == &session.expected_rules
        });
        let Some(propagated_file_indices) = propagated_file_indices.filter(|_| session_is_current)
        else {
            window.push_notification(
                crate::tr!(
                    "搜索结果或颜色设置已发生变化，请重新应用",
                    "Search results or color settings changed. Apply the selection again."
                ),
                cx,
            );
            return;
        };
        let notification = match &prepared.outcome {
            ColorRuleOutcome::EmptyKeywords => {
                window.push_notification(
                    crate::tr!(
                        "请先选择包含文字的日志行",
                        "Select log lines containing text first"
                    ),
                    cx,
                );
                return;
            }
            ColorRuleOutcome::MissingLabels => {
                window.push_notification(
                    crate::tr!(
                        "请先在“颜色标签…”中添加标签",
                        "Add a label in Color labels… first"
                    ),
                    cx,
                );
                return;
            }
            ColorRuleOutcome::MissingLabel => {
                window.push_notification(
                    crate::tr!("颜色标签已不存在", "The color label no longer exists"),
                    cx,
                );
                return;
            }
            ColorRuleOutcome::CycleRemoved { count } => crate::tr_args!(
                "已移除 {} 行文字的颜色标签",
                "Removed color labels from {} lines of text",
                count
            ),
            ColorRuleOutcome::CycleApplied { label, count } => crate::tr_args!(
                "已用“{}”高亮 {} 行文字",
                "Highlighted “{}” in {} lines of text",
                label.localized_name(),
                count
            ),
            ColorRuleOutcome::Applied => {
                crate::tr!("已应用颜色标签", "Color label applied").to_string()
            }
            ColorRuleOutcome::Removed => {
                crate::tr!("已移除颜色标签", "Color label removed").to_string()
            }
            ColorRuleOutcome::Cleared => crate::tr!(
                "已清除当前文件的所有颜色",
                "Cleared all colors from the current file"
            )
            .to_string(),
        };
        let Some(resolved) = prepared.resolved else {
            debug_assert!(
                false,
                "successful color updates must resolve their matchers"
            );
            return;
        };
        self.documents[active_ix].file.keyword_color_rules = prepared.rules;
        self.documents[active_ix].file.resolved_color_rules = resolved.clone();
        self.last_color_label_id = prepared.last_color_label_id;
        for table in [
            self.documents[active_ix].log_table.clone(),
            self.documents[active_ix].result_table.clone(),
        ] {
            table.update(cx, |table, cx| {
                table.delegate_mut().set_color_rules(resolved.clone());
                table.refresh(cx);
            });
        }
        let mut document_ids = vec![self.documents[active_ix].id];
        for (propagated, document_ix) in prepared
            .propagated_files
            .into_iter()
            .zip(propagated_file_indices)
        {
            self.documents[document_ix].file.keyword_color_rules = propagated.rules;
            self.documents[document_ix].file.resolved_color_rules = propagated.resolved.clone();
            for table in [
                self.documents[document_ix].log_table.clone(),
                self.documents[document_ix].result_table.clone(),
            ] {
                let color_rules = propagated.resolved.clone();
                table.update(cx, |table, cx| {
                    table.delegate_mut().set_color_rules(color_rules);
                    table.refresh(cx);
                });
            }
            document_ids.push(propagated.document_id);
        }
        let search_session_changed = if let Some(session) = prepared.search_session {
            let context = match session.scope {
                SearchScope::AllOpenFiles => &mut self.global_search.all_open_context,
                SearchScope::Directory => &mut self.global_search.directory_context,
                SearchScope::CurrentFile => unreachable!(),
            };
            context.keyword_color_rules = session.rules;
            context.resolved_color_rules = session.resolved;
            true
        } else {
            false
        };
        self.refresh_global_result_rows(window, cx);
        self.refresh_active_log_search_presentation(cx);
        for document_id in document_ids {
            self.schedule_checkpoint(document_id, window, cx);
        }
        if search_session_changed {
            self.schedule_workspace_search_state_save(window, cx);
        }
        window.push_notification(notification, cx);
        cx.notify();
    }

    pub(super) fn open_document_ix_for_global_result(&self, document_id: u64) -> Option<usize> {
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

    pub(super) fn presentation_document_ix_for_global_result(
        &self,
        document_id: u64,
    ) -> Option<usize> {
        let document_ix = self.open_document_ix_for_global_result(document_id)?;
        let Some(result) = self.global_search.results.get(&document_id) else {
            return Some(document_ix);
        };
        let open_document = &self.documents.get(document_ix)?.document;
        result_snapshot_matches_document(&result.path, &result.document, open_document)
            .then_some(document_ix)
    }

    pub(super) fn resolve_global_mark_targets(
        &self,
        selected_rows: &BTreeMap<u64, CompressedRows>,
    ) -> Option<BTreeMap<u64, CompressedRows>> {
        let directory_results = self.global_search.result_scope == Some(SearchScope::Directory);
        group_result_rows_by_document(selected_rows, |result_document_id, rows| {
            let document_ix = if directory_results {
                self.presentation_document_ix_for_global_result(result_document_id)?
            } else {
                self.documents
                    .iter()
                    .position(|tab| tab.id == result_document_id)?
            };
            let tab = self.documents.get(document_ix)?;
            let first = rows.first()?;
            let last = rows.get(rows.len().saturating_sub(1))?;
            if !tab.document.contains_source_row(first) || !tab.document.contains_source_row(last) {
                return None;
            }
            Some(tab.id)
        })
    }

    pub(super) fn apply_context_color_label(
        &mut self,
        label_id: Option<String>,
        selected_text: Option<String>,
        clear_all: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (_, target) = match self.context_color_target(selected_text.as_deref(), cx) {
            Ok(target) => target,
            Err(message) => {
                window.push_notification(message, cx);
                return;
            }
        };
        self.start_color_rule_action(
            target,
            ColorRuleAction::Apply {
                label_id,
                clear_all,
            },
            window,
            cx,
        );
    }

    pub(super) fn context_mark_label(&self, cx: &App) -> &'static str {
        if self.active_log_region == LogRegion::GlobalResults {
            let selected = self.global_table.read(cx).delegate().selection_snapshot();
            if let Some(targets) = (!selected.is_empty())
                .then(|| self.resolve_global_mark_targets(&selected))
                .flatten()
                && targets.iter().all(|(document_id, rows)| {
                    self.documents
                        .iter()
                        .find(|tab| tab.id == *document_id)
                        .is_some_and(|tab| tab.file.marked_rows.contains_all(rows))
                })
            {
                crate::tr!("取消标记", "Unmark")
            } else {
                crate::tr!("标记", "Mark")
            }
        } else if self.active_document().is_some_and(|tab| {
            let rows = tab.selected_source_rows_compressed(cx);
            !rows.is_empty() && tab.file.marked_rows.contains_all(&rows)
        }) {
            crate::tr!("取消标记", "Unmark")
        } else {
            crate::tr!("标记", "Mark")
        }
    }

    pub(super) fn context_color_label_id(
        &self,
        selected_text: Option<&str>,
        cx: &App,
    ) -> Option<String> {
        let selected_text = selected_text
            .map(str::trim)
            .filter(|text| !text.is_empty())?;
        let session_rules: &[KeywordColorRule] = match self.global_search.scope {
            SearchScope::AllOpenFiles => &self.global_search.all_open_context.keyword_color_rules,
            SearchScope::Directory => &self.global_search.directory_context.keyword_color_rules,
            SearchScope::CurrentFile => &[],
        };
        let session_rule = session_rules
            .iter()
            .find(|rule| {
                rule.enabled && rule.case_sensitive && rule.keyword.as_str() == selected_text
            })
            .and_then(|rule| rule.label_id.clone());
        if session_rule.is_some() {
            return session_rule;
        }
        let (tab_ix, _) = self.context_color_target(Some(selected_text), cx).ok()?;
        self.documents[tab_ix]
            .file
            .keyword_color_rules
            .iter()
            .find(|rule| {
                rule.enabled && rule.case_sensitive && rule.keyword.as_str() == selected_text
            })
            .and_then(|rule| rule.label_id.clone())
    }

    pub(super) fn build_log_context_menu(
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
        let color_state_known = selected_text.is_some();
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
                            .checked(color_state_known && current_label_id.is_none())
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
                            .bg(color_with_alpha(
                                label.background_color,
                                label.background_alpha,
                            ));
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

    pub(super) fn capture_local_row_viewport_anchor(
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

    pub(super) fn capture_local_viewport_anchor(
        tab: &DocumentTab,
        region: WrappedRegion,
        row_height: Pixels,
        cx: &App,
    ) -> Option<ViewportAnchor<LogRowKey>> {
        let anchor = Self::capture_local_row_viewport_anchor(tab, region, row_height, cx)?;
        let (table, viewport) = if region == WrappedRegion::Results {
            (&tab.result_table, &tab.result_viewport)
        } else {
            (&tab.log_table, &tab.log_viewport)
        };
        let preferred_row = table.read(cx).active_log_row();
        Some(ViewportAnchor {
            key: anchor.key,
            viewport_y: anchor.viewport_y,
            at_end: Self::viewport_anchor_retains_end(
                viewport.is_at_end(),
                preferred_row,
                anchor.fallback_ix,
            ),
            fallback_ix: anchor.fallback_ix,
        })
    }

    pub(super) fn capture_persisted_local_viewport(
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
        Some(
            ViewportBookmark::new(
                source_row,
                position.viewport_y.as_f32(),
                viewport.horizontal_offset().as_f32(),
                viewport.is_at_end(),
            )
            .with_anchor_row_height(
                viewport
                    .effective_row_height(position.row_ix, row_height)
                    .as_f32(),
            ),
        )
    }

    pub(super) fn restore_persisted_local_viewport(
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
        let key = LogRowKey::Row {
            document_id: tab.id,
            source_row: bookmark.anchor_source_row,
        };
        let row_count = table.read(cx).delegate().row_count();
        let restored_ix = table.read(cx).delegate().row_ix_for_key(key);
        let fallback_ix = restored_ix.unwrap_or_default();
        let viewport_y = px(bookmark.anchor_viewport_y());
        if viewport.is_wrapped() {
            if let Some(restored_ix) = restored_ix {
                // A negative viewport position means the visible frame begins inside a wrapped
                // logical row. Prime its persisted height before resolving the sparse scroll
                // offset; otherwise the row is initially treated as one line and can be skipped
                // before its current layout reports the real height. The offset-derived minimum
                // keeps pre-height bookmarks compatible.
                let minimum_visible_height = (-viewport_y + row_height).max(row_height);
                let anchor_height = bookmark
                    .anchor_row_height()
                    .map(px)
                    .unwrap_or(row_height)
                    .max(row_height)
                    .max(minimum_visible_height);
                viewport.prime_wrapped_measured_heights(
                    row_count,
                    row_height,
                    [(restored_ix, anchor_height)],
                );
            } else {
                viewport.wrapped_sizes(row_count, row_height);
            }
        }
        Self::restore_local_viewport_anchor(
            tab,
            region,
            Some(ViewportAnchor {
                key,
                viewport_y,
                at_end: bookmark.at_end,
                fallback_ix,
            }),
            row_height,
            cx,
        );
        viewport.set_horizontal_offset(px(bookmark.horizontal_offset()));
    }

    pub(super) fn capture_global_row_viewport_anchor(
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

    pub(super) fn capture_global_viewport_anchor(
        &self,
        row_height: Pixels,
        cx: &App,
    ) -> Option<ViewportAnchor<LogRowKey>> {
        let anchor = self.capture_global_row_viewport_anchor(row_height, cx)?;
        let preferred_row = self.global_table.read(cx).active_log_row();
        Some(ViewportAnchor {
            key: anchor.key,
            viewport_y: anchor.viewport_y,
            at_end: Self::viewport_anchor_retains_end(
                self.global_viewport.is_at_end(),
                preferred_row,
                anchor.fallback_ix,
            ),
            fallback_ix: anchor.fallback_ix,
        })
    }

    pub(super) fn position_local_row_viewport_anchor(
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
        let row_ix = {
            let table = table.read(cx);
            let delegate = table.delegate();
            let Some(row_ix) = delegate.nearest_row_ix_for_key(anchor.key).or_else(|| {
                let row_count = delegate.row_count();
                (row_count > 0).then(|| anchor.fallback_ix.min(row_count - 1))
            }) else {
                return;
            };
            row_ix
        };
        viewport.restore_viewport(row_ix, anchor.viewport_y, false, row_height);
    }

    pub(super) fn restore_local_viewport_anchor(
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

    pub(super) fn position_global_row_viewport_anchor(
        &self,
        anchor: Option<RowViewportAnchor<LogRowKey>>,
        row_height: Pixels,
        cx: &mut App,
    ) {
        let Some(anchor) = anchor else {
            return;
        };
        let row_ix = {
            let table = self.global_table.read(cx);
            let delegate = table.delegate();
            let Some(row_ix) = delegate
                .nearest_row_ix_for_key(anchor.key)
                .or_else(|| delegate.closest_match_row(anchor.fallback_ix))
                .or_else(|| {
                    let row_count = delegate.rows_len();
                    (row_count > 0).then(|| anchor.fallback_ix.min(row_count - 1))
                })
            else {
                return;
            };
            row_ix
        };
        self.global_viewport
            .restore_viewport(row_ix, anchor.viewport_y, false, row_height);
    }

    pub(super) fn restore_global_viewport_anchor(
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

    pub(super) fn extend_log_selection(
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

    pub(super) fn extend_selection_up(
        &mut self,
        _: &ExtendSelectionUp,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.extend_log_selection(-1, false, None, cx);
    }

    pub(super) fn extend_selection_down(
        &mut self,
        _: &ExtendSelectionDown,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.extend_log_selection(1, false, None, cx);
    }

    pub(super) fn extend_selection_page_up(
        &mut self,
        _: &ExtendSelectionPageUp,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.extend_log_selection(-1, true, None, cx);
    }

    pub(super) fn extend_selection_page_down(
        &mut self,
        _: &ExtendSelectionPageDown,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.extend_log_selection(1, true, None, cx);
    }

    pub(super) fn extend_selection_first(
        &mut self,
        _: &ExtendSelectionFirst,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.extend_log_selection(-1, false, Some(false), cx);
    }

    pub(super) fn extend_selection_last(
        &mut self,
        _: &ExtendSelectionLast,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.extend_log_selection(1, false, Some(true), cx);
    }

    pub(super) fn prepare_wrapped_global_context(
        &mut self,
        row_ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.search_results_viewer.focus_handle.focus(window, cx);
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

    pub(super) fn prepare_wrapped_global_group_context(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.search_results_viewer.focus_handle.focus(window, cx);
        self.remember_user_log_region(LogRegion::GlobalResults);
        self.global_table.update(cx, |table, cx| {
            table.delegate().clear_row_selection();
            table.clear_selection(cx);
        });
        cx.notify();
    }

    pub(super) fn render_wrapped_global_rows(
        &mut self,
        visible_range: Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<VirtualLogRow<LogRowKey>> {
        let _performance_scope =
            crate::ui_performance::scope("Workspace::render_wrapped_global_rows");
        self.global_table.update(cx, |table, _| {
            table.set_visible_range(visible_range.clone())
        });
        self.schedule_global_visible_lines(visible_range.clone(), cx);
        let (
            font_size,
            font_family,
            line_number_width,
            line_number_text_color,
            line_number_background_color,
            log_text_color,
            show_line_number_row_separators,
            show_row_separators,
            max_line_columns,
        ) = {
            let table = self.global_table.read(cx);
            (
                table.delegate().log_font_size(),
                table.delegate().resolved_font_family(cx),
                table.delegate().line_number_width(),
                table.delegate().line_number_text_color(cx),
                table.delegate().line_number_background_color(cx),
                table.delegate().log_text_color(cx),
                table.delegate().show_line_number_row_separators(),
                table.delegate().show_row_separators(),
                table.delegate().max_line_columns(),
            )
        };
        let base_height = self.log_row_height();
        let marker_width = line_marker_column_width();
        let fixed_columns_width = marker_width + px(line_number_width as f32);
        let word_wrap = self.global_viewport.is_wrapped();
        let horizontal_offset = self.global_viewport.horizontal_offset();
        let message_width =
            message_column_width(max_line_columns, font_family.clone(), font_size, cx);
        let suppress_text_selection = self.row_drag_selection.is_some_and(|drag| {
            drag.region == WrappedRegion::GlobalResults && drag.mode == RowDragMode::Lines
        });
        self.global_viewport
            .retain_wrapped_visible_rows(&visible_range);
        let rendered_row_bounds = self.global_viewport.wrapped_row_bounds();

        visible_range
            .filter_map(|row_ix| {
                let row =
                    VirtualLogListDelegate::row(self.global_table.read(cx).delegate(), row_ix)?;
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
                    } => Some(VirtualLogRow::new(
                        row_ix,
                        LogRowKey::FileGroup { document_id },
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
                            ),
                    )),
                    WrappedGlobalRow::Match {
                        document_id,
                        source_row,
                        text,
                        selected,
                        marked,
                        matched,
                        log_level_style,
                        source_unavailable,
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
                        let log_level_style =
                            (!source_unavailable).then_some(log_level_style).flatten();
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
                        .suppress_selection(suppress_text_selection);
                        Some(VirtualLogRow::new(
                            row_ix,
                            LogRowKey::Row {
                                document_id,
                                source_row,
                            },
                            div()
                                .id(format!("wrapped-global-result-{document_id}-{source_row}"))
                                .on_prepaint(move |bounds, _, _| {
                                    row_bounds.borrow_mut().insert(row_ix, bounds);
                                })
                                .relative()
                                .w_full()
                                .min_h(base_height)
                                .when(!word_wrap, |row| row.h(base_height).overflow_hidden())
                                .flex()
                                .items_start()
                                .when_some(log_level_style, |row, style| {
                                    row.bg(style.background)
                                        .child(log_level_accent_overlay(style.foreground))
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
                                        .when(word_wrap, |cell| cell.whitespace_normal())
                                        .when(!word_wrap, |cell| cell.whitespace_nowrap())
                                        .px(log_cell_horizontal_padding(cx))
                                        .text_color(
                                            log_level_style
                                                .map_or(log_text_color, |style| style.foreground),
                                        )
                                        .text_size(px(font_size as f32))
                                        .line_height(base_height)
                                        .font_family(font_family.clone())
                                        .when(source_unavailable, |cell| {
                                            cell.text_color(cx.theme().danger)
                                        })
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
                                        .child(
                                            div()
                                                .relative()
                                                .when(!word_wrap, |content| {
                                                    content
                                                        .left(-horizontal_offset)
                                                        .w(message_width)
                                                })
                                                .child(selectable),
                                        ),
                                )
                                .child(log_fixed_column_divider_overlay(fixed_columns_width, cx)),
                        ))
                    }
                }
            })
            .collect()
    }

    pub(super) fn render_wrapped_global_table(
        &self,
        surface: Entity<LogRegionSurface>,
        workspace: WeakEntity<Workspace>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let _performance_scope =
            crate::ui_performance::scope("Workspace::render_wrapped_global_table");
        let delegate = self.global_table.read(cx).delegate();
        let count = VirtualLogListDelegate::row_count(delegate);
        let base_height = snap_to_device_pixels(delegate.minimum_row_height(), self.scale_factor);
        if count == 0 {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_2()
                .text_color(cx.theme().muted_foreground)
                .child(crate::tr!("尚未执行全局搜索", "Global search has not run"))
                .into_any_element();
        }
        let fixed_columns_width =
            line_marker_column_width() + px(delegate.line_number_width() as f32);
        let content_width = if self.global_viewport.is_wrapped() {
            px(0.)
        } else {
            delegate.unwrapped_content_width(cx)
        };
        if self.global_viewport.wrapped_base_height() != base_height {
            self.global_viewport
                .ensure_wrapped_measurement_anchor(self.global_table.read(cx).active_log_row());
        }
        self.global_viewport.wrapped_sizes(count, base_height);
        let list_scroll = self.global_viewport.wrapped_scroll_handle();
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
                    .key_context("VirtualLogList")
                    .child(
                        v_virtual_log_list(
                            surface,
                            "wrapped-global-results",
                            list_scroll,
                            count,
                            base_height,
                            content_width,
                            move |_, range, window, cx| {
                                workspace
                                    .update(cx, |workspace, cx| {
                                        workspace.render_wrapped_global_rows(range, window, cx)
                                    })
                                    .unwrap_or_default()
                            },
                        )
                        .size_full()
                        .when(!self.global_viewport.is_wrapped(), |list| {
                            list.pb(Scrollbar::width())
                        }),
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
            .when(!self.global_viewport.is_wrapped(), |container| {
                container.child(
                    div()
                        .absolute()
                        .left(fixed_columns_width)
                        .right(Scrollbar::width())
                        .bottom_0()
                        .h(Scrollbar::width())
                        .bg(scrollbar_background)
                        .child(
                            persistent_log_scrollbar(
                                Scrollbar::horizontal(&logical_scroll)
                                    .id("global-results-horizontal-scrollbar")
                                    .viewport_from_layout(),
                                scrollbar_background,
                            )
                            .max_fps(60),
                        ),
                )
            })
            .into_any_element()
    }

    pub(super) fn render_log_region_surface(
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
            let key = (0, WrappedRegion::GlobalResults);
            let target = if let Some(offset) = self.global_viewport.take_pending_scrollbar_offset()
            {
                self.pending_log_scroll_frames.clear(key);
                Some(LogScrollFrameTarget::Scrollbar(offset))
            } else {
                self.pending_log_scroll_frames.take(key)
            };
            if let Some(target) = target {
                self.prepare_global_scroll_frame(target, row_height, window, cx);
            }
            return self.render_wrapped_global_table(surface, cx.weak_entity(), cx);
        }
        let Some(tab_ix) = self.documents.iter().position(|tab| tab.id == document_id) else {
            return div().into_any_element();
        };
        let key = (document_id, region);
        let pending_offset = if region == WrappedRegion::Results {
            self.documents[tab_ix]
                .result_viewport
                .take_pending_scrollbar_offset()
        } else {
            self.documents[tab_ix]
                .log_viewport
                .take_pending_scrollbar_offset()
        };
        let target = if let Some(offset) = pending_offset {
            self.pending_log_scroll_frames.clear(key);
            Some(LogScrollFrameTarget::Scrollbar(offset))
        } else {
            self.pending_log_scroll_frames.take(key)
        };
        if let Some(target) = target {
            self.prepare_local_scroll_frame(document_id, region, target, row_height, window, cx);
        }
        self.render_wrapped_log_table(document_id, region, surface, cx.weak_entity(), cx)
    }

    pub(super) fn render_new_tab_workspace(&self, cx: &mut Context<Self>) -> impl IntoElement {
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

    pub(super) fn render_document_workspace(
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
        let local_line_number_width = if tab.view.show_line_numbers {
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
                        .track_focus(&self.search_results_viewer.focus_handle)
                        .tab_index(0)
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _: &MouseDownEvent, window, cx| {
                                this.search_results_viewer.focus_handle.focus(window, cx);
                                this.remember_user_log_region(LogRegion::CurrentResults);
                            }),
                        )
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(|this, _: &MouseDownEvent, window, cx| {
                                this.search_results_viewer.focus_handle.focus(window, cx);
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
                                    bounds.size.height,
                                    window,
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
                        .child(self.search_results_viewer.surface.clone())
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
                        .text_selection_scope(self.search_results_viewer.text_selection_scope),
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
                        .track_focus(&self.search_results_viewer.focus_handle)
                        .tab_index(0)
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _: &MouseDownEvent, window, cx| {
                                this.search_results_viewer.focus_handle.focus(window, cx);
                                this.remember_user_log_region(LogRegion::GlobalResults);
                            }),
                        )
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(|this, _: &MouseDownEvent, window, cx| {
                                this.search_results_viewer.focus_handle.focus(window, cx);
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
                                    bounds.size.height,
                                    window,
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
                        .child(self.search_results_viewer.surface.clone())
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
                        .text_selection_scope(self.search_results_viewer.text_selection_scope),
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
                                .track_focus(&self.log_viewer.focus_handle)
                                .tab_index(0)
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, _: &MouseDownEvent, window, cx| {
                                        this.log_viewer.focus_handle.focus(window, cx);
                                        this.remember_user_log_region(LogRegion::Body);
                                    }),
                                )
                                .on_mouse_down(
                                    MouseButton::Right,
                                    cx.listener(|this, _: &MouseDownEvent, window, cx| {
                                        this.log_viewer.focus_handle.focus(window, cx);
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
                                            bounds.size.height,
                                            window,
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
                                .child(self.log_viewer.surface.clone())
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
                                .text_selection_scope(self.log_viewer.text_selection_scope),
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

    pub(super) fn render_status_bar(&self, cx: &App) -> impl IntoElement {
        let _performance_scope = crate::ui_performance::scope("Workspace::render_status_bar");
        let marked_count = self
            .active_document()
            .map_or(0, |tab| tab.file.marked_rows.len());
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
            .unwrap_or_else(|| format!("core {}", crate::build_info::VERSION));

        StatusBar::new()
            .h(px(30.))
            .px(px(12.))
            .gap(px(8.))
            .text_size(px(11.))
            .bg(ui_theme::footer_material(&ui_theme::palette(cx)))
            .right(right)
    }
}
