use super::*;

impl Workspace {
    pub(crate) fn init_window_registry(cx: &mut App) {
        cx.set_global(WorkspaceWindowRegistry::default());
    }

    pub(crate) fn open_external_paths_in_last_active_window(
        paths: &[PathBuf],
        cx: &mut App,
    ) -> bool {
        if paths.is_empty() {
            return false;
        }
        let candidates = cx
            .global::<WorkspaceWindowRegistry>()
            .windows_by_recent_focus();
        for candidate in candidates {
            let workspace = candidate.workspace.clone();
            let paths = paths.to_vec();
            if candidate
                .window
                .update(cx, move |_, window, cx| {
                    workspace.update(cx, |workspace, cx| {
                        workspace.enqueue_external_paths(paths, window, cx)
                    });
                    cx.activate(true);
                    window.activate_window();
                })
                .is_ok()
            {
                return true;
            }
        }
        false
    }

    pub(crate) fn register_window(workspace: &Entity<Self>, window: &mut Window, cx: &mut App) {
        let window_handle = window.window_handle();
        let registered_workspace = workspace.clone();
        cx.update_global::<WorkspaceWindowRegistry, _>(|registry, _| {
            registry.register(window_handle, registered_workspace)
        });
        workspace.update(cx, |workspace, cx| {
            let activation_subscription =
                cx.observe_window_activation(window, |workspace, window, cx| {
                    workspace.file_watch_window_active = window.is_window_active();
                    workspace.sync_file_watch(window.window_handle(), cx);
                    if window.is_window_active() {
                        let window_handle = window.window_handle();
                        cx.update_global::<WorkspaceWindowRegistry, _>(|registry, _| {
                            registry.mark_focused(window_handle)
                        });
                        workspace.restore_input_focus(window, cx);
                        cx.notify();
                    } else {
                        workspace.cancel_tag_drag(window, cx);
                        TextSelection::end(window, cx);
                        workspace.end_all_row_drag_selection(window, cx);
                        workspace.release_input_focus(window, cx);
                        cx.notify();
                    }
                });
            let appearance_subscription =
                cx.observe_window_appearance(window, |workspace, window, cx| {
                    if workspace.persistence.store.is_some()
                        && workspace.app_settings.theme_preference == ThemePreference::System
                    {
                        let settings = workspace.app_settings.clone();
                        Self::apply_theme_preference(&settings, window, cx);
                    }
                });
            workspace._subscriptions.push(activation_subscription);
            workspace._subscriptions.push(appearance_subscription);
            // Initialize from the actual window state, including initially inactive windows.
            workspace.start_file_watch(window, cx);
        });
    }

    /// Install lifecycle observation once. Each foreground document owns its monitoring task.
    fn start_file_watch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.file_watch_window_active = window.is_window_active();
        let window_handle = window.window_handle();
        let subscription = cx.observe_self(move |this, cx| this.sync_file_watch(window_handle, cx));
        self._subscriptions.push(subscription);
        self.sync_file_watch(window_handle, cx);
    }

    pub(super) fn sync_file_watch(
        &mut self,
        window_handle: AnyWindowHandle,
        cx: &mut Context<Self>,
    ) {
        let target = self
            .active_document()
            .filter(|tab| {
                self.file_watch_window_active && tab.load_state == DocumentLoadState::Ready
            })
            .map(|tab| (tab.id, tab.document.path().to_path_buf()));
        let document_id = target.as_ref().map(|(id, _)| *id);
        if self.file_watch_document_id != document_id {
            // Dropping the scope cancels its timers and pending publication. The native
            // worker releases its watches in the background; no inactive scope keeps polling.
            self.file_watch_task = None;
            self.file_watch = None;
            self.file_refresh_task = None;
            self.file_watch_document_id = document_id;
            if let Some(document_id) = document_id {
                self.file_watch = match crate::file_watch::FileWatch::new() {
                    Ok(watcher) => Some(watcher),
                    Err(error) => {
                        log::warn!("Could not start file watcher; using slow checks: {error}");
                        None
                    }
                };
                let receiver = self.file_watch.as_ref().map(|watcher| watcher.receiver());
                self.file_watch_task = Some(cx.spawn(async move |this, cx| {
                    // Activation and tab switches check once without waiting for an OS event.
                    Self::refresh_watched_file(&this, document_id, window_handle, true, cx).await;
                    loop {
                        if let Some(receiver) = receiver.as_ref() {
                            if receiver.recv().await.is_err() {
                                break;
                            }
                            // A busy writer cannot postpone publication indefinitely.
                            cx.background_executor()
                                .timer(crate::file_watch::REFRESH_INTERVAL)
                                .await;
                            _ = receiver.try_recv();
                        } else {
                            cx.background_executor()
                                .timer(crate::file_watch::RECHECK_INTERVAL)
                                .await;
                        }
                        Self::refresh_watched_file(&this, document_id, window_handle, false, cx)
                            .await;
                    }
                }));
            }
        }
        if let Some(watcher) = self.file_watch.as_ref() {
            watcher.sync(
                target.into_iter().collect(),
                self.open_task.is_some()
                    || self.file_refresh_task.is_some()
                    || self.row_tag_interaction_active()
                    || self.search_tabs.busy(),
            );
        }
    }

    async fn refresh_watched_file(
        this: &WeakEntity<Self>,
        document_id: u64,
        window_handle: AnyWindowHandle,
        initial_check: bool,
        cx: &mut gpui::AsyncApp,
    ) {
        let document = window_handle
            .update(cx, |_, window, cx| {
                this.update(cx, |this, _| {
                    if !window.is_window_active()
                        || this.file_watch_document_id != Some(document_id)
                        || this.active_tab_id != WorkspaceTabId::Document(document_id)
                        || this.open_task.is_some()
                        || this.file_refresh_task.is_some()
                        || this.row_tag_interaction_active()
                        || this.search_tabs.busy()
                    {
                        return None;
                    }
                    if let Some(watcher) = this.file_watch.as_ref() {
                        let dirty = watcher.take_dirty();
                        if !initial_check && !dirty.contains(&document_id) {
                            return None;
                        }
                    }
                    this.active_document()
                        .filter(|tab| tab.load_state == DocumentLoadState::Ready)
                        .map(|tab| tab.document.clone())
                })
                .ok()
                .flatten()
            })
            .ok()
            .flatten();
        let Some(document) = document else {
            return;
        };
        let source = document.clone();
        let changed = cx
            .background_spawn(async move { source.source_changed() })
            .await;
        _ = window_handle.update(cx, |_, window, cx| {
            this.update(cx, |this, cx| {
                if !window.is_window_active()
                    || this.file_watch_document_id != Some(document_id)
                    || this.active_tab_id != WorkspaceTabId::Document(document_id)
                    || this
                        .active_document()
                        .is_none_or(|tab| !Arc::ptr_eq(&tab.document, &document))
                {
                    return;
                }
                match changed {
                    Ok(false) => {}
                    Err(_) => {
                        if let Some(watcher) = this.file_watch.as_ref() {
                            watcher.retry([document_id]);
                        }
                    }
                    Ok(true) => {
                        let started = !this.search_tabs.busy()
                            && this.reload_document(
                                document_id,
                                ReloadStrategy::ExtendAppend,
                                window,
                                cx,
                            );
                        this.sync_file_watch(window_handle, cx);
                        if let Some(watcher) = this.file_watch.as_ref() {
                            if started {
                                watcher.retry([document_id]);
                            } else {
                                watcher.restore_dirty([document_id]);
                            }
                        }
                    }
                }
            })
        });
    }

    /// 窗口失活时把焦点从输入框收回：`gpui-component` 的光标闪烁定时器只在输入框失焦时停下，
    /// 否则后台窗口每 500ms 都会因为光标翻转而重绘一次。
    fn release_input_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let inputs = [
            self.query.focus_handle(cx),
            self.quick_find.query.focus_handle(cx),
        ];
        let Some(focused) = inputs.into_iter().find(|handle| handle.is_focused(window)) else {
            return;
        };
        self.deactivated_input_focus = Some(focused);
        self.focus_handle.focus(window, cx);
    }

    fn restore_input_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(focused) = self.deactivated_input_focus.take() else {
            return;
        };
        focused.focus(window, cx);
    }

    pub(super) fn apply_theme_preference(
        settings: &AppSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mode = match settings.theme_preference {
            ThemePreference::Light => ThemeMode::Light,
            ThemePreference::Dark => ThemeMode::Dark,
            ThemePreference::System => window.appearance().into(),
        };
        ui_theme::apply_product_theme(mode, cx);
        ui_theme::apply_log_background(settings.log_background_color(mode.is_dark()), mode, cx);

        cx.refresh_windows();
    }

    pub(crate) fn unregister_window(window_id: WindowId, cx: &mut App) {
        let (workspace, target_to_clear) =
            cx.update_global::<WorkspaceWindowRegistry, _>(|registry, _| {
                let mut target_to_clear = None;
                if registry
                    .cross_window_tab_drag
                    .as_ref()
                    .is_some_and(|drag| drag.source_window.window_id() == window_id)
                {
                    target_to_clear = registry
                        .cross_window_tab_drag
                        .take()
                        .and_then(|drag| drag.target);
                } else if let Some(drag) = &mut registry.cross_window_tab_drag
                    && drag
                        .target
                        .as_ref()
                        .is_some_and(|target| target.window.window_id() == window_id)
                {
                    drag.target = None;
                }
                (registry.unregister(window_id), target_to_clear)
            });
        if let Some(target_to_clear) = target_to_clear {
            Self::set_cross_window_drop_visual(&target_to_clear, false, cx);
        }
        let Some(workspace) = workspace else {
            return;
        };
        let snapshot = workspace.update(cx, |workspace, cx| {
            workspace.file_watch_window_active = false;
            workspace.file_watch_document_id = None;
            workspace.file_watch_task = None;
            workspace.file_watch = None;
            workspace.file_refresh_task = None;
            workspace.take_quit_snapshot(cx)
        });
        let background_executor = cx.background_executor().clone();
        let task = cx.spawn(async move |_| {
            for task in snapshot.state_tasks {
                task.await;
            }
            if let Some(task) = snapshot.workspace_order_task {
                task.await;
            }
            let result = background_executor
                .spawn(async move {
                    let store = match snapshot.store {
                        Some(store) => store,
                        None => Arc::new(StateStore::open_default()?),
                    };
                    if let Some(predefined_filters) = snapshot.predefined_filters {
                        save_predefined_filters_if_current(
                            &store,
                            &predefined_filters,
                            snapshot.predefined_filters_revision,
                        )?;
                    }
                    store.save_sessions(&snapshot.sessions)?;
                    if let Some(search_state) = snapshot.search_state {
                        store.save_workspace_search_state(&search_state)?;
                    }
                    Ok::<_, anyhow::Error>(())
                })
                .await;
            if let Err(error) = result {
                log::error!("关闭窗口时文件会话未能保存：{error}");
            }
        });
        cx.update_global::<WorkspaceWindowRegistry, _>(|registry, _| {
            registry.closed_flush_tasks.push(task)
        });
    }

    fn set_cross_window_drop_visual(target: &CrossWindowDropTarget, visible: bool, cx: &mut App) {
        target.workspace.update(cx, |workspace, cx| {
            workspace.cross_window_drop_ix = visible.then_some(target.target_ix);
            // Pointer movement also invalidates GPUI's drag preview in this window,
            // even when the insertion index has not changed.
            cx.notify();
        });
    }

    fn cross_window_tab_drop_target(
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<CrossWindowDropTarget> {
        let source_window = window.window_handle();
        let source_bounds = window.bounds();
        let source_scale = window.scale_factor();
        let screen_x = (source_bounds.origin.x.as_f32() + position.x.as_f32()) * source_scale;
        let screen_y = (source_bounds.origin.y.as_f32() + position.y.as_f32()) * source_scale;
        let mut candidates = cx.global::<WorkspaceWindowRegistry>().windows.clone();
        candidates.sort_by_key(|entry| std::cmp::Reverse(entry.focus_order));

        for candidate in candidates {
            // Respect the source window when it covers another workspace window.
            if candidate.window == source_window {
                if Bounds::new(Point::default(), source_bounds.size).contains(&position) {
                    return None;
                }
                continue;
            }
            let hit = candidate.window.update(cx, |_, target_window, cx| {
                let bounds = target_window.bounds();
                let target_scale = target_window.scale_factor();
                let left = bounds.origin.x.as_f32() * target_scale;
                let top = bounds.origin.y.as_f32() * target_scale;
                let right = (bounds.origin.x + bounds.size.width).as_f32() * target_scale;
                let bottom = (bounds.origin.y + bounds.size.height).as_f32() * target_scale;
                if screen_x < left || screen_x >= right || screen_y < top || screen_y >= bottom {
                    return None;
                }
                let position = Point {
                    x: px((screen_x - left) / target_scale),
                    y: px((screen_y - top) / target_scale),
                };
                let workspace = candidate.workspace.read(cx);
                let target_ix = workspace
                    .tab_drop_layout
                    .borrow()
                    .drop_index(position)
                    .unwrap_or(workspace.tabs.len());
                Some((target_ix, position))
            });
            if let Ok(Some((target_ix, position))) = hit {
                return Some(CrossWindowDropTarget {
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
        dragged: &DraggedTab,
        event: &DragMoveEvent<DraggedTab>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(document_id) = dragged.tab_id.document_id() else {
            return;
        };
        let source_window = window.window_handle();
        let next_target = Self::cross_window_tab_drop_target(event.event.position, window, cx);
        let (previous_target, changed) =
            cx.update_global::<WorkspaceWindowRegistry, _>(|registry, _| {
                let unchanged = registry.cross_window_tab_drag.as_ref().is_some_and(|drag| {
                    drag.source_window == source_window
                        && drag.document_id == document_id
                        && match (&drag.target, &next_target) {
                            (Some(left), Some(right)) => {
                                left.window == right.window
                                    && left.target_ix == right.target_ix
                                    && left.position == right.position
                            }
                            (None, None) => true,
                            _ => false,
                        }
                });
                if unchanged {
                    return (None, false);
                }
                let previous_target = registry
                    .cross_window_tab_drag
                    .take()
                    .and_then(|drag| drag.target);
                registry.cross_window_tab_drag = Some(CrossWindowTabDrag {
                    source_window,
                    source: dragged.source.clone(),
                    document_id,
                    target: next_target.clone(),
                });
                (previous_target, true)
            });
        if !changed {
            return;
        }
        if let Some(previous_target) = previous_target
            && next_target
                .as_ref()
                .is_none_or(|target| target.window != previous_target.window)
        {
            Self::set_cross_window_drop_visual(&previous_target, false, cx);
        }
        if let Some(next_target) = next_target {
            Self::set_cross_window_drop_visual(&next_target, true, cx);
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
        let source_window = window.window_handle();
        let drag = cx.update_global::<WorkspaceWindowRegistry, _>(|registry, _| {
            registry
                .cross_window_tab_drag
                .as_ref()
                .is_some_and(|drag| drag.source_window == source_window)
                .then(|| registry.cross_window_tab_drag.take())
                .flatten()
        });
        let Some(drag) = drag else {
            return;
        };
        let mode = if window.modifiers().control {
            TabTransferMode::Copy
        } else {
            TabTransferMode::Move
        };
        if let Some(target) = &drag.target {
            Self::set_cross_window_drop_visual(target, false, cx);
        }
        let Some(source) = drag.source.upgrade() else {
            return;
        };
        // Resolve the release itself: the last move event may precede a boundary crossing.
        if let Some(target) = Self::cross_window_tab_drop_target(event.position, window, cx) {
            cx.stop_active_drag(window);
            source.update(cx, |source, cx| {
                source.transfer_tab_to_window_target(
                    drag.document_id,
                    mode,
                    TabTransferTarget {
                        window: target.window,
                        workspace: target.workspace,
                        target_ix: Some(target.target_ix),
                    },
                    window,
                    cx,
                );
            });
            return;
        }

        let client_bounds = Bounds::new(Point::default(), window.bounds().size);
        if client_bounds.contains(&event.position) {
            return;
        }
        cx.stop_active_drag(window);
        let screen_position = window.bounds().origin + event.position;
        let (bounds, display_id) = Self::detached_window_placement(screen_position, window, cx);
        source.update(cx, |source, cx| {
            source.transfer_tab_to_new_window(
                drag.document_id,
                mode,
                Some((bounds, display_id)),
                window,
                cx,
            );
        });
    }

    fn detached_window_placement(
        screen_position: Point<Pixels>,
        window: &Window,
        cx: &App,
    ) -> (Bounds<Pixels>, Option<DisplayId>) {
        let display = cx
            .displays()
            .into_iter()
            .find(|display| display.bounds().contains(&screen_position))
            .or_else(|| window.display(cx));
        let Some(display) = display else {
            return (
                Bounds::new(
                    screen_position - point(px(180.), px(24.)),
                    size(px(1280.), px(800.)),
                ),
                None,
            );
        };
        let visible = display.visible_bounds();
        let window_size = size(px(1280.), px(800.)).min(&visible.size);
        let minimum_origin = visible.origin;
        let maximum_origin = point(
            visible.origin.x + visible.size.width - window_size.width,
            visible.origin.y + visible.size.height - window_size.height,
        );
        let preferred_origin = screen_position - point(px(180.), px(24.));
        let origin = point(
            preferred_origin.x.clamp(minimum_origin.x, maximum_origin.x),
            preferred_origin.y.clamp(minimum_origin.y, maximum_origin.y),
        );
        (Bounds::new(origin, window_size), Some(display.id()))
    }

    pub(crate) fn flush_all_on_quit(cx: &mut App) -> impl Future<Output = ()> + use<> {
        let (registered, closed_flush_tasks) =
            cx.update_global::<WorkspaceWindowRegistry, _>(|registry, _| {
                registry.cross_window_tab_drag = None;
                (
                    registry
                        .windows
                        .iter()
                        .map(|entry| entry.workspace.clone())
                        .collect::<Vec<_>>(),
                    std::mem::take(&mut registry.closed_flush_tasks),
                )
            });
        let snapshots = registered
            .into_iter()
            .map(|workspace| workspace.update(cx, |workspace, cx| workspace.take_quit_snapshot(cx)))
            .collect::<Vec<_>>();
        let background_executor = cx.background_executor().clone();

        async move {
            for task in closed_flush_tasks {
                task.await;
            }
            let mut store = None;
            let mut sessions = Vec::new();
            let mut session_paths = BTreeMap::new();
            let mut open_paths = Vec::new();
            let mut open_path_set = BTreeSet::new();
            let mut active_path = None;
            let mut search_state = None;
            let mut predefined_filters = None;

            for mut snapshot in snapshots {
                for task in snapshot.state_tasks {
                    task.await;
                }
                if let Some(task) = snapshot.workspace_order_task {
                    task.await;
                }
                store = store.or(snapshot.store.take());
                for (path, state) in snapshot.sessions {
                    // Keep the established first-window policy for ordinary session
                    // fields, but apply every window's independent tag edits.
                    let first = session_paths
                        .entry(path.clone())
                        .or_insert_with(|| state.clone());
                    let mut combined = first.clone();
                    combined.row_tags = state.row_tags;
                    combined.row_tags_base = state.row_tags_base;
                    sessions.push((path, combined));
                }
                for path in snapshot.open_paths {
                    if open_path_set.insert(path.clone()) {
                        open_paths.push(path);
                    }
                }
                if active_path.is_none()
                    && snapshot
                        .active_path
                        .as_ref()
                        .is_some_and(|path| open_path_set.contains(path))
                {
                    active_path = snapshot.active_path;
                }
                search_state = search_state.or(snapshot.search_state);
                if let Some(filters) = snapshot.predefined_filters {
                    predefined_filters = Some((snapshot.predefined_filters_revision, filters));
                }
            }

            let result = background_executor
                .spawn(async move {
                    let store = match store {
                        Some(store) => store,
                        None => Arc::new(StateStore::open_default()?),
                    };
                    store.save_workspace(&sessions, &open_paths, active_path.as_deref())?;
                    if let Some((revision, predefined_filters)) = predefined_filters {
                        save_predefined_filters_if_current(&store, &predefined_filters, revision)?;
                    }
                    if let Some(search_state) = search_state {
                        store.save_workspace_search_state(&search_state)?;
                    }
                    Ok::<_, anyhow::Error>(())
                })
                .await;
            if let Err(error) = result {
                log::error!("退出状态未能保存：{error}");
            }
        }
    }
}
