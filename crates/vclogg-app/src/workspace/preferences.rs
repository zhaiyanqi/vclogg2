use super::*;

impl Workspace {
    fn capture_font_viewport_anchors(&self, cx: &App) -> FontViewportAnchors {
        let row_height = self.log_row_height();
        FontViewportAnchors {
            documents: self
                .documents
                .iter()
                .map(|tab| DocumentFontViewportAnchors {
                    document_id: tab.id,
                    log: Self::capture_local_row_viewport_anchor(
                        tab,
                        WrappedRegion::Log,
                        row_height,
                        cx,
                    ),
                    results: Self::capture_local_row_viewport_anchor(
                        tab,
                        WrappedRegion::Results,
                        row_height,
                        cx,
                    ),
                })
                .collect(),
            global_results: self.capture_global_row_viewport_anchor(row_height, cx),
        }
    }

    fn restore_font_viewport_anchors(&self, anchors: FontViewportAnchors, cx: &mut App) {
        let row_height = self.log_row_height();
        for anchors in anchors.documents {
            let Some(tab) = self
                .documents
                .iter()
                .find(|tab| tab.id == anchors.document_id)
            else {
                continue;
            };
            Self::position_local_row_viewport_anchor(
                tab,
                WrappedRegion::Log,
                anchors.log,
                row_height,
                cx,
            );
            Self::position_local_row_viewport_anchor(
                tab,
                WrappedRegion::Results,
                anchors.results,
                row_height,
                cx,
            );
        }
        self.position_global_row_viewport_anchor(anchors.global_results, row_height, cx);
    }

    pub(super) fn active_file_is_pinned(&self) -> bool {
        self.active_document().is_some_and(|tab| {
            self.pinned_files
                .iter()
                .any(|file| paths_match(&file.path, tab.document.path()))
        })
    }

    pub(super) fn toggle_active_file_pinned(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = self
            .active_document()
            .map(|tab| tab.document.path().to_path_buf())
        else {
            return;
        };
        let Some(store) = self.persistence.store.clone() else {
            return;
        };
        if self.pinned_updating {
            return;
        }
        let pinned = !self.active_file_is_pinned();
        self.pinned_updating = true;
        cx.notify();
        self.persistence
            .state_tasks
            .push(cx.spawn_in(window, async move |this, cx| {
                let result = cx
                    .background_spawn(async move {
                        store.set_pinned(&path, pinned)?;
                        Ok::<_, anyhow::Error>((store.recent_files(8)?, store.pinned_files()?))
                    })
                    .await;
                _ = this.update_in(cx, |this, window, cx| {
                    this.pinned_updating = false;
                    match result {
                        Ok((recent_files, pinned_files)) => {
                            this.recent_files = recent_files;
                            this.pinned_files = pinned_files;
                        }
                        Err(error) => window.push_notification(
                            crate::tr_args!(
                                "收藏状态未能保存：{error}",
                                "Couldn’t save favorite status: {error}"
                            ),
                            cx,
                        ),
                    }
                    cx.notify();
                });
            }));
    }

    pub(super) fn clear_pinned_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(store) = self.persistence.store.clone() else {
            return;
        };
        if self.pinned_updating || self.pinned_files.is_empty() {
            return;
        }
        self.pinned_updating = true;
        cx.notify();
        self.persistence
            .state_tasks
            .push(cx.spawn_in(window, async move |this, cx| {
                let result = cx
                    .background_spawn(async move {
                        store.clear_pinned()?;
                        store.recent_files(8)
                    })
                    .await;
                _ = this.update_in(cx, |this, window, cx| {
                    this.pinned_updating = false;
                    match result {
                        Ok(recent_files) => {
                            this.recent_files = recent_files;
                            this.pinned_files.clear();
                        }
                        Err(error) => window.push_notification(
                            crate::tr_args!(
                                "收藏未能清空：{error}",
                                "Couldn’t clear favorites: {error}"
                            ),
                            cx,
                        ),
                    }
                    cx.notify();
                });
            }));
    }

    pub(super) fn open_history_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(store) = self.persistence.store.clone() else {
            window.push_notification(
                crate::tr!("状态库尚未就绪", "State storage is not ready"),
                cx,
            );
            return;
        };
        if self.history_dialog_loading {
            return;
        }
        let current_workspace_id = cx.entity_id();
        let workspaces = cx
            .global::<WorkspaceWindowRegistry>()
            .windows
            .iter()
            .map(|entry| entry.workspace.clone())
            .collect::<Vec<_>>();
        let mut open_paths = self
            .documents
            .iter()
            .map(|tab| tab.document.path().to_path_buf())
            .collect::<Vec<_>>();
        for workspace in workspaces {
            if workspace.entity_id() == current_workspace_id {
                continue;
            }
            open_paths.extend(
                workspace
                    .read(cx)
                    .documents
                    .iter()
                    .map(|tab| tab.document.path().to_path_buf()),
            );
        }
        self.history_dialog_loading = true;
        cx.notify();
        self.persistence
            .state_tasks
            .push(cx.spawn_in(window, async move |this, cx| {
                let store_for_query = store.clone();
                let result = cx
                    .background_spawn(async move {
                        Ok::<_, anyhow::Error>((
                            store_for_query.session_history()?,
                            store_for_query.database_info()?,
                            result_export::temporary_result_files()?,
                        ))
                    })
                    .await;
                _ = this.update_in(cx, |this, window, cx| {
                    this.history_dialog_loading = false;
                    match result {
                        Ok((sessions, database_info, temporary_results)) => {
                            let history = cx.new(|cx| {
                                HistoryDialog::new(
                                    sessions,
                                    database_info,
                                    temporary_results,
                                    open_paths,
                                    store.clone(),
                                    window,
                                    cx,
                                )
                            });
                            this.history_dialog_subscription = Some(cx.subscribe_in(
                                &history,
                                window,
                                |this, _, event: &HistoryDialogEvent, window, cx| match event {
                                    HistoryDialogEvent::Open(path) => {
                                        // GPUI queues emitted events, so the owner closes the
                                        // dialog only after this subscription receives the event.
                                        window.close_dialog(cx);
                                        this.open_recent_file(path.clone(), window, cx);
                                    }
                                    HistoryDialogEvent::ClearHistory => {
                                        window.close_dialog(cx);
                                        this.confirm_clear_history(window, cx);
                                    }
                                    HistoryDialogEvent::HistoryChanged {
                                        recent_files,
                                        pinned_files,
                                        last_workspace_files,
                                    } => {
                                        this.recent_files = recent_files.clone();
                                        this.pinned_files = pinned_files.clone();
                                        this.last_workspace_files = last_workspace_files.clone();
                                        cx.notify();
                                    }
                                },
                            ));
                            let (history_dialog_size, history_dialog_margin_top) =
                                management_dialog_geometry(window);
                            window.open_dialog(cx, move |dialog, _, _| {
                                let history = history.clone();
                                dialog
                                    .w(history_dialog_size.width)
                                    .h(history_dialog_size.height)
                                    .margin_top(history_dialog_margin_top)
                                    .title(crate::tr!("文件历史", "File history"))
                                    .content(move |content, _, _| {
                                        content.min_h_0().overflow_hidden().child(history.clone())
                                    })
                                    .button_props(
                                        DialogButtonProps::default()
                                            .ok_text(crate::tr!("关闭", "Close")),
                                    )
                            });
                        }
                        Err(error) => window.push_notification(
                            crate::tr_args!(
                                "历史记录未能读取：{error}",
                                "Couldn’t read history: {error}"
                            ),
                            cx,
                        ),
                    }
                    cx.notify();
                });
            }));
    }

    pub(super) fn remember_settings_category(
        &mut self,
        category: SettingsCategory,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !category.is_available() {
            return;
        }
        let changed = cx.update_global::<WorkspaceWindowRegistry, _>(|registry, _| {
            let changed = !registry.last_settings_category_loaded
                || registry.last_settings_category != category;
            registry.last_settings_category = category;
            registry.last_settings_category_loaded = true;
            changed
        });
        if !changed {
            return;
        }
        let Some(store) = self.persistence.store.clone() else {
            return;
        };
        let category = category.storage_value().to_string();
        let previous_save = self.persistence.settings_category_save_task.take();
        self.persistence.settings_category_save_task =
            Some(cx.spawn_in(window, async move |this, cx| {
                if let Some(previous_save) = previous_save {
                    previous_save.await;
                }
                let result = cx
                    .background_spawn(async move { store.save_last_settings_category(&category) })
                    .await;
                if let Err(error) = result {
                    _ = this.update_in(cx, |_, window, cx| {
                        window.push_notification(
                            crate::tr_args!(
                                "设置页位置未能保存：{error}",
                                "Couldn’t save the settings page position: {error}"
                            ),
                            cx,
                        );
                    });
                }
            }));
    }

    pub(super) fn restore_search_panel_height(
        &mut self,
        stored_height: Option<f32>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.search_panel_height_modified {
            if let Some(height) = self.search_panel_height {
                self.remember_search_panel_height(height, window, cx);
            }
            return;
        }
        let Some(height) = stored_height.map(px) else {
            return;
        };
        self.search_panel_height = Some(height);
        if self.search_panel_state.read(cx).sizes().len() < 2 {
            return;
        }
        self.search_panel_state.update(cx, |state, cx| {
            state.resize_panel(1, height, window, cx);
        });
    }

    pub(super) fn remember_search_panel_height(
        &mut self,
        height: Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.search_panel_height = Some(height);
        self.search_panel_height_modified = true;
        let Some(store) = self.persistence.store.clone() else {
            return;
        };
        let previous_save = self.persistence.search_panel_height_save_task.take();
        self.persistence.search_panel_height_save_task =
            Some(cx.spawn_in(window, async move |this, cx| {
                if let Some(previous_save) = previous_save {
                    previous_save.await;
                }
                let result = cx
                    .background_spawn(
                        async move { store.save_search_panel_height(height.as_f32()) },
                    )
                    .await;
                if let Err(error) = result {
                    _ = this.update_in(cx, |_, window, cx| {
                        window.push_notification(
                            crate::tr_args!(
                                "搜索面板高度未能保存：{error}",
                                "Couldn’t save the search panel height: {error}"
                            ),
                            cx,
                        );
                    });
                }
            }));
    }

    pub(super) fn resize_search_panel_from_drag(
        &mut self,
        pointer_y: Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(gesture) = self.search_panel_resize_gesture else {
            return;
        };

        let requested_height = gesture.initial_height + gesture.start_y - pointer_y;
        self.search_panel_state.update(cx, |state, cx| {
            state.resize_panel(1, requested_height, window, cx);
        });
        let height = self.search_panel_state.read(cx).sizes().get(1).copied();
        if self.search_panel_height != height {
            self.search_panel_height = height;
            cx.notify();
        }
    }

    pub(super) fn finish_search_panel_resize(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.search_panel_resize_gesture.take().is_none() {
            return false;
        }

        let height = self
            .search_panel_state
            .read(cx)
            .sizes()
            .get(1)
            .copied()
            .or(self.search_panel_height);
        if let Some(height) = height {
            self.remember_search_panel_height(height, window, cx);
        }
        true
    }

    pub(super) fn render_search_panel_resize_event_layer(
        &self,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let resize_bounds = self.search_panel_resize_bounds.clone();
        let workspace = cx.weak_entity();

        canvas(
            |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
            move |_, event_hitbox, window, _cx| {
                window.on_mouse_event({
                    let resize_bounds = resize_bounds.clone();
                    let workspace = workspace.clone();
                    move |event: &MouseDownEvent, phase, window, cx| {
                        if phase.bubble()
                            || event.button != MouseButton::Left
                            || !event_hitbox.is_hovered(window)
                        {
                            return;
                        }
                        let Some(bounds) = resize_bounds.get() else {
                            return;
                        };
                        if !bounds.contains(&event.position) {
                            return;
                        }

                        let started = workspace
                            .update(cx, |workspace, cx| {
                                let initial_height = workspace
                                    .search_panel_state
                                    .read(cx)
                                    .sizes()
                                    .get(1)
                                    .copied()
                                    .or(workspace.search_panel_height)
                                    .unwrap_or(window.rem_size() * 16.);
                                workspace.search_panel_resize_gesture =
                                    Some(SearchPanelResizeGesture {
                                        start_y: event.position.y,
                                        initial_height,
                                    });
                            })
                            .is_ok();
                        if started {
                            window.capture_pointer(event_hitbox.id);
                            cx.stop_propagation();
                        }
                    }
                });

                window.on_mouse_event({
                    let workspace = workspace.clone();
                    move |event: &MouseMoveEvent, phase, window, cx| {
                        if phase.bubble() {
                            return;
                        }

                        let mut release_pointer = false;
                        let consumed = workspace
                            .update(cx, |workspace, cx| {
                                if workspace.search_panel_resize_gesture.is_none() {
                                    return false;
                                }
                                if event.dragging() {
                                    workspace.resize_search_panel_from_drag(
                                        event.position.y,
                                        window,
                                        cx,
                                    );
                                } else {
                                    release_pointer =
                                        workspace.finish_search_panel_resize(window, cx);
                                }
                                true
                            })
                            .unwrap_or(false);
                        if release_pointer {
                            window.release_pointer();
                        }
                        if consumed {
                            cx.stop_propagation();
                        }
                    }
                });

                window.on_mouse_event({
                    move |event: &MouseUpEvent, phase, window, cx| {
                        if phase.bubble() || event.button != MouseButton::Left {
                            return;
                        }
                        let consumed = workspace
                            .update(cx, |workspace, cx| {
                                workspace.finish_search_panel_resize(window, cx)
                            })
                            .unwrap_or(false);
                        if consumed {
                            cx.stop_propagation();
                        }
                    }
                });
            },
        )
        // This is the second child of a block container. Explicitly anchor it so
        // Taffy does not place the full-size event layer at its post-content static position.
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .into_any_element()
    }

    pub(super) fn open_settings_dialog(
        &mut self,
        requested_category: Option<SettingsCategory>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.settings_saving {
            return;
        }
        let active_category = requested_category
            .filter(|category| category.is_available())
            .unwrap_or_else(|| {
                cx.global::<WorkspaceWindowRegistry>()
                    .last_settings_category
            });
        self.remember_settings_category(active_category, window, cx);
        let original_settings = self.app_settings.clone();
        let original_search_history = self.search_history.clone();
        let settings = cx.new(|cx| {
            SettingsDialog::new(
                self.app_settings.clone(),
                original_search_history.clone(),
                SettingsNetworkSnapshot {
                    settings: self.cloud.settings.clone(),
                    client: self.cloud.client.clone(),
                    connection: self.cloud.connection.clone(),
                    client_error: self.cloud.client_error.clone(),
                },
                active_category,
                window,
                cx,
            )
        });
        self.settings_dialog_subscription = Some(cx.subscribe_in(
            &settings,
            window,
            |this, settings, event: &SettingsDialogEvent, window, cx| match event {
                SettingsDialogEvent::DraftChanged => {
                    let draft = {
                        let settings = settings.read(cx);
                        let Ok(draft) = settings.settings(cx) else {
                            return;
                        };
                        draft
                    };
                    this.preview_app_settings(draft, window, cx);
                }
                SettingsDialogEvent::CategoryChanged(category) => {
                    this.remember_settings_category(*category, window, cx)
                }
                SettingsDialogEvent::CloudSettings(settings) => {
                    this.save_cloud_settings(settings.clone(), window, cx)
                }
                SettingsDialogEvent::CloudConnection(connection) => {
                    this.cloud.connection = connection.clone();
                    cx.notify();
                }
            },
        ));
        let workspace = cx.entity();
        let (settings_dialog_size, settings_dialog_margin_top) = management_dialog_geometry(window);
        window.open_dialog(cx, move |dialog, _, _| {
            let settings = settings.clone();
            let workspace_for_save = workspace.clone();
            let workspace_for_cancel = workspace.clone();
            let workspace_for_close = workspace.clone();
            let original_settings = original_settings.clone();
            let original_search_history = original_search_history.clone();
            dialog
                .w(settings_dialog_size.width)
                .h(settings_dialog_size.height)
                .margin_top(settings_dialog_margin_top)
                .title(crate::tr!("设置", "Settings"))
                .child(settings.clone())
                .footer(
                    h_flex()
                        .w_full()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("settings-dialog-cancel")
                                .label(crate::tr!("取消", "Cancel"))
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(Box::new(Cancel), cx)
                                }),
                        )
                        .child(
                            Button::new("settings-dialog-save")
                                .primary()
                                .label(crate::tr!("保存", "Save"))
                                .on_click(|_, window, cx| {
                                    window
                                        .dispatch_action(Box::new(Confirm { secondary: false }), cx)
                                }),
                        ),
                )
                .on_ok(move |_, window, cx| {
                    let (draft, search_history, network_settings) = {
                        let settings = settings.read(cx);
                        let draft = match settings.settings(cx) {
                            Ok(draft) => draft,
                            Err(error) => {
                                window.push_notification(error, cx);
                                return false;
                            }
                        };
                        (
                            draft,
                            settings.search_history(),
                            settings.network_settings(cx),
                        )
                    };
                    let retained = search_history
                        .iter()
                        .map(String::as_str)
                        .collect::<HashSet<_>>();
                    let removed = original_search_history
                        .iter()
                        .filter(|query| !retained.contains(query.as_str()))
                        .cloned()
                        .collect::<Vec<_>>();
                    workspace_for_save.update(cx, |this, cx| {
                        this.save_app_settings(draft, window, cx);
                        this.save_cloud_settings(network_settings, window, cx);
                        this.remove_search_history_entries(&removed, window, cx);
                    });
                    true
                })
                .on_cancel(move |_, window, cx| {
                    workspace_for_cancel.update(cx, |this, cx| {
                        this.preview_app_settings(original_settings.clone(), window, cx);
                    });
                    true
                })
                .on_close(move |_, _, cx| {
                    workspace_for_close.update(cx, |this, _| {
                        this.settings_dialog_subscription = None;
                    });
                })
        });
    }

    pub(super) fn apply_color_labels(&mut self, labels: Vec<ColorLabel>, cx: &mut Context<Self>) {
        self.cancel_color_rule_action();
        self.cancel_color_labels_resolution();
        self.color_labels = labels;
        if self
            .last_color_label_id
            .as_ref()
            .is_some_and(|id| self.color_labels.iter().all(|label| &label.id != id))
        {
            self.last_color_label_id = None;
        }
        let revision = self.color_labels_resolution_revision;
        let labels = self.color_labels.clone();
        let inputs = self
            .documents
            .iter()
            .map(|tab| ColorRuleResolutionInput {
                document_id: tab.id,
                document: tab.document.clone(),
                rules: tab.file.keyword_color_rules.clone(),
            })
            .collect();
        let cancellation = SearchCancellation::default();
        self.color_labels_resolution_cancellation = Some(cancellation.clone());
        self.color_labels_resolution_task = Some(cx.spawn(async move |this, cx| {
            let prepared = cx
                .background_spawn(async move {
                    prepare_color_rule_resolutions(inputs, &labels, &cancellation)
                        .map(|prepared| (labels, prepared))
                })
                .await;
            _ = this.update(cx, |this, cx| {
                if this.color_labels_resolution_revision != revision {
                    return;
                }
                this.color_labels_resolution_task = None;
                this.color_labels_resolution_cancellation = None;
                let Some((labels, prepared)) = prepared else {
                    return;
                };
                if this.color_labels != labels {
                    return;
                }
                for prepared in prepared {
                    let Some(tab) = this.documents.iter_mut().find(|tab| {
                        tab.id == prepared.document_id
                            && Arc::ptr_eq(&tab.document, &prepared.document)
                            && tab.file.keyword_color_rules == prepared.rules
                    }) else {
                        continue;
                    };
                    tab.file.resolved_color_rules = prepared.resolved.clone();
                    for table in [tab.log_table.clone(), tab.result_table.clone()] {
                        table.update(cx, |table, cx| {
                            table
                                .delegate_mut()
                                .set_color_rules(prepared.resolved.clone());
                            table.refresh(cx);
                        });
                    }
                }
                this.refresh_global_color_rules(cx);
                cx.notify();
            });
        }));
    }

    pub(super) fn cancel_color_labels_resolution(&mut self) {
        self.color_labels_resolution_revision =
            self.color_labels_resolution_revision.saturating_add(1);
        if let Some(cancellation) = self.color_labels_resolution_cancellation.take() {
            cancellation.cancel();
        }
        self.color_labels_resolution_task = None;
    }

    pub(super) fn refresh_global_color_rules(&mut self, cx: &mut Context<Self>) {
        self.global_search.all_open_context.resolved_color_rules = resolve_color_rules(
            &self.global_search.all_open_context.keyword_color_rules,
            &self.color_labels,
        );
        self.global_search.directory_context.resolved_color_rules = resolve_color_rules(
            &self.global_search.directory_context.keyword_color_rules,
            &self.color_labels,
        );
        let session_color_rules = match self.global_search.scope {
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
            SearchScope::CurrentFile => Arc::default(),
        };
        let color_rules_by_path = self
            .documents
            .iter()
            .map(|tab| {
                (
                    path_match_key(tab.document.path()),
                    (tab.document.clone(), tab.file.resolved_color_rules.clone()),
                )
            })
            .collect::<BTreeMap<_, _>>();
        self.global_table.update(cx, |table, cx| {
            table.delegate_mut().update_color_rules(|source| {
                let color_rules = path_match_map_get(&color_rules_by_path, &source.path)
                    .filter(|(document, _)| {
                        result_snapshot_matches_document(&source.path, &source.document, document)
                    })
                    .map(|(_, color_rules)| color_rules.clone())
                    .unwrap_or_default();
                ResolvedColorRules::layered(color_rules, session_color_rules.clone())
            });
            table.refresh(cx);
        });
        self.refresh_active_log_search_presentation(cx);
    }

    pub(super) fn open_color_labels_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.color_labels_saving {
            return;
        }
        let labels = cx.new(|cx| ColorLabelsDialog::new(self.color_labels.clone(), window, cx));
        let workspace = cx.entity();
        let (color_labels_dialog_size, color_labels_dialog_margin_top) =
            management_dialog_geometry(window);
        window.open_dialog(cx, move |dialog, _, _| {
            let labels = labels.clone();
            let workspace = workspace.clone();
            dialog
                .w(color_labels_dialog_size.width)
                .h(color_labels_dialog_size.height)
                .margin_top(color_labels_dialog_margin_top)
                .title(crate::tr!("颜色标签", "Color labels"))
                .child(labels.clone())
                .footer(
                    DialogFooter::new()
                        .child(
                            DialogClose::new().child(
                                Button::new("color-label-dialog-cancel")
                                    .label(crate::tr!("取消", "Cancel")),
                            ),
                        )
                        .child(
                            DialogAction::new().child(
                                Button::new("color-label-dialog-save")
                                    .primary()
                                    .label(crate::tr!("保存", "Save")),
                            ),
                        ),
                )
                .on_ok(move |_, window, cx| {
                    let draft = match labels.read(cx).labels(cx) {
                        Ok(draft) => draft,
                        Err(error) => {
                            window.push_notification(error, cx);
                            return false;
                        }
                    };
                    workspace.update(cx, |this, cx| {
                        this.save_color_labels(draft, window, cx);
                    });
                    true
                })
        });
    }

    pub(super) fn save_color_labels(
        &mut self,
        labels: Vec<ColorLabel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(store) = self.persistence.store.clone() else {
            window.push_notification(
                crate::tr!(
                    "状态库尚未就绪，颜色标签未保存",
                    "State storage is not ready; color labels weren’t saved"
                ),
                cx,
            );
            return;
        };
        self.apply_color_labels(labels.clone(), cx);
        let source_window = window.window_handle();
        let other_workspaces = cx
            .global::<WorkspaceWindowRegistry>()
            .windows
            .iter()
            .filter(|entry| entry.window != source_window)
            .map(|entry| entry.workspace.clone())
            .collect::<Vec<_>>();
        for workspace in other_workspaces {
            let shared_labels = labels.clone();
            workspace.update(cx, |workspace, cx| {
                workspace.apply_color_labels(shared_labels, cx)
            });
        }
        self.color_labels_saving = true;
        cx.notify();
        self.persistence
            .state_tasks
            .push(cx.spawn_in(window, async move |this, cx| {
                let result = cx
                    .background_spawn(async move { store.save_color_labels(&labels) })
                    .await;
                _ = this.update_in(cx, |this, window, cx| {
                    this.color_labels_saving = false;
                    match result {
                        Ok(()) => window.push_notification(
                            crate::tr!("颜色标签已保存", "Color labels saved"),
                            cx,
                        ),
                        Err(error) => window.push_notification(
                            crate::tr_args!(
                                "颜色标签未能保存：{error}",
                                "Couldn’t save color labels: {error}"
                            ),
                            cx,
                        ),
                    }
                    cx.notify();
                });
            }));
    }

    pub(super) fn open_predefined_filters_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.predefined_filters_saving {
            return;
        }
        let filters = cx.new(|cx| {
            PredefinedFiltersDialog::new(
                self.predefined_filters.clone(),
                self.cloud.settings.clone(),
                self.cloud.client.clone(),
                self.cloud.connection.clone(),
                self.cloud.client_error.clone(),
                window,
                cx,
            )
        });
        self.predefined_filters_dialog_subscription = Some(cx.subscribe_in(
            &filters,
            window,
            |this, _, event: &PredefinedFiltersDialogEvent, window, cx| match event {
                PredefinedFiltersDialogEvent::Filters(filters) => {
                    this.save_predefined_filters(filters.clone(), window, cx)
                }
                PredefinedFiltersDialogEvent::CloudSettings(settings) => {
                    this.save_cloud_settings(settings.clone(), window, cx)
                }
                PredefinedFiltersDialogEvent::CloudConnection(connection) => {
                    this.cloud.connection = connection.clone();
                    cx.notify();
                }
            },
        ));
        let workspace = cx.entity();
        let (predefined_filters_dialog_size, predefined_filters_dialog_margin_top) =
            management_dialog_geometry(window);
        window.open_dialog(cx, move |dialog, _, _| {
            let filters = filters.clone();
            let content_filters = filters.clone();
            let workspace = workspace.clone();
            dialog
                .w(predefined_filters_dialog_size.width)
                .h(predefined_filters_dialog_size.height)
                .margin_top(predefined_filters_dialog_margin_top)
                .title(crate::tr!("预定义过滤器", "Predefined filters"))
                .content(move |content, _, _| {
                    content
                        .p_0()
                        .min_h_0()
                        .overflow_hidden()
                        .child(content_filters.clone())
                })
                .on_ok(move |_, window, cx| {
                    if !filters.read(cx).accepts_confirm() {
                        return false;
                    }
                    let draft = match filters.read(cx).filters(cx) {
                        Ok(draft) => draft,
                        Err(error) => {
                            window.push_notification(error, cx);
                            return false;
                        }
                    };
                    workspace.update(cx, |this, cx| {
                        this.save_predefined_filters(draft, window, cx);
                    });
                    true
                })
        });
    }

    pub(super) fn apply_predefined_filters(
        &mut self,
        filters: Vec<PredefinedFilter>,
        cx: &mut Context<Self>,
    ) {
        self.predefined_filters = filters;
        self.refresh_search_autocomplete(cx);
    }

    pub(super) fn save_predefined_filters(
        &mut self,
        filters: Vec<PredefinedFilter>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let revision = PREDEFINED_FILTERS_SAVE_REVISION.fetch_add(1, Ordering::AcqRel) + 1;
        cx.update_global::<WorkspaceWindowRegistry, _>(|registry, _| {
            registry.predefined_filters = Some(filters.clone());
        });
        self.apply_predefined_filters(filters.clone(), cx);
        let source_window = window.window_handle();
        let other_workspaces = cx
            .global::<WorkspaceWindowRegistry>()
            .windows
            .iter()
            .filter(|entry| entry.window != source_window)
            .map(|entry| entry.workspace.clone())
            .collect::<Vec<_>>();
        for workspace in other_workspaces {
            let shared_filters = filters.clone();
            workspace.update(cx, |workspace, cx| {
                workspace.apply_predefined_filters(shared_filters, cx)
            });
        }
        if self.persistence.store.is_none() {
            window.push_notification(
                crate::tr!(
                    "过滤器已应用，但状态库尚未就绪，未持久保存",
                    "The filter was applied but not persisted because state storage is not ready"
                ),
                cx,
            );
            return;
        }
        if self.predefined_filters_saving {
            self.pending_predefined_filters_save = Some((revision, filters));
            cx.notify();
            return;
        }
        self.persist_predefined_filters(revision, filters, window, cx);
    }

    pub(super) fn persist_predefined_filters(
        &mut self,
        revision: u64,
        filters: Vec<PredefinedFilter>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(store) = self.persistence.store.clone() else {
            window.push_notification(
                crate::tr!(
                    "状态库尚未就绪，预定义过滤器未保存",
                    "State storage is not ready; predefined filters weren’t saved"
                ),
                cx,
            );
            return;
        };
        self.predefined_filters_saving = true;
        cx.notify();
        self.persistence
            .state_tasks
            .push(cx.spawn_in(window, async move |this, cx| {
                let result = cx
                    .background_spawn(async move {
                        save_predefined_filters_if_current(&store, &filters, revision)
                    })
                    .await;
                _ = this.update_in(cx, |this, window, cx| {
                    this.predefined_filters_saving = false;
                    let pending = this.pending_predefined_filters_save.take();
                    match result {
                        Ok(true) if pending.is_none() => window.push_notification(
                            crate::tr!("预定义过滤器已保存", "Predefined filters saved"),
                            cx,
                        ),
                        Ok(_) => {}
                        Err(error) => window.push_notification(
                            crate::tr_args!(
                                "预定义过滤器未能保存：{error}",
                                "Couldn’t save predefined filters: {error}"
                            ),
                            cx,
                        ),
                    }
                    if let Some((revision, pending)) = pending {
                        this.persist_predefined_filters(revision, pending, window, cx);
                    }
                    cx.notify();
                });
            }));
    }

    pub(super) fn save_cloud_settings(
        &mut self,
        settings: CloudSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(store) = self.persistence.store.clone() else {
            window.push_notification(
                crate::tr!(
                    "状态库尚未就绪，云端连接设置未保存",
                    "State storage is not ready; cloud connection settings weren’t saved"
                ),
                cx,
            );
            return;
        };
        self.cloud.settings = settings.clone();
        let source_window = window.window_handle();
        let other_workspaces = cx
            .global::<WorkspaceWindowRegistry>()
            .windows
            .iter()
            .filter(|entry| entry.window != source_window)
            .map(|entry| entry.workspace.clone())
            .collect::<Vec<_>>();
        for workspace in other_workspaces {
            let settings = settings.clone();
            workspace.update(cx, |workspace, cx| {
                workspace.cloud.settings = settings;
                cx.notify();
            });
        }
        self.persistence
            .state_tasks
            .push(cx.spawn_in(window, async move |this, cx| {
                let result = cx
                    .background_spawn(async move { store.save_cloud_settings(&settings) })
                    .await;
                if let Err(error) = result {
                    _ = this.update_in(cx, |_, window, cx| {
                        window.push_notification(
                            crate::tr_args!(
                                "云端连接设置未能保存：{error}",
                                "Couldn’t save cloud connection settings: {error}"
                            ),
                            cx,
                        );
                    });
                }
            }));
    }

    pub(super) fn apply_search_defaults(&mut self, case_sensitive: bool, regex: bool) {
        self.app_settings.default_case_sensitive = case_sensitive;
        self.app_settings.default_use_regex = regex;
        if self.documents.is_empty()
            && self.global_search.query.text.is_empty()
            && self.global_search.directory_query.text.is_empty()
        {
            self.case_sensitive = case_sensitive;
            self.regex = regex;
            self.global_search.query.case_sensitive = case_sensitive;
            self.global_search.query.regex = regex;
            self.global_search.directory_query.case_sensitive = case_sensitive;
            self.global_search.directory_query.regex = regex;
        }
        self.search_defaults_modified = true;
    }

    pub(super) fn queue_app_settings_save(
        &mut self,
        settings: AppSettings,
        report_completion: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(store) = self.persistence.store.clone() else {
            return;
        };
        let previous_save = self.persistence.app_settings_save_task.take();
        if report_completion {
            self.settings_saving = true;
        }
        self.persistence.app_settings_save_task =
            Some(cx.spawn_in(window, async move |this, cx| {
                if let Some(previous_save) = previous_save {
                    previous_save.await;
                }
                let result = cx
                    .background_spawn(async move { store.save_app_settings(settings) })
                    .await;
                _ = this.update_in(cx, |this, window, cx| {
                    if report_completion {
                        this.settings_saving = false;
                    }
                    match result {
                        Ok(()) if report_completion => {
                            window.push_notification(crate::tr!("设置已保存", "Settings saved"), cx)
                        }
                        Ok(()) => {}
                        Err(error) if report_completion => window.push_notification(
                            crate::tr_args!(
                                "设置未能保存：{error}",
                                "Couldn’t save settings: {error}"
                            ),
                            cx,
                        ),
                        Err(error) => window.push_notification(
                            crate::tr_args!(
                                "搜索默认值未能保存：{error}",
                                "Couldn’t save search defaults: {error}"
                            ),
                            cx,
                        ),
                    }
                    cx.notify();
                });
            }));
    }

    pub(super) fn refresh_localized_input_copy(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.query.update(cx, |input, cx| {
            input.set_placeholder(crate::tr!("搜索", "Search"), window, cx);
        });
        self.quick_find.query.update(cx, |input, cx| {
            input.set_placeholder(
                crate::tr!("在当前视图中查找", "Find in current view"),
                window,
                cx,
            );
        });
    }

    pub(super) fn preview_app_settings(
        &mut self,
        settings: AppSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_app_settings_inner(settings, false, window, cx);
    }

    pub(super) fn apply_app_settings(
        &mut self,
        settings: AppSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_app_settings_inner(settings, true, window, cx);
    }

    pub(super) fn apply_app_settings_inner(
        &mut self,
        settings: AppSettings,
        commit_defaults: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.app_settings.search_result_limit() != settings.search_result_limit() {
            self.cancel_search();
        }
        let font_viewport_anchors = log_font_layout_changed(&self.app_settings, &settings)
            .then(|| self.capture_font_viewport_anchors(cx));
        crate::actions::apply_shortcuts(&self.app_settings.shortcuts, &settings.shortcuts, cx);
        cx.set_reduce_motion(settings.reduce_motion);
        crate::i18n::set_language(settings.language);
        self.refresh_localized_input_copy(window, cx);
        crate::app_log::set_level(settings.app_log_level);
        Self::apply_theme_preference(settings.theme_preference, window, cx);
        self.app_settings = settings.clone();
        if commit_defaults {
            self.apply_search_defaults(settings.default_case_sensitive, settings.default_use_regex);
            let document_id = self.active_ix.map(|active_ix| {
                let tab = &mut self.documents[active_ix];
                tab.view.show_line_numbers = settings.default_show_line_numbers;
                tab.view.show_row_separators = settings.default_show_row_separators;
                tab.view.uses_default_view_options = true;
                tab.refresh_view_options(cx);
                tab.id
            });
            if let Some(document_id) = document_id {
                self.schedule_checkpoint(document_id, window, cx);
            }
        }
        let search_result_limit = settings.search_result_limit();
        for tab in &mut self.documents {
            tab.search_query.max_results = search_result_limit;
            tab.refresh_appearance(&settings, cx);
            tab.refresh_word_boundary_characters(&settings.word_boundary_characters, cx);
            tab.refresh_log_level_highlighting(settings.highlight_log_levels, cx);
            tab.refresh_search_matcher(settings.highlight_matches, cx);
        }
        self.refresh_active_log_search_presentation(cx);
        self.global_search.query.max_results = search_result_limit;
        self.global_search.directory_query.max_results = search_result_limit;
        let global_matcher = self.global_result_matcher();
        self.global_table.update(cx, |table, cx| {
            table.delegate_mut().set_appearance(&settings);
            table
                .delegate_mut()
                .set_word_boundary_characters(settings.word_boundary_characters.clone());
            table
                .delegate_mut()
                .set_highlight_log_levels(settings.highlight_log_levels);
            table.delegate_mut().set_search_matcher(global_matcher);
            table.refresh(cx);
            cx.notify();
        });
        if let Some(anchors) = font_viewport_anchors {
            self.restore_font_viewport_anchors(anchors, cx);
        }
        let source_window = window.window_handle();
        let other_workspaces = cx
            .global::<WorkspaceWindowRegistry>()
            .windows
            .iter()
            .filter(|entry| entry.window != source_window)
            .map(|entry| entry.workspace.clone())
            .collect::<Vec<_>>();
        for workspace in other_workspaces {
            let shared_settings = settings.clone();
            workspace.update(cx, |workspace, cx| {
                let font_viewport_anchors =
                    log_font_layout_changed(&workspace.app_settings, &shared_settings)
                        .then(|| workspace.capture_font_viewport_anchors(cx));
                if workspace.app_settings.search_result_limit()
                    != shared_settings.search_result_limit()
                {
                    workspace.cancel_search();
                }
                workspace.app_settings = shared_settings.clone();
                workspace.refresh_localized_input_copy(window, cx);
                if commit_defaults {
                    workspace.apply_search_defaults(
                        shared_settings.default_case_sensitive,
                        shared_settings.default_use_regex,
                    );
                }
                let search_result_limit = shared_settings.search_result_limit();
                for tab in &mut workspace.documents {
                    tab.search_query.max_results = search_result_limit;
                    tab.refresh_appearance(&shared_settings, cx);
                    tab.refresh_word_boundary_characters(
                        &shared_settings.word_boundary_characters,
                        cx,
                    );
                    tab.refresh_log_level_highlighting(shared_settings.highlight_log_levels, cx);
                    tab.refresh_search_matcher(shared_settings.highlight_matches, cx);
                }
                workspace.refresh_active_log_search_presentation(cx);
                workspace.global_search.query.max_results = search_result_limit;
                workspace.global_search.directory_query.max_results = search_result_limit;
                let global_matcher = workspace.global_result_matcher();
                workspace.global_table.update(cx, |table, cx| {
                    table.delegate_mut().set_appearance(&shared_settings);
                    table.delegate_mut().set_word_boundary_characters(
                        shared_settings.word_boundary_characters.clone(),
                    );
                    table
                        .delegate_mut()
                        .set_highlight_log_levels(shared_settings.highlight_log_levels);
                    table.delegate_mut().set_search_matcher(global_matcher);
                    table.refresh(cx);
                    cx.notify();
                });
                if let Some(anchors) = font_viewport_anchors {
                    workspace.restore_font_viewport_anchors(anchors, cx);
                }
                cx.notify();
            });
        }
        cx.notify();
    }

    /// 菜单里的开关只改一个字段就落盘。这里刻意不走 `apply_app_settings`，因为那条路会把
    /// 视图默认值重新下发给当前标签，顺带覆盖用户在该标签上单独调过的行号与分隔线。
    pub(super) fn update_app_setting(
        &mut self,
        update: impl FnOnce(&mut AppSettings),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.persistence.store.is_none() {
            window.push_notification(
                crate::tr!(
                    "状态库尚未就绪，设置未保存",
                    "State storage is not ready; settings weren’t saved"
                ),
                cx,
            );
            return;
        }
        let mut settings = self.app_settings.clone();
        update(&mut settings);
        self.apply_app_settings_inner(settings.clone(), false, window, cx);
        self.queue_app_settings_save(settings, false, window, cx);
    }

    pub(super) fn save_app_settings(
        &mut self,
        settings: AppSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.persistence.store.is_none() {
            window.push_notification(
                crate::tr!(
                    "状态库尚未就绪，设置未保存",
                    "State storage is not ready; settings weren’t saved"
                ),
                cx,
            );
            return;
        }
        self.apply_app_settings(settings.clone(), window, cx);
        self.queue_app_settings_save(settings, true, window, cx);
    }

    pub(super) fn open_settings_action(
        &mut self,
        _: &OpenSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_settings_dialog(None, window, cx);
    }

    pub(super) fn adjust_log_font_size_from_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !event.modifiers.secondary() {
            return;
        }
        let delta_y = event.delta.pixel_delta(window.line_height()).y;
        if delta_y == px(0.) {
            return;
        }
        cx.stop_propagation();
        if self.persistence.store.is_none() {
            return;
        }

        let current = self.app_settings.log_font_size;
        let next = if delta_y > px(0.) {
            current.saturating_add(1).min(32)
        } else {
            current.saturating_sub(1).max(8)
        };
        if next == current {
            return;
        }

        let font_viewport_anchors = self.capture_font_viewport_anchors(cx);
        self.app_settings.log_font_size = next;
        for tab in &self.documents {
            tab.refresh_appearance(&self.app_settings, cx);
        }
        self.global_table.update(cx, |table, cx| {
            table.delegate_mut().set_appearance(&self.app_settings);
            table.refresh(cx);
        });
        self.restore_font_viewport_anchors(font_viewport_anchors, cx);

        let source_window = window.window_handle();
        let shared_settings = self.app_settings.clone();
        let other_workspaces = cx
            .global::<WorkspaceWindowRegistry>()
            .windows
            .iter()
            .filter(|entry| entry.window != source_window)
            .map(|entry| entry.workspace.clone())
            .collect::<Vec<_>>();
        for workspace in other_workspaces {
            let shared_settings = shared_settings.clone();
            workspace.update(cx, |workspace, cx| {
                let font_viewport_anchors = workspace.capture_font_viewport_anchors(cx);
                workspace.app_settings = shared_settings.clone();
                for tab in &workspace.documents {
                    tab.refresh_appearance(&shared_settings, cx);
                }
                workspace.global_table.update(cx, |table, cx| {
                    table.delegate_mut().set_appearance(&shared_settings);
                    table.refresh(cx);
                });
                workspace.restore_font_viewport_anchors(font_viewport_anchors, cx);
                cx.notify();
            });
        }
        self.schedule_appearance_save(window, cx);
        cx.notify();
    }

    pub(super) fn capture_log_wheel(
        workspace: Entity<Self>,
        document_id: u64,
        region: WrappedRegion,
    ) -> impl IntoElement {
        canvas(
            |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
            move |_, hitbox, window, _| {
                // The table's scroll mask consumes wheel input during capture, so register this
                // listener first while painting the owning log region.
                window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
                    if !phase.capture() || !hitbox.should_handle_scroll(window) {
                        return;
                    }
                    workspace.update(cx, |workspace, cx| {
                        if event.modifiers.secondary() {
                            workspace.adjust_log_font_size_from_wheel(event, window, cx);
                        } else {
                            workspace.handle_log_region_scroll_wheel(
                                document_id,
                                region,
                                event,
                                window,
                                cx,
                            );
                        }
                    });
                });
            },
        )
        .absolute()
        .size_full()
    }

    pub(super) fn handle_log_region_scroll_wheel(
        &mut self,
        document_id: u64,
        region: WrappedRegion,
        event: &ScrollWheelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.modifiers.control || event.modifiers.platform || event.modifiers.shift {
            return;
        }

        let axis_delta = event.delta.pixel_delta(window.line_height());
        if axis_delta.y == px(0.) || axis_delta.x.abs() > axis_delta.y.abs() {
            return;
        }
        let Some(document_ix) = self.documents.iter().position(|tab| tab.id == document_id) else {
            return;
        };

        let word_wrap = match region {
            WrappedRegion::Log => self.documents[document_ix].log_viewport.is_wrapped(),
            WrappedRegion::Results => self.documents[document_ix].result_viewport.is_wrapped(),
            WrappedRegion::GlobalResults => self.global_viewport.is_wrapped(),
        };
        let line_scroll = self.app_settings.scroll_by_line
            && (!word_wrap || self.app_settings.scroll_by_line_when_word_wrap);
        let scroll_scale = self.app_settings.mouse_wheel_scroll_percent as f32 / 100.;
        let custom_pixel_scale = (scroll_scale - 1.).abs() >= f32::EPSILON;
        let delta_y = if word_wrap && (line_scroll || custom_pixel_scale) {
            event.delta.pixel_delta(px(20.)).y
        } else {
            axis_delta.y
        };
        if delta_y == px(0.) {
            return;
        }

        let auto_follow_changed = region == WrappedRegion::Log
            && std::mem::replace(&mut self.documents[document_ix].view.auto_follow, false);
        let row_height = self.log_row_height();
        let line_count = usize::from(self.app_settings.mouse_wheel_scroll_lines.max(1));
        let row_count = match region {
            WrappedRegion::Log => self.documents[document_ix].document.line_count(),
            WrappedRegion::Results => self.documents[document_ix].result_row_count(cx),
            WrappedRegion::GlobalResults => self.global_table.read(cx).delegate().rows_len(),
        };
        // A wheel event is newer than any scrollbar offset that was recorded but has not yet
        // reached Workspace rendering. Do not let that older drag sample overwrite the wheel
        // target on the next frame.
        match region {
            WrappedRegion::Log => {
                self.documents[document_ix]
                    .log_viewport
                    .take_pending_scrollbar_offset();
            }
            WrappedRegion::Results => {
                self.documents[document_ix]
                    .result_viewport
                    .take_pending_scrollbar_offset();
            }
            WrappedRegion::GlobalResults => {
                self.global_viewport.take_pending_scrollbar_offset();
            }
        }
        let key = if region == WrappedRegion::GlobalResults {
            (0, region)
        } else {
            (document_id, region)
        };
        let latest_target = self.pending_log_scroll_frames.latest(key);
        let wheel_request = LogWheelScrollRequest {
            delta_y,
            row_count,
            row_height,
            line_count,
            line_scroll,
            scale: scroll_scale,
        };
        let target_offset = match region {
            WrappedRegion::Log => {
                let viewport = &self.documents[document_ix].log_viewport;
                let current = latest_target.map_or_else(
                    || viewport.committed_scroll_offset(),
                    |target| viewport.viewport_offset_for_target(target, row_count, row_height),
                );
                viewport.wheel_scroll_target(current, wheel_request)
            }
            WrappedRegion::Results => {
                let viewport = &self.documents[document_ix].result_viewport;
                let current = latest_target.map_or_else(
                    || viewport.committed_scroll_offset(),
                    |target| viewport.viewport_offset_for_target(target, row_count, row_height),
                );
                viewport.wheel_scroll_target(current, wheel_request)
            }
            WrappedRegion::GlobalResults => {
                let viewport = &self.global_viewport;
                let current = latest_target.map_or_else(
                    || viewport.committed_scroll_offset(),
                    |target| viewport.viewport_offset_for_target(target, row_count, row_height),
                );
                viewport.wheel_scroll_target(current, wheel_request)
            }
        };

        cx.stop_propagation();
        if let Some(offset) = target_offset {
            self.pending_log_scroll_frames
                .request(key, LogScrollFrameTarget::Viewport(offset));
            let surface = match region {
                WrappedRegion::Log => self.log_viewer.surface.clone(),
                WrappedRegion::Results => self.search_results_viewer.surface.clone(),
                WrappedRegion::GlobalResults => self.search_results_viewer.surface.clone(),
            };
            Self::refresh_log_surfaces_atomically([surface], window, cx);
        }

        if auto_follow_changed || target_offset.is_some() {
            cx.notify();
        }
    }

    pub(super) fn schedule_appearance_save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(store) = self.persistence.store.clone() else {
            return;
        };
        let settings = self.app_settings.clone();
        self.persistence.appearance_save_task = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(300))
                .await;
            let result = cx
                .background_spawn(async move { store.save_app_settings(settings) })
                .await;
            if let Err(error) = result {
                _ = this.update_in(cx, |_, window, cx| {
                    window.push_notification(
                        crate::tr_args!(
                            "日志字号未能保存：{error}",
                            "Couldn’t save the log font size: {error}"
                        ),
                        cx,
                    );
                });
            }
        }));
    }

    pub(super) fn confirm_clear_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.history_loading || self.history_clearing || self.recent_files.is_empty() {
            return;
        }
        let workspace = cx.entity();
        window.open_alert_dialog(cx, move |alert, _, cx| {
            let workspace = workspace.clone();
            alert
                .icon(Icon::new(IconName::Info).text_color(cx.theme().danger))
                .title(crate::tr!("清除历史？", "Clear history?"))
                .description(crate::tr!("未打开、未收藏且没有行标记的文件会话将被删除。日志文件本身不会改变。", "File sessions that are not open, favorited, or marked will be deleted. Log files will not be changed."))
                .button_props(
                    DialogButtonProps::default()
                        .ok_variant(ButtonVariant::Danger)
                        .ok_text(crate::tr!("清除历史", "Clear history"))
                        .cancel_text(crate::tr!("取消", "Cancel"))
                        .show_cancel(true),
                )
                .on_ok(move |_, window, cx| {
                    workspace.update(cx, |this, cx| this.clear_history(window, cx));
                    true
                })
        });
    }

    pub(super) fn clear_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(store) = self.persistence.store.clone() else {
            return;
        };
        if self.history_clearing {
            return;
        }
        let open_paths = self
            .documents
            .iter()
            .map(|tab| tab.document.path().to_path_buf())
            .collect::<Vec<_>>();
        self.history_clearing = true;
        cx.notify();
        self.persistence
            .state_tasks
            .push(cx.spawn_in(window, async move |this, cx| {
                let result = cx
                    .background_spawn(async move {
                        let removed = store.clear_history(&open_paths)?;
                        Ok::<_, anyhow::Error>((
                            removed,
                            store.recent_files(8)?,
                            store.pinned_files()?,
                            store.last_workspace()?,
                        ))
                    })
                    .await;
                _ = this.update_in(cx, |this, window, cx| {
                    this.history_clearing = false;
                    match result {
                        Ok((removed, recent_files, pinned_files, last_workspace_files)) => {
                            this.recent_files = recent_files;
                            this.pinned_files = pinned_files;
                            this.last_workspace_files = last_workspace_files;
                            window.push_notification(
                                if removed == 0 {
                                    crate::tr!(
                                        "没有可清除的历史记录",
                                        "There is no history to clear"
                                    )
                                    .to_string()
                                } else {
                                    crate::tr_args!(
                                        "已清除 {removed} 条历史记录",
                                        "Cleared {removed} history entries"
                                    )
                                },
                                cx,
                            );
                        }
                        Err(error) => window.push_notification(
                            crate::tr_args!(
                                "历史记录未能清除：{error}",
                                "Couldn’t clear history: {error}"
                            ),
                            cx,
                        ),
                    }
                    cx.notify();
                });
            }));
    }
}
