//! One pointer gesture, independent of the window currently owning its document.
use super::search_tabs::{SearchTabGroup, SearchTabId, SearchTabOwner};
use super::*;

pub(super) struct CrossWindowTabDrag {
    pub(super) source_window: AnyWindowHandle,
    pub(super) document_id: u64,
    pub(super) target: Option<CrossWindowDropTarget>,
    pub(super) floating: Option<Bounds<Pixels>>,
    cursor_offset: Point<Pixels>,
    size: Size<Pixels>,
    window_anchor: Point<Pixels>,
    copied: bool,
}

struct LiveTabTransfer {
    path: PathBuf,
    prepared: PreparedDocument,
    searches: Option<SearchTabGroup>,
    transient: bool,
    selected_rows: CompressedRows,
}

fn update_owner<R>(
    target: &CrossWindowDropTarget,
    window: &mut Window,
    cx: &mut App,
    update: impl FnOnce(&mut Workspace, &mut Window, &mut Context<Workspace>) -> R,
) -> Option<R> {
    if target.window == window.window_handle() {
        Some(
            target
                .workspace
                .update(cx, |this, cx| update(this, window, cx)),
        )
    } else {
        target
            .window
            .update(cx, |_, window, cx| {
                target
                    .workspace
                    .update(cx, |this, cx| update(this, window, cx))
            })
            .ok()
    }
}

impl Workspace {
    pub(super) fn begin_cross_window_tab_drag(
        dragged: &DraggedTab,
        cursor_offset: Point<Pixels>,
        size: Size<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(document_id) = dragged.tab_id.document_id() else {
            return;
        };
        let Some(source) = dragged.source.upgrade() else {
            return;
        };
        let window_anchor = source
            .read(cx)
            .tab_drop_layout
            .borrow()
            .tabs
            .first()
            .map(|tab| tab.origin + cursor_offset)
            .unwrap_or(cursor_offset);
        cx.global_mut::<WorkspaceWindowRegistry>()
            .cross_window_tab_drag = Some(CrossWindowTabDrag {
            source_window: window.window_handle(),
            document_id,
            target: Some(CrossWindowDropTarget {
                window: window.window_handle(),
                workspace: source,
                target_ix: 0,
                position: window.mouse_position(),
            }),
            floating: None,
            cursor_offset,
            size,
            window_anchor,
            copied: false,
        });
    }

    /// Topmost workspace wins; a floating window must not hide its docking target.
    fn tab_drag_target(
        screen: Point<Pixels>,
        floating: Option<AnyWindowHandle>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<CrossWindowDropTarget> {
        let mut candidates = cx.global::<WorkspaceWindowRegistry>().windows.clone();
        candidates.sort_by_key(|entry| std::cmp::Reverse(entry.focus_order));
        for candidate in candidates {
            if Some(candidate.window) == floating {
                continue;
            }
            let inspect = |window: &mut Window, cx: &mut App| {
                let position = screen - crate::window_drag::screen_origin(window, cx);
                if !Bounds::new(Point::default(), window.bounds().size).contains(&position) {
                    return None;
                }
                let layout = candidate.workspace.read(cx).tab_drop_layout.borrow();
                let ix = layout.drop_index(position).or_else(|| {
                    layout
                        .viewport
                        .filter(|bounds| bounds.contains(&position))
                        .map(|_| {
                            layout
                                .tabs
                                .iter()
                                .take_while(|tab| tab.center().x < position.x)
                                .count()
                        })
                });
                Some((ix, position))
            };
            let hit = if candidate.window == window.window_handle() {
                inspect(window, cx)
            } else {
                candidate
                    .window
                    .update(cx, |_, window, cx| inspect(window, cx))
                    .ok()
                    .flatten()
            };
            if let Some((ix, position)) = hit {
                // Do not dock through another window's document area.
                return ix.map(|target_ix| CrossWindowDropTarget {
                    window: candidate.window,
                    workspace: candidate.workspace,
                    target_ix,
                    position,
                });
            }
        }
        None
    }

    pub(super) fn track_cross_window_tab_drag(
        _: &DraggedTab,
        event: &DragMoveEvent<DraggedTab>,
        window: &mut Window,
        cx: &mut App,
    ) {
        if event.event.pressed_button != Some(MouseButton::Left) {
            return;
        }
        Self::move_cross_window_tab_drag(
            event.event.position,
            event.event.modifiers.control,
            window,
            cx,
        );
    }

    fn move_cross_window_tab_drag(
        position: Point<Pixels>,
        copy: bool,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(mut drag) = cx
            .global_mut::<WorkspaceWindowRegistry>()
            .cross_window_tab_drag
            .take()
        else {
            return;
        };
        let Some(owner) = drag.target.clone() else {
            return;
        };
        if !owner
            .workspace
            .read(cx)
            .documents
            .iter()
            .any(|tab| tab.id == drag.document_id)
        {
            Self::clear_cross_window_drag(drag, cx);
            cx.stop_active_drag(window);
            return;
        }
        let screen = crate::window_drag::screen_origin(window, cx) + position;
        let next = Self::tab_drag_target(screen, drag.floating.map(|_| owner.window), window, cx);
        let should_copy = copy && !drag.copied;
        if let Some(next) = next.filter(|next| next.window != owner.window) {
            if Self::transfer_dragged_document(&mut drag, next, should_copy, window, cx) {
                // A detached shell belongs to this gesture; remove it after docking.
                if drag.floating.take().is_some() {
                    Self::close_empty_drag_window(&owner, cx);
                }
            }
        } else if let Some(mut bounds) = drag.floating {
            bounds.origin = screen - drag.window_anchor;
            let moved = update_owner(&owner, window, cx, |_, window, cx| {
                crate::window_drag::move_to(window, bounds.origin, cx)
            });
            match moved {
                Some(Ok(())) => drag.floating = Some(bounds),
                Some(Err(error)) => {
                    window.notify_message(
                        crate::tr_args!(
                            "无法继续移动窗口：{error}",
                            "Couldn’t continue moving the window: {error}"
                        ),
                        cx,
                    );
                    Self::clear_cross_window_drag(drag, cx);
                    cx.stop_active_drag(window);
                    return;
                }
                None => {
                    Self::clear_cross_window_drag(drag, cx);
                    cx.stop_active_drag(window);
                    return;
                }
            }
        } else {
            let inside = update_owner(&owner, window, cx, |_, window, cx| {
                Bounds::new(
                    crate::window_drag::screen_origin(window, cx),
                    window.bounds().size,
                )
                .contains(&screen)
            })
            .unwrap_or(false);
            if !inside {
                Self::detach_dragged_document(&mut drag, screen, copy, window, cx);
            }
        }
        if let Some(target) = &mut drag.target {
            let floating = drag.floating;
            let position = update_owner(target, window, cx, |this, window, cx| {
                let origin = floating.map_or_else(
                    || crate::window_drag::screen_origin(window, cx),
                    |bounds| bounds.origin,
                );
                let position = screen - origin;
                if floating.is_none() {
                    this.move_file_tab_drag(position.x, window, cx);
                }
                cx.notify();
                position
            });
            if let Some(position) = position {
                target.position = position;
            } else {
                Self::clear_cross_window_drag(drag, cx);
                cx.stop_active_drag(window);
                return;
            }
        }
        cx.global_mut::<WorkspaceWindowRegistry>()
            .cross_window_tab_drag = Some(drag);
    }

    fn prepare_live_tab_transfer(
        &mut self,
        id: u64,
        cx: &mut Context<Self>,
    ) -> Option<LiveTabTransfer> {
        self.capture_active_search_tab(cx);
        let tab = self.documents.iter().find(|tab| tab.id == id)?;
        // In-flight editing/loading still belongs to its original window's callbacks.
        if tab.load_state != DocumentLoadState::Ready
            || tab.edit.is_some()
            || tab.edit_load_task.is_some()
            || self.pending_tab_moves.contains(&id)
        {
            return None;
        }
        let path = tab.document.path().to_path_buf();
        let mut searches = self
            .search_tabs
            .groups
            .get(&SearchTabOwner::File(id))
            .map(|group| SearchTabGroup {
                active: group.active,
                next_id: group.next_id,
                tabs: group.tabs.clone(),
            });
        if let Some(group) = &mut searches {
            let owner = SearchTabOwner::File(id);
            for state in &mut group.tabs {
                let tab_id = SearchTabId(state.saved.id);
                state.needs_restore |= self.search_tabs.running.as_ref().is_some_and(
                    |(running_owner, running_id, _, _)| {
                        *running_owner == owner && *running_id == tab_id
                    },
                ) || self
                    .search_tabs
                    .queue
                    .iter()
                    .any(|job| job.owner == owner && job.id == tab_id);
            }
        }
        Some(LiveTabTransfer {
            prepared: PreparedDocument {
                document: tab.document.clone(),
                cached_complete_document: None,
                session: Some(self.file_session_state(tab, cx)),
                color_labels_snapshot: Some(self.color_labels.clone()),
                resolved_color_rules: tab.file.resolved_color_rules.clone(),
                search_result: tab.search_result.clone(),
                search_range: self.search_ranges.get(&path),
                search_matcher: tab.search_matcher.clone(),
                search_case_sensitive: tab.search_query.case_sensitive,
                search_regex: tab.search_query.regex,
                warning: None,
                load_state: tab.load_state,
                pending_index_cache: None,
                upgrade_frame: None,
            },
            transient: path_match_set_contains(&self.transient_paths, &path),
            selected_rows: tab
                .log_table
                .read(cx)
                .delegate()
                .selected_source_rows_compressed(),
            path,
            searches,
        })
    }

    fn transfer_dragged_document(
        drag: &mut CrossWindowTabDrag,
        target: CrossWindowDropTarget,
        copy: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        let Some(owner) = drag.target.clone() else {
            return false;
        };
        let Some(snapshot) = update_owner(&owner, window, cx, |this, _, cx| {
            this.prepare_live_tab_transfer(drag.document_id, cx)
        })
        .flatten() else {
            return false;
        };
        let can_receive = target.workspace.read(cx).open_task.is_none()
            && !target
                .workspace
                .read(cx)
                .documents
                .iter()
                .any(|tab| paths_match(tab.document.path(), &snapshot.path));
        if !can_receive {
            return false;
        }
        let installed = update_owner(&target, window, cx, |this, window, cx| {
            let LiveTabTransfer {
                path,
                prepared,
                searches,
                transient,
                selected_rows,
            } = snapshot;
            if transient {
                this.transient_paths.insert(path_match_key(&path));
            }
            this.cancel_pending_tab_activation();
            this.install_documents(
                vec![(path.clone(), Ok(prepared))],
                Some(&path),
                &BTreeMap::from([(path.clone(), target.target_ix)]),
                None,
                true,
                window,
                cx,
            );
            let id = this
                .documents
                .iter()
                .find(|tab| paths_match(tab.document.path(), &path))
                .unwrap()
                .id;
            if let Some(searches) = searches {
                this.search_tabs
                    .groups
                    .insert(SearchTabOwner::File(id), searches);
                this.search_tabs.installed = None;
                this.sync_search_tab(window, cx);
            }
            let tab = this.documents.iter().find(|tab| tab.id == id).unwrap();
            let active = this.file_session_state(tab, cx).selected_row;
            tab.log_table.update(cx, |table, cx| {
                table
                    .delegate_mut()
                    .restore_search_tab_selection(selected_rows, active);
                cx.notify();
            });
            let source = cx.weak_entity();
            let preview = cx.new(|_| DraggedTab {
                tab_id: WorkspaceTabId::Document(id),
                position: drag.cursor_offset,
                size: drag.size,
                source,
            });
            this.begin_tab_drag(
                super::tab_drag::TabDragKey::File(WorkspaceTabId::Document(id)),
                preview.into(),
                drag.cursor_offset,
                cx,
            );
            this.persist_workspace_order(window, cx);
            window.activate_window();
            id
        });
        let Some(id) = installed else {
            return false;
        };
        update_owner(&owner, window, cx, |this, window, cx| {
            this.clear_tab_drag(cx);
            if !copy {
                this.close_tab_by_id(drag.document_id, window, cx);
                this.tab_motion
                    .closing
                    .retain(|tab| tab.id != WorkspaceTabId::Document(drag.document_id));
            }
        });
        drag.document_id = id;
        drag.target = Some(target);
        drag.copied |= copy;
        true
    }

    fn detach_dragged_document(
        drag: &mut CrossWindowTabDrag,
        screen: Point<Pixels>,
        copy: bool,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(owner) = drag.target.clone() else {
            return;
        };
        // Check before creating a shell so a loading/edited tab never spawns empty windows.
        if update_owner(&owner, window, cx, |this, _, cx| {
            this.prepare_live_tab_transfer(drag.document_id, cx)
        })
        .flatten()
        .is_none()
        {
            return;
        }
        let (mut bounds, display) = Self::detached_window_placement(screen, window, cx);
        bounds.origin = screen - drag.window_anchor;
        let initial_bounds = crate::window_drag::initial_bounds(bounds, display, cx);
        let Ok(handle) =
            crate::open_workspace_window_at(cx, false, Vec::new(), initial_bounds, display)
        else {
            return;
        };
        let workspace = cx
            .global::<WorkspaceWindowRegistry>()
            .windows
            .iter()
            .find(|entry| entry.window == handle)
            .unwrap()
            .workspace
            .clone();
        workspace.update(cx, |this, _| this.tabs.clear());
        let target = CrossWindowDropTarget {
            window: handle,
            workspace,
            target_ix: 0,
            position: drag.window_anchor,
        };
        if Self::transfer_dragged_document(drag, target.clone(), copy && !drag.copied, window, cx) {
            // The whole window represents the drag here; its tab is visible normally.
            update_owner(&target, window, cx, |this, _, cx| this.clear_tab_drag(cx));
            drag.floating = Some(bounds);
        } else {
            Self::close_empty_drag_window(&target, cx);
        }
    }

    fn close_empty_drag_window(target: &CrossWindowDropTarget, cx: &mut App) {
        if target.workspace.read(cx).documents.is_empty()
            && target.workspace.read(cx).new_file_drafts.is_empty()
        {
            // Defer removal: this may be the window dispatching the current input event.
            let handle = target.window;
            cx.defer(move |cx| {
                _ = handle.update(cx, |_, window, _| window.remove_window());
            });
        }
    }

    pub(super) fn clear_cross_window_drag(drag: CrossWindowTabDrag, cx: &mut App) {
        if let Some(target) = drag.target {
            // May be requested from an update of the original owner; avoid reentrancy.
            cx.defer(move |cx| {
                _ = drag.source_window.update(cx, |_, window, cx| {
                    cx.stop_active_drag(window);
                });
                _ = target.window.update(cx, |_, window, cx| {
                    cx.stop_active_drag(window);
                    target
                        .workspace
                        .update(cx, |this, cx| this.clear_tab_drag(cx));
                });
            });
        }
    }

    pub(super) fn cancel_cross_window_tab_drag(_: gpui_kit::EntityId, cx: &mut App) {
        if let Some(drag) = cx
            .global_mut::<WorkspaceWindowRegistry>()
            .cross_window_tab_drag
            .take()
        {
            Self::clear_cross_window_drag(drag, cx);
        }
    }

    pub(super) fn finish_cross_window_tab_drag(
        event: &MouseUpEvent,
        window: &mut Window,
        cx: &mut App,
    ) {
        if event.button != MouseButton::Left {
            return;
        }
        // Resolve a final boundary crossing even if the OS coalesced the last move.
        Self::move_cross_window_tab_drag(event.position, event.modifiers.control, window, cx);
        let Some(drag) = cx
            .global_mut::<WorkspaceWindowRegistry>()
            .cross_window_tab_drag
            .take()
        else {
            return;
        };
        cx.stop_active_drag(window);
        if let Some(target) = &drag.target {
            update_owner(target, window, cx, |this, window, cx| {
                if drag.floating.is_none() {
                    this.finish_tab_drag(target.position, window, cx);
                } else {
                    this.clear_tab_drag(cx);
                }
            });
        }
    }
}
