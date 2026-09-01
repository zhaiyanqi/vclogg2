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
                    if window.is_window_active() {
                        let window_handle = window.window_handle();
                        cx.update_global::<WorkspaceWindowRegistry, _>(|registry, _| {
                            registry.mark_focused(window_handle)
                        });
                        workspace.restore_input_focus(window, cx);
                        workspace.start_file_watch(window, cx);
                        cx.notify();
                    } else {
                        TextSelection::end(window, cx);
                        workspace.end_all_row_drag_selection(window, cx);
                        workspace.release_input_focus(window, cx);
                        workspace.file_watch_task = None;
                    }
                });
            let appearance_subscription =
                cx.observe_window_appearance(window, |workspace, window, cx| {
                    if workspace.persistence.store.is_some()
                        && workspace.app_settings.theme_preference == ThemePreference::System
                    {
                        Self::apply_theme_preference(ThemePreference::System, window, cx);
                    }
                });
            workspace.subscriptions.push(activation_subscription);
            workspace.subscriptions.push(appearance_subscription);
            // 新窗口打开后不会再收到一次激活通知，跟随轮询要在这里起头。
            workspace.start_file_watch(window, cx);
        });
    }

    /// 自动跟随只在窗口激活期间轮询：后台窗口看不到新内容，而定时唤醒会让进程一直占用 CPU。
    fn start_file_watch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.file_watch_task.is_some() {
            return;
        }
        self.file_watch_task = Some(cx.spawn_in(window, async move |this, cx| {
            loop {
                Self::poll_auto_follow(&this, cx).await;
                cx.background_executor().timer(FILE_WATCH_INTERVAL).await;
            }
        }));
    }

    async fn poll_auto_follow(this: &WeakEntity<Self>, cx: &mut AsyncWindowContext) {
        let candidate = this
            .update_in(cx, |this, _, cx| this.auto_follow_candidate(cx))
            .ok()
            .flatten();
        let Some((document_id, path, indexed_size, indexed_modified)) = candidate else {
            return;
        };
        let metadata = cx
            .background_spawn(async move {
                std::fs::metadata(path).map(|metadata| (metadata.len(), metadata.modified().ok()))
            })
            .await;
        let Ok((current_size, current_modified)) = metadata else {
            return;
        };
        if current_size == indexed_size && current_modified == indexed_modified {
            return;
        }

        _ = this.update_in(cx, |this, window, cx| {
            let remains_stale = this.documents.iter().any(|tab| {
                tab.id == document_id
                    && tab.auto_follow
                    && (tab.document.metadata().file_size != current_size
                        || tab.document.metadata().modified != current_modified)
            });
            if remains_stale {
                this.reload_document(document_id, true, ReloadStrategy::ExtendAppend, window, cx);
            }
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
        preference: ThemePreference,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mode = match preference {
            ThemePreference::Light => ThemeMode::Light,
            ThemePreference::Dark => ThemeMode::Dark,
            ThemePreference::System => window.appearance().into(),
        };
        ui_theme::apply_product_theme(mode, cx);

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
            Self::set_cross_window_drop_visual(&target_to_clear, None, cx);
        }
        let Some(workspace) = workspace else {
            return;
        };
        let snapshot = workspace.update(cx, |workspace, cx| workspace.take_quit_snapshot(cx));
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

    fn set_cross_window_drop_visual(
        target: &CrossWindowDropTarget,
        mode: Option<TabTransferMode>,
        cx: &mut App,
    ) {
        target.workspace.update(cx, |workspace, cx| {
            let target_ix = mode.map(|_| target.target_ix);
            let visible = mode.is_some();
            if workspace.cross_window_drop_ix != target_ix
                || workspace.file_drop_visible != visible
                || workspace.file_drop_tab_transfer != mode
            {
                workspace.cross_window_drop_ix = target_ix;
                workspace.file_drop_visible = visible;
                workspace.file_drop_tab_transfer = mode;
                cx.notify();
            }
        });
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
        let source_bounds = window.bounds();
        let source_scale = window.scale_factor();
        let screen_x =
            (source_bounds.origin.x.as_f32() + event.event.position.x.as_f32()) * source_scale;
        let screen_y =
            (source_bounds.origin.y.as_f32() + event.event.position.y.as_f32()) * source_scale;
        let mut candidates = cx
            .global::<WorkspaceWindowRegistry>()
            .windows
            .iter()
            .filter(|entry| entry.window != source_window)
            .cloned()
            .collect::<Vec<_>>();
        candidates.sort_by_key(|entry| std::cmp::Reverse(entry.focus_order));

        let mut next_target = None;
        let mut over_workspace_window = false;
        for candidate in candidates {
            let target_window = candidate.window;
            let target_workspace = candidate.workspace.clone();
            let hit = target_window
                .update(cx, |_, target_window, cx| {
                    let bounds = target_window.bounds();
                    let target_scale = target_window.scale_factor();
                    let left = bounds.origin.x.as_f32() * target_scale;
                    let top = bounds.origin.y.as_f32() * target_scale;
                    let right = (bounds.origin.x + bounds.size.width).as_f32() * target_scale;
                    let bottom = (bounds.origin.y + bounds.size.height).as_f32() * target_scale;
                    if screen_x < left || screen_x >= right || screen_y < top || screen_y >= bottom
                    {
                        return (false, None);
                    }
                    let local_position = Point {
                        x: px((screen_x - left) / target_scale),
                        y: px((screen_y - top) / target_scale),
                    };
                    let workspace = target_workspace.read(cx);
                    (
                        true,
                        workspace
                            .tab_drop_layout
                            .borrow()
                            .drop_index(local_position),
                    )
                })
                .unwrap_or((false, None));
            let (inside_window, target_ix) = hit;
            if inside_window {
                over_workspace_window = true;
            }
            if let Some(target_ix) = target_ix {
                next_target = Some(CrossWindowDropTarget {
                    window: target_window,
                    workspace: candidate.workspace,
                    target_ix,
                });
                break;
            }
            if inside_window {
                break;
            }
        }

        let mode = if window.modifiers().control {
            TabTransferMode::Copy
        } else {
            TabTransferMode::Move
        };
        let (previous_target, changed) =
            cx.update_global::<WorkspaceWindowRegistry, _>(|registry, _| {
                let unchanged = registry.cross_window_tab_drag.as_ref().is_some_and(|drag| {
                    drag.source_window == source_window
                        && drag.document_id == document_id
                        && drag.mode == mode
                        && drag.over_workspace_window == over_workspace_window
                        && match (&drag.target, &next_target) {
                            (Some(left), Some(right)) => {
                                left.window == right.window && left.target_ix == right.target_ix
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
                    mode,
                    target: next_target.clone(),
                    over_workspace_window,
                });
                (previous_target, true)
            });
        if !changed {
            return;
        }
        if let Some(previous_target) = previous_target {
            Self::set_cross_window_drop_visual(&previous_target, None, cx);
        }
        if let Some(next_target) = next_target {
            Self::set_cross_window_drop_visual(&next_target, Some(mode), cx);
        }
    }

    pub(super) fn finish_cross_window_tab_drag(
        event: &MouseUpEvent,
        window: &mut Window,
        cx: &mut App,
    ) {
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
        let Some(source) = drag.source.upgrade() else {
            return;
        };
        if let Some(target) = drag.target {
            Self::set_cross_window_drop_visual(&target, None, cx);
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
        if drag.over_workspace_window || client_bounds.contains(&event.position) {
            return;
        }
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
            let mut session_paths = BTreeSet::new();
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
                    if session_paths.insert(path.clone()) {
                        sessions.push((path, state));
                    }
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
