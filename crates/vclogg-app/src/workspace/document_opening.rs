use super::*;

impl Workspace {
    pub(crate) fn open_initial_documents_after_activation(
        &mut self,
        documents: Vec<InitialDocument>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.on_next_frame(window, move |_, window, cx| {
            cx.on_next_frame(window, move |this, window, cx| {
                this.pending_initial_documents.extend(documents);
                this.show_initial_document_shells(window, cx);
                this.open_queued_external_paths_if_idle(window, cx);
            });
        });
    }

    pub(super) fn open_files_in_directory(
        &mut self,
        directory: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.open_task.is_some() {
            return;
        }
        self.file_refresh_task.take();

        // Menu handlers run before dismissal, and rfd initializes the native
        // panel synchronously even for its async API. Frame callbacks run before
        // drawing: cross two boundaries so the dismissed menu is presented first.
        let (ready, dismissed) = async_channel::bounded(1);
        window.on_next_frame(move |window, _| {
            window.on_next_frame(move |_, _| {
                _ = ready.try_send(());
            });
        });
        // Reserve the shared loader slot now, including the frame handoff, so
        // another menu/shortcut/external-open cannot create a second picker.
        self.open_task = Some(cx.spawn_in(window, async move |this, cx| {
            if dismissed.recv().await.is_err() {
                return;
            }
            // GPUI's PathPromptOptions has no initial-directory option.
            let Ok(prompt) = this.update_in(cx, |_, window, _| {
                rfd::AsyncFileDialog::new()
                    .set_parent(window)
                    .set_directory(directory)
                    .set_title(crate::tr!("选择日志文件", "Select log files"))
                    .pick_files()
            }) else {
                return;
            };
            let files = prompt.await;
            _ = this.update_in(cx, |this, window, cx| {
                this.open_task = None;
                if let Some(files) = files {
                    this.begin_open_paths(
                        files
                            .into_iter()
                            .map(|file| file.path().to_path_buf())
                            .collect(),
                        window,
                        cx,
                    );
                }
                this.open_queued_external_paths_if_idle(window, cx);
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(super) fn open_files(
        &mut self,
        _: &OpenFiles,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The picker and document loader share this slot; replacing it cancels the load.
        if self.open_task.is_some() {
            return;
        }
        // Explicit file actions take priority over automatic refresh publication.
        self.file_refresh_task.take();

        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some(crate::tr!("选择日志文件", "Select log files").into()),
        });

        self.open_task = Some(cx.spawn_in(window, async move |this, cx| {
            let paths = prompt.await.ok().and_then(Result::ok).flatten();
            _ = this.update_in(cx, |this, window, cx| {
                this.open_task = None;
                if let Some(paths) = paths {
                    this.begin_open_paths(paths, window, cx);
                }
                this.open_queued_external_paths_if_idle(window, cx);
            });
        }));
    }

    pub(super) fn open_recent_file(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.begin_open_paths(vec![path], window, cx);
    }

    pub(super) fn begin_file_drop(&mut self, paths: ExternalPaths, cx: &mut Context<Self>) {
        self.file_drop_paths = Some(paths.clone());
        self.file_drop_has_directory = false;
        self.file_drop_recursive_bounds.set(None);
        cx.spawn(async move |this, cx| {
            let candidates = paths.paths().to_vec();
            let has_directory = cx
                .background_spawn(async move { candidates.iter().any(|path| path.is_dir()) })
                .await;
            _ = this.update(cx, |this, cx| {
                if this
                    .file_drop_paths
                    .as_ref()
                    .is_some_and(|current| current.paths() == paths.paths())
                {
                    this.file_drop_has_directory = has_directory;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn open_dropped_paths(
        &mut self,
        paths: &ExternalPaths,
        recursive: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.open_task.is_some() {
            window.notify_message(
                crate::tr!(
                    "当前正在打开其他文件，请稍后再拖入",
                    "Another file is being opened. Drop files again shortly.",
                ),
                cx,
            );
            return;
        }
        let paths = paths.paths().to_vec();
        // A modal may own unsaved settings. Check it outside the workspace
        // update; closing it programmatically would skip its cancel callback.
        let workspace = cx.weak_entity();
        window.defer(cx, move |window, cx| {
            if window.has_active_dialog(cx) || window.has_active_sheet(cx) {
                window.notify_message(
                    crate::tr!(
                        "请先关闭当前对话框，再拖入文件",
                        "Close the current dialog before dropping files",
                    ),
                    cx,
                );
                return;
            }
            _ = workspace.update(cx, |workspace, cx| {
                workspace.begin_open_dropped_paths(paths, recursive, window, cx);
            });
        });
    }

    fn begin_open_dropped_paths(
        &mut self,
        paths: Vec<PathBuf>,
        recursive: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Recheck after the deferred modal guard before reserving the loader slot.
        if self.open_task.is_some() {
            window.notify_message(
                crate::tr!(
                    "当前正在打开其他文件，请稍后再拖入",
                    "Another file is being opened. Drop files again shortly.",
                ),
                cx,
            );
            return;
        }
        self.file_refresh_task.take();
        self.activity = Activity::Opening;
        self.open_task = Some(cx.spawn_in(window, async move |this, cx| {
            let (files, errors) = cx
                .background_spawn(async move { expand_dropped_paths(paths, recursive) })
                .await;
            _ = this.update_in(cx, |this, window, cx| {
                this.open_task = None;
                this.activity = Activity::Ready;
                if !errors.is_empty() {
                    let details = errors.join("\n");
                    window.notify_message(
                        crate::tr_args!(
                            "部分拖入路径无法读取：{details}",
                            "Some dropped paths could not be read: {details}"
                        ),
                        cx,
                    );
                } else if files.is_empty() {
                    window.notify_message(
                        if recursive {
                            crate::tr!("拖入的文件夹及子文件夹中没有可打开的文件", "The dropped folders and subfolders contain no files to open")
                        } else {
                            crate::tr!("拖入的文件夹中没有可打开的文件（不包含子文件夹）", "The dropped folders contain no files to open (subfolders are excluded)")
                        },
                        cx,
                    );
                }
                this.begin_open_paths(files, window, cx);
                this.open_queued_external_paths_if_idle(window, cx);
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(super) fn restore_last_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = self
            .last_workspace_files
            .iter()
            .map(|file| file.path.clone())
            .collect::<Vec<_>>();
        let active_path = self
            .last_workspace_files
            .iter()
            .find(|file| file.was_active)
            .map(|file| file.path.clone());
        self.begin_open_paths_with_active(paths, active_path, window, cx);
    }

    pub(super) fn begin_open_paths(
        &mut self,
        paths: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.begin_open_paths_with_active(paths, None, window, cx);
    }

    pub(super) fn enqueue_external_paths(
        &mut self,
        paths: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        log::info!(target: crate::open_log::TARGET, "external_enqueue window={:?} paths={paths:?} open_busy={} history_loading={}", window.window_handle().window_id(), self.open_task.is_some(), self.history_loading);
        let paths = deduplicate_paths(
            paths
                .into_iter()
                .filter(|path| !path.as_os_str().is_empty()),
        );
        let new_paths = paths
            .iter()
            .filter(|path| {
                !self
                    .documents
                    .iter()
                    .any(|tab| paths_match(tab.document.path(), path))
            })
            .cloned()
            .collect::<Vec<_>>();
        // Publish placeholders even while another file or the state store is loading.
        self.show_opening_paths(&new_paths, window, cx);
        if let Some(ix) = paths.last().and_then(|path| {
            self.documents
                .iter()
                .position(|tab| paths_match(tab.document.path(), path))
        }) {
            self.activate_tab(ix, window, cx);
        }
        for path in &paths {
            if !new_paths.contains(path) {
                log::info!(target: crate::open_log::TARGET, "external_existing_tab path={path:?} ready={}", self.documents.iter().any(|tab| paths_match(tab.document.path(), path) && tab.load_state == DocumentLoadState::Ready));
            }
        }
        self.pending_external_paths.extend(new_paths);
        self.open_queued_external_paths_if_idle(window, cx);
        // Activation clears the installation's reveal request. Reapply it after
        // dispatch so external opens reveal both new and already-open file tabs.
        if !paths.is_empty() {
            self.pending_document_tab_reveal
                .set(self.active_tab_id.document_id());
            cx.notify();
        }
    }

    pub(super) fn open_queued_external_paths_if_idle(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.history_loading || self.open_task.is_some() {
            return;
        }
        if !self.pending_initial_documents.is_empty() {
            let initial_documents = std::mem::take(&mut self.pending_initial_documents);
            self.resume_initial_documents(initial_documents, window, cx);
            return;
        }
        if self.pending_external_paths.is_empty() {
            return;
        }
        let selected_tab = self.active_tab_id;
        let paths = std::mem::take(&mut self.pending_external_paths);
        let active_path = self
            .active_document()
            .map(|tab| tab.document.path().to_path_buf());
        self.begin_open_paths_with_sessions(
            paths,
            active_path,
            OpenDocumentOverrides::default(),
            false,
            window,
            cx,
        );
        if self.tabs.contains(&selected_tab) && self.active_tab_id != selected_tab {
            self.activate_workspace_tab(selected_tab, window, cx);
        }
    }

    pub(super) fn show_initial_document_shells(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pending_initial_documents.is_empty() {
            return;
        }
        let mut shells = Vec::new();
        let mut target_indices = BTreeMap::new();
        for initial in &self.pending_initial_documents {
            if initial.transient {
                self.transient_paths.insert(path_match_key(&initial.path));
            }
            if let Some(range) = initial.search_range {
                self.search_ranges.set(&initial.path, range);
            }
            if let Some(target_ix) = initial.target_ix {
                path_buf_map_insert(&mut target_indices, initial.path.clone(), target_ix);
            }
            shells.push((
                initial.path.clone(),
                Ok(prepare_document_shell(
                    &initial.path,
                    initial.session.clone(),
                    self.app_settings.default_case_sensitive,
                    self.app_settings.default_use_regex,
                )),
            ));
        }
        let replacement = match self.active_tab_id {
            WorkspaceTabId::New(id)
                if self
                    .pending_initial_documents
                    .iter()
                    .all(|initial| initial.replace_new_tab) =>
            {
                Some(id)
            }
            _ => None,
        };
        let active_path = self
            .pending_initial_documents
            .last()
            .map(|initial| initial.path.clone());
        self.install_documents(
            shells,
            active_path.as_deref(),
            &target_indices,
            replacement,
            false,
            window,
            cx,
        );
    }

    pub(super) fn show_opening_paths(
        &mut self,
        paths: &[PathBuf],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if paths.is_empty() {
            return;
        }
        let replacement = match self.active_tab_id {
            WorkspaceTabId::New(id) if !self.new_file_drafts.contains_key(&id) => Some(id),
            _ => None,
        };
        let shells = paths
            .iter()
            .map(|path| {
                (
                    path.clone(),
                    Ok(prepare_document_shell(
                        path,
                        None,
                        self.app_settings.default_case_sensitive,
                        self.app_settings.default_use_regex,
                    )),
                )
            })
            .collect();
        self.install_documents(
            shells,
            paths.last().map(PathBuf::as_path),
            &BTreeMap::new(),
            replacement,
            false,
            window,
            cx,
        );
    }

    fn resume_initial_documents(
        &mut self,
        initial_documents: Vec<InitialDocument>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Closing a startup placeholder must not reopen it after state restoration.
        let initial_documents = initial_documents
            .into_iter()
            .filter_map(|mut initial| {
                if self
                    .documents
                    .iter()
                    .any(|tab| paths_match(tab.document.path(), &initial.path))
                {
                    // The placeholder already consumed any replaceable new tab.
                    initial.replace_new_tab = false;
                    Some(initial)
                } else {
                    if let Some(completion) = initial.move_completion.take() {
                        completion.finish(false, cx);
                    }
                    None
                }
            })
            .collect();
        let selected_tab = self.active_tab_id;
        self.begin_open_initial_documents(initial_documents, window, cx);
        if self.tabs.contains(&selected_tab) && self.active_tab_id != selected_tab {
            self.activate_workspace_tab(selected_tab, window, cx);
        }
        self.open_queued_external_paths_if_idle(window, cx);
    }

    pub(super) fn begin_open_initial_documents(
        &mut self,
        initial_documents: Vec<InitialDocument>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if initial_documents.is_empty() {
            return;
        }
        let mut overrides = OpenDocumentOverrides::default();
        let mut paths = Vec::with_capacity(initial_documents.len());
        let replace_new_tab = initial_documents
            .iter()
            .all(|initial| initial.replace_new_tab);
        for initial in initial_documents {
            if let Some(range) = initial.search_range {
                self.search_ranges.set(&initial.path, range);
            }
            if initial.transient {
                self.transient_paths.insert(path_match_key(&initial.path));
            }
            if let Some(completion) = initial.move_completion {
                overrides
                    .move_completions
                    .insert(initial.path.clone(), completion);
            }
            if let Some(target_ix) = initial.target_ix {
                path_buf_map_insert(
                    &mut overrides.target_indices,
                    initial.path.clone(),
                    target_ix,
                );
            }
            if let Some(session) = initial.session {
                path_buf_map_insert(&mut overrides.sessions, initial.path.clone(), session);
            }
            paths.push(initial.path);
        }
        let active_path = paths.last().cloned();
        self.begin_open_paths_with_sessions(
            paths,
            active_path,
            overrides,
            replace_new_tab,
            window,
            cx,
        );
    }

    pub(super) fn begin_open_paths_with_active(
        &mut self,
        paths: Vec<PathBuf>,
        active_path: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.begin_open_paths_with_sessions(
            paths,
            active_path,
            OpenDocumentOverrides::default(),
            true,
            window,
            cx,
        );
    }

    pub(super) fn begin_open_paths_with_sessions(
        &mut self,
        paths: Vec<PathBuf>,
        active_path: Option<PathBuf>,
        mut overrides: OpenDocumentOverrides,
        replace_new_tab: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let stage = crate::open_log::Stage::new(
            "Document.open_batch",
            format_args!(
                "window={:?} paths={paths:?}",
                window.window_handle().window_id()
            ),
        );
        let paths = deduplicate_paths(
            paths
                .into_iter()
                .filter(|path| !path.as_os_str().is_empty()),
        );
        if paths.is_empty() || self.open_task.is_some() {
            stage.event("skipped_empty_or_busy");
            return;
        }
        self.file_refresh_task.take();
        for path in &paths {
            if path_buf_map_get(&overrides.sessions, path).is_none()
                && let Some(session) =
                    path_buf_map_get(&self.persistence.pending_session_overrides, path)
            {
                path_buf_map_insert(&mut overrides.sessions, path.clone(), session.clone());
            }
        }
        self.activity = Activity::Opening;
        let replacement_new_tab_id =
            replace_new_tab
                .then_some(self.active_tab_id)
                .and_then(|tab_id| match tab_id {
                    WorkspaceTabId::New(id) if !self.new_file_drafts.contains_key(&id) => Some(id),
                    WorkspaceTabId::New(_) => None,
                    WorkspaceTabId::Document(_) => None,
                });
        let shells = paths
            .iter()
            .map(|path| {
                (
                    path.clone(),
                    Ok(prepare_document_shell(
                        path,
                        path_buf_map_get(&overrides.sessions, path).cloned(),
                        self.app_settings.default_case_sensitive,
                        self.app_settings.default_use_regex,
                    )),
                )
            })
            .collect::<Vec<_>>();
        self.install_documents(
            shells,
            active_path.as_deref(),
            &overrides.target_indices,
            replacement_new_tab_id,
            false,
            window,
            cx,
        );
        let paths = paths
            .into_iter()
            .filter(|path| {
                self.documents
                    .iter()
                    .find(|tab| paths_match(tab.document.path(), path))
                    .is_some_and(|tab| tab.load_state != DocumentLoadState::Ready)
            })
            .collect::<Vec<_>>();
        if paths.is_empty() {
            stage.event("already_ready");
            self.activity = Activity::Ready;
            self.maybe_restore_persisted_search(window, cx);
            for (path, completion) in overrides.move_completions {
                let installed = self.documents.iter().any(|tab| {
                    paths_match(tab.document.path(), &path)
                        && tab.load_state == DocumentLoadState::Ready
                });
                completion.finish(installed, cx);
            }
            self.open_queued_external_paths_if_idle(window, cx);
            cx.notify();
            return;
        }
        let opening_ids = paths
            .iter()
            .filter_map(|path| {
                self.documents
                    .iter()
                    .find(|tab| {
                        paths_match(tab.document.path(), path)
                            && matches!(
                                tab.load_state,
                                DocumentLoadState::Opening
                                    | DocumentLoadState::Preview
                                    | DocumentLoadState::IndexFailed
                            )
                    })
                    .map(|tab| (path.clone(), tab.id))
            })
            .collect::<BTreeMap<_, _>>();
        let state_store = self.persistence.store.clone();
        let OpenDocumentOverrides {
            sessions,
            move_completions,
            target_indices,
        } = overrides;
        let case_sensitive = self.app_settings.default_case_sensitive;
        let regex = self.app_settings.default_use_regex;
        let search_result_limit = self.app_settings.search_result_limit();
        let search_options = SearchPreparationOptions {
            case_sensitive,
            regex,
            max_results: search_result_limit,
        };
        let color_labels = self.color_labels.clone();
        let search_ranges = self.search_ranges.clone();

        // Frame callbacks run before drawing. Let the opening shell reach the
        // screen before starting session reads, previews, or indexing.
        let (ready, presented) = async_channel::bounded(1);
        window.on_next_frame(move |window, _| {
            window.on_next_frame(move |_, _| {
                _ = ready.try_send(());
            });
        });
        self.open_task = Some(cx.spawn_in(window, async move |this, cx| {
            if presented.recv().await.is_err() {
                return;
            }
            stage.event("shell_frame_callbacks_complete session_load_begin");
            let restore_paths = paths.clone();
            let restore_store = state_store.clone();
            let (sessions, fallback_store, effective_search_result_limit) = cx
                .background_spawn(async move {
                    let effective_search_result_limit = restore_store
                        .as_deref()
                        .and_then(|store| store.load_app_settings().ok())
                        .map(|settings| settings.search_result_limit())
                        .unwrap_or(search_result_limit);
                    let (sessions, fallback_store) = match restore_store {
                        Some(store) => match store.load_sessions(&restore_paths) {
                            Ok(mut restored) => {
                                for (path, session) in sessions {
                                    path_buf_map_insert(&mut restored, path, session);
                                }
                                (restored, None)
                            }
                            Err(_) => (sessions, Some(store)),
                        },
                        None => (sessions, None),
                    };
                    (sessions, fallback_store, effective_search_result_limit)
                })
                .await;

            stage.event("session_load_end preview_begin");
            let preview_paths = paths.clone();
            let preview_sessions = sessions.clone();
            let preview_store = fallback_store.clone();
            let preview_color_labels = color_labels.clone();
            let mut previews = cx
                .background_spawn(async move {
                    prepare_paths_bounded(preview_paths, |path| {
                        prepare_document_preview(
                            path,
                            preview_store.as_deref(),
                            path_buf_map_get(&preview_sessions, path).cloned(),
                            SearchPreparationOptions {
                                max_results: effective_search_result_limit,
                                ..search_options
                            },
                            &preview_color_labels,
                        )
                    })
                })
                .await;
            for (path, result) in &previews {
                match result {
                    Ok(_) => stage.event(format_args!("preview_prepared path={path:?}")),
                    Err(error) => {
                        stage.event(format_args!("preview_failed path={path:?} error={error:#}"))
                    }
                }
            }
            stage.event("preview_prepare_end upgrade_begin");
            let cached_complete_documents = previews
                .iter()
                .filter_map(|(path, prepared)| {
                    prepared
                        .as_ref()
                        .ok()?
                        .cached_complete_document
                        .clone()
                        .map(|document| (path.clone(), document))
                })
                .collect::<BTreeMap<_, _>>();

            let preview_upgrade_jobs = this
                .update_in(cx, |this, window, cx| {
                    this.prepare_document_upgrade_jobs(&previews, window, cx)
                })
                .unwrap_or_default();
            if !preview_upgrade_jobs.is_empty() {
                let frames = cx
                    .background_spawn(
                        async move { load_document_upgrade_frames(preview_upgrade_jobs) },
                    )
                    .await;
                Workspace::attach_document_upgrade_frames(&mut previews, frames);
            }

            _ = this.update_in(cx, |this, window, cx| {
                let previews = previews
                    .into_iter()
                    .filter(|(path, _)| {
                        path_buf_map_get(&opening_ids, path).is_some_and(|expected_id| {
                            this.documents.iter().any(|tab| {
                                tab.id == *expected_id
                                    && paths_match(tab.document.path(), path)
                                    && matches!(
                                        tab.load_state,
                                        DocumentLoadState::Opening | DocumentLoadState::IndexFailed
                                    )
                            })
                        })
                    })
                    .collect();
                let preserve_selection = this.pending_directory_group_activation.is_none()
                    && this.pending_search_result_jump.is_none();
                let selected_path = if preserve_selection {
                    this.active_document()
                        .map(|tab| tab.document.path().to_path_buf())
                } else {
                    active_path.clone()
                };
                let selected_tab = this.active_tab_id;
                this.install_documents(
                    previews,
                    selected_path.as_deref(),
                    &target_indices,
                    None,
                    false,
                    window,
                    cx,
                );
                if preserve_selection
                    && this.tabs.contains(&selected_tab)
                    && this.active_tab_id != selected_tab
                {
                    this.activate_workspace_tab(selected_tab, window, cx);
                }
            });

            stage.event("preview_install_returned full_prepare_begin");
            let full_paths = paths.clone();
            let full_store = fallback_store;
            let mut opened = cx
                .background_spawn(async move {
                    prepare_paths_bounded(full_paths, |path| {
                        prepare_document_in_range(
                            path,
                            path_buf_map_get(&cached_complete_documents, path).cloned(),
                            full_store.as_deref(),
                            path_buf_map_get(&sessions, path).cloned(),
                            SearchPreparationOptions {
                                max_results: effective_search_result_limit,
                                ..search_options
                            },
                            &color_labels,
                            search_ranges.get(path),
                        )
                    })
                })
                .await;

            for (path, result) in &opened {
                match result {
                    Ok(_) => stage.event(format_args!("full_prepared path={path:?}")),
                    Err(error) => {
                        stage.event(format_args!("full_failed path={path:?} error={error:#}"))
                    }
                }
            }
            stage.event("full_prepare_end upgrade_begin");
            let upgrade_jobs = this
                .update_in(cx, |this, window, cx| {
                    this.prepare_document_upgrade_jobs(&opened, window, cx)
                })
                .unwrap_or_default();
            if !upgrade_jobs.is_empty() {
                let frames = cx
                    .background_spawn(async move { load_document_upgrade_frames(upgrade_jobs) })
                    .await;
                Workspace::attach_document_upgrade_frames(&mut opened, frames);
            }

            _ = this.update_in(cx, |this, window, cx| {
                let preserve_selection = this.pending_directory_group_activation.is_none()
                    && this.pending_search_result_jump.is_none();
                let selected_path = if preserve_selection {
                    this.active_document()
                        .map(|tab| tab.document.path().to_path_buf())
                } else {
                    active_path.clone()
                };
                let selected_tab = this.active_tab_id;
                stage.event("install_completed_begin");
                this.install_completed_documents(
                    opened,
                    selected_path.as_deref(),
                    &target_indices,
                    &opening_ids,
                    window,
                    cx,
                );
                if preserve_selection
                    && this.tabs.contains(&selected_tab)
                    && this.active_tab_id != selected_tab
                {
                    this.activate_workspace_tab(selected_tab, window, cx);
                }
                stage.event("install_completed_end");
                let frame_stage = crate::open_log::Stage::new(
                    "Document.completion_frames",
                    format_args!("paths={paths:?}"),
                );
                window.on_next_frame(move |window, _| {
                    window.on_next_frame(move |_, _| frame_stage.event("frame_callbacks_complete"));
                });
                this.open_task = None;
                this.maybe_restore_persisted_search(window, cx);
                for (path, completion) in move_completions {
                    let installed = this.documents.iter().any(|tab| {
                        paths_match(tab.document.path(), &path)
                            && tab.load_state == DocumentLoadState::Ready
                    });
                    completion.finish(installed, cx);
                }
                this.finish_pending_clipboard_edit(window, cx);
                this.open_queued_external_paths_if_idle(window, cx);
            });
        }));
    }

    pub(super) fn record_recent_paths(
        &mut self,
        paths: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(store) = self.persistence.store.clone() else {
            return;
        };
        if paths.is_empty() {
            return;
        }

        self.persistence.state_tasks.push(
            cx.spawn_in(window, async move |this, cx| {
                let result = cx
                    .background_spawn(async move {
                        store.record_opened(&paths)?;
                        Ok::<_, anyhow::Error>((store.recent_files(8)?, store.pinned_files()?))
                    })
                    .await;
                _ = this.update_in(cx, |this, window, cx| match result {
                    Ok((recent_files, pinned_files)) => {
                        this.recent_files = recent_files;
                        this.pinned_files = pinned_files;
                        cx.notify();
                    }
                    Err(error) => window.notify_message(
                        crate::tr_args!(
                            "最近文件未能保存：{error}",
                            "Couldn’t save recent files: {error}"
                        ),
                        cx,
                    ),
                });
            }),
            cx,
        );
    }
}
