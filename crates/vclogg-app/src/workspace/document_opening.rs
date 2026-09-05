use super::*;

impl Workspace {
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

    pub(super) fn open_dropped_paths(
        &mut self,
        paths: &ExternalPaths,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.file_drop_visible = false;
        self.file_drop_tab_transfer = None;
        if self.open_task.is_some() {
            window.push_notification(
                crate::tr!(
                    "当前正在打开其他文件，请稍后再拖入",
                    "Another file is being opened. Drop files again shortly.",
                ),
                cx,
            );
            return;
        }
        let (files, ignored_count): (Vec<_>, Vec<_>) = paths
            .paths()
            .iter()
            .cloned()
            .partition(|path| !path.is_dir());
        if files.is_empty() {
            window.push_notification(
                crate::tr!("请拖入一个或多个日志文件", "Drop one or more log files"),
                cx,
            );
            return;
        }
        if !ignored_count.is_empty() {
            window.push_notification(
                crate::tr_args!(
                    "已忽略 {} 个文件夹",
                    "Ignored {} folders",
                    ignored_count.len(),
                ),
                cx,
            );
        }
        self.begin_open_paths(files, window, cx);
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
        for path in paths {
            if !path.as_os_str().is_empty()
                && !self
                    .pending_external_paths
                    .iter()
                    .any(|queued| paths_match(queued, &path))
            {
                self.pending_external_paths.push(path);
            }
        }
        self.open_queued_external_paths_if_idle(window, cx);
    }

    pub(super) fn open_queued_external_paths_if_idle(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.open_task.is_some() || self.pending_external_paths.is_empty() {
            return;
        }
        let paths = std::mem::take(&mut self.pending_external_paths);
        self.begin_open_paths(paths, window, cx);
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
        let paths = deduplicate_paths(
            paths
                .into_iter()
                .filter(|path| !path.as_os_str().is_empty()),
        );
        if paths.is_empty() || self.open_task.is_some() {
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
                    WorkspaceTabId::New(id) => Some(id),
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

        self.open_task = Some(cx.spawn_in(window, async move |this, cx| {
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
                this.install_documents(
                    previews,
                    active_path.as_deref(),
                    &target_indices,
                    None,
                    false,
                    window,
                    cx,
                );
            });

            let full_paths = paths.clone();
            let full_store = fallback_store;
            let mut opened = cx
                .background_spawn(async move {
                    prepare_paths_bounded(full_paths, |path| {
                        prepare_document(
                            path,
                            path_buf_map_get(&cached_complete_documents, path).cloned(),
                            full_store.as_deref(),
                            path_buf_map_get(&sessions, path).cloned(),
                            SearchPreparationOptions {
                                max_results: effective_search_result_limit,
                                ..search_options
                            },
                            &color_labels,
                        )
                    })
                })
                .await;

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
                this.install_completed_documents(
                    opened,
                    active_path.as_deref(),
                    &target_indices,
                    &opening_ids,
                    window,
                    cx,
                );
                this.open_task = None;
                this.maybe_restore_persisted_search(window, cx);
                for (path, completion) in move_completions {
                    let installed = this.documents.iter().any(|tab| {
                        paths_match(tab.document.path(), &path)
                            && tab.load_state == DocumentLoadState::Ready
                    });
                    completion.finish(installed, cx);
                }
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

        self.persistence
            .state_tasks
            .push(cx.spawn_in(window, async move |this, cx| {
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
                    Err(error) => window.push_notification(
                        crate::tr_args!(
                            "最近文件未能保存：{error}",
                            "Couldn’t save recent files: {error}"
                        ),
                        cx,
                    ),
                });
            }));
    }
}
