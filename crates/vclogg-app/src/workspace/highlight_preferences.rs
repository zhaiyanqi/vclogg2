use super::*;

pub(super) struct PendingLogColoring {
    sequence: u64,
    group_id: String,
    enabled: bool,
}

impl Workspace {
    pub(super) fn log_coloring_selection(&self) -> (&str, bool) {
        self.log_coloring_pending.as_ref().map_or(
            (
                self.app_settings.log_coloring.active_group_id.as_str(),
                self.app_settings.highlight_log_levels,
            ),
            |pending| (pending.group_id.as_str(), pending.enabled),
        )
    }

    pub(super) fn open_color_labels_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_settings_dialog(Some(SettingsCategory::Highlight), window, cx);
    }

    pub(super) fn highlight_editor(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<ColorLabelsDialog> {
        cx.new(|cx| {
            ColorLabelsDialog::new(
                self.app_settings.highlight_log_levels,
                self.app_settings.log_coloring.clone(),
                self.color_labels.clone(),
                window,
                cx,
            )
            .with_selection_styles(self.app_settings.selection_styles.clone(), window, cx)
            .with_keyword_match_styles(
                self.app_settings.keyword_match_styles.clone(),
                window,
                cx,
            )
        })
    }

    pub(super) fn highlight_config(&self) -> crate::color_labels_dialog::LogColoringConfig {
        crate::color_labels_dialog::LogColoringConfig {
            log_coloring: self.app_settings.log_coloring.clone(),
            highlight_log_levels: self.app_settings.highlight_log_levels,
            selection_styles: self.app_settings.selection_styles.clone(),
            keyword_match_styles: self.app_settings.keyword_match_styles.clone(),
            labels: self.color_labels.clone(),
        }
    }

    pub(super) fn save_highlight_settings_dialog(
        &mut self,
        editor: Entity<ColorLabelsDialog>,
        baseline: crate::color_labels_dialog::LogColoringConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if editor.read(cx).is_saving() {
            return;
        }
        match editor.read(cx).config(cx) {
            Ok(draft) => {
                self.commit_highlight_change(draft, baseline, Some(editor), false, window, cx)
            }
            Err(error) => editor.update(cx, |editor, cx| editor.save_failed(error, cx)),
        }
    }

    pub(super) fn set_log_coloring_enabled(
        &mut self,
        enabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let baseline = self.highlight_config();
        let mut draft = baseline.clone();
        draft.highlight_log_levels = enabled;
        self.commit_highlight_change(draft, baseline, None, false, window, cx);
    }

    fn commit_highlight_change(
        &mut self,
        draft: crate::color_labels_dialog::LogColoringConfig,
        baseline: crate::color_labels_dialog::LogColoringConfig,
        editor: Option<Entity<ColorLabelsDialog>>,
        select_group: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if editor.is_some() && self.color_labels_saving {
            return;
        }
        let Some(store) = self.persistence.store.clone() else {
            let error = crate::tr!(
                "状态库尚未就绪，请稍后重试",
                "Storage is not ready. Try again shortly."
            )
            .to_string();
            if let Some(editor) = editor {
                editor.update(cx, |editor, cx| editor.save_failed(error, cx));
            } else {
                window.notify_message(error, cx);
            }
            return;
        };
        let quick_sequence = if editor.is_none() {
            self.log_coloring_request_sequence = self.log_coloring_request_sequence.wrapping_add(1);
            let group_id = if select_group {
                draft.log_coloring.active_group_id.clone()
            } else {
                self.log_coloring_selection().0.to_owned()
            };
            self.log_coloring_pending = Some(PendingLogColoring {
                sequence: self.log_coloring_request_sequence,
                group_id,
                enabled: draft.highlight_log_levels,
            });
            cx.notify();
            Some(self.log_coloring_request_sequence)
        } else {
            None
        };
        if let Some(editor) = &editor {
            editor.update(cx, |editor, cx| editor.begin_save(cx));
            self.color_labels_saving = true;
            window.refresh();
            cx.notify();
        }
        // Quick changes share the save queue without rebuilding the highlight editor.
        // Subsequent group selections stay usable while a previous acknowledgement is pending.
        let previous_save = self.persistence.app_settings_save_task.take();
        let (completion, receiver) = async_channel::bounded::<()>(1);
        let previous_highlight_save =
            cx.update_global::<WorkspaceWindowRegistry, _>(|registry, _| {
                registry
                    .highlight_settings_save_completion
                    .replace(receiver)
            });
        self.persistence.app_settings_save_task =
            Some(cx.spawn_in(window, async move |this, cx| {
                if let Some(previous) = previous_save {
                    previous.await;
                }
                if let Some(previous) = previous_highlight_save {
                    _ = previous.recv().await;
                }
                let quick = editor.is_none();
                let Ok((merged, settings)) = this.update_in(cx, |this, _, _| {
                    let mut merged = this.highlight_config();
                    if quick {
                        // A quick command is explicit even when its opening snapshot already had this value.
                        merged.highlight_log_levels = draft.highlight_log_levels;
                        if select_group {
                            merged.log_coloring.active_group_id =
                                draft.log_coloring.active_group_id.clone();
                        }
                    } else {
                        let previous_active = merged.log_coloring.active_group_id.clone();
                        merged.log_coloring = draft
                            .log_coloring
                            .merge(&baseline.log_coloring, &merged.log_coloring);
                        if draft.highlight_log_levels != baseline.highlight_log_levels {
                            merged.highlight_log_levels = draft.highlight_log_levels;
                        }
                        if !merged
                            .log_coloring
                            .groups
                            .iter()
                            .any(|group| group.id == previous_active)
                        {
                            merged.highlight_log_levels = false;
                        }
                        if draft.selection_styles != baseline.selection_styles {
                            merged.selection_styles = draft.selection_styles.clone();
                        }
                        if draft.keyword_match_styles != baseline.keyword_match_styles {
                            merged.keyword_match_styles = draft.keyword_match_styles.clone();
                        }
                        if draft.labels != baseline.labels {
                            merged.labels = draft.labels.clone();
                        }
                    }
                    if !merged
                        .log_coloring
                        .groups
                        .iter()
                        .any(|group| group.id == merged.log_coloring.active_group_id)
                    {
                        merged.log_coloring.active_group_id =
                            merged.log_coloring.groups[0].id.clone();
                        merged.highlight_log_levels = false;
                    }
                    let mut settings = this.app_settings.clone();
                    settings.highlight_log_levels = merged.highlight_log_levels;
                    settings.log_coloring = merged.log_coloring.clone();
                    settings.selection_styles = merged.selection_styles.clone();
                    settings.keyword_match_styles = merged.keyword_match_styles.clone();
                    (merged, settings)
                }) else {
                    return;
                };
                let labels = merged.labels.clone();
                let result = cx
                    .background_spawn(async move {
                        // Compile once off the UI thread; every window and table shares this snapshot.
                        let resolved = crate::color_labels::resolve_log_level_rules(
                            settings.log_coloring.active_rules(),
                        );
                        let result = if quick {
                            store.save_log_coloring(settings)
                        } else {
                            store.save_highlight_settings(settings, &labels)
                        };
                        result.map(|()| resolved)
                    })
                    .await;
                // Publish only acknowledged state. Failure leaves the previous global state intact.
                _ = this.update_in(cx, |this, window, cx| {
                    if !quick {
                        this.color_labels_saving = false;
                    }
                    if quick_sequence.is_some_and(|sequence| {
                        this.log_coloring_pending
                            .as_ref()
                            .is_some_and(|pending| pending.sequence == sequence)
                    }) {
                        this.log_coloring_pending = None;
                    }
                    match result {
                        Ok(resolved) => {
                            this.install_highlight_settings(&merged, &resolved, cx);
                            let source = window.window_handle();
                            let others = cx
                                .global::<WorkspaceWindowRegistry>()
                                .windows
                                .iter()
                                .filter(|entry| entry.window != source)
                                .map(|entry| entry.workspace.clone())
                                .collect::<Vec<_>>();
                            for workspace in others {
                                workspace.update(cx, |workspace, cx| {
                                    workspace.install_highlight_settings(&merged, &resolved, cx)
                                });
                            }
                            if editor.is_some() {
                                window.close_dialog(cx);
                            }
                        }
                        Err(error) => {
                            let message = crate::tr_args!(
                                "高亮配置未能保存：{error}",
                                "Couldn’t save highlight settings: {error}"
                            );
                            if let Some(editor) = &editor {
                                editor.update(cx, |editor, cx| editor.save_failed(message, cx));
                            } else {
                                window.notify_message(message, cx);
                            }
                        }
                    }
                    if !quick {
                        window.refresh();
                    }
                    cx.notify();
                });
                completion.close();
            }));
    }
    fn install_highlight_settings(
        &mut self,
        draft: &crate::color_labels_dialog::LogColoringConfig,
        resolved: &Arc<crate::color_labels::ResolvedLogLevelRules>,
        cx: &mut Context<Self>,
    ) {
        let appearance_changed = self.app_settings.selection_styles != draft.selection_styles
            || self.app_settings.keyword_match_styles != draft.keyword_match_styles
            || self.app_settings.log_coloring.active_rules() != draft.log_coloring.active_rules()
            || self.app_settings.highlight_log_levels != draft.highlight_log_levels;
        let labels_changed = self.color_labels != draft.labels;
        self.app_settings.selection_styles = draft.selection_styles.clone();
        self.app_settings.keyword_match_styles = draft.keyword_match_styles.clone();
        self.app_settings.highlight_log_levels = draft.highlight_log_levels;
        self.app_settings.log_coloring = draft.log_coloring.clone();
        if appearance_changed {
            for tab in &self.documents {
                for table in [&tab.log_table, &tab.result_table] {
                    table.update(cx, |table, cx| {
                        table
                            .delegate_mut()
                            .set_log_coloring(draft.highlight_log_levels, resolved.clone());
                        // Only paint changes; rebuilding the table's columns is unnecessary.
                        cx.notify();
                    });
                }
            }
            self.global_table.update(cx, |table, cx| {
                table
                    .delegate_mut()
                    .set_log_coloring(draft.highlight_log_levels, resolved.clone());
                cx.notify();
            });
        }
        if labels_changed {
            self.apply_color_labels(draft.labels.clone(), cx);
        }
        cx.notify();
    }
}
