use super::*;
#[cfg(test)]
use crate::color_labels_dialog::ColorLabelsDialog;

pub(super) struct PendingLogColoring {
    sequence: u64,
    group_id: String,
    enabled: bool,
}

enum HighlightChange {
    Quick,
    Autosave,
}

pub(super) fn merge_highlight_changes(
    draft: &crate::color_labels_dialog::LogColoringConfig,
    baseline: &crate::color_labels_dialog::LogColoringConfig,
    mut merged: crate::color_labels_dialog::LogColoringConfig,
) -> crate::color_labels_dialog::LogColoringConfig {
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
    merged
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

    #[cfg(test)]
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

    pub(super) fn set_log_coloring_enabled(
        &mut self,
        enabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let baseline = self.highlight_config();
        let mut draft = baseline.clone();
        draft.highlight_log_levels = enabled;
        self.commit_highlight_change(draft, baseline, HighlightChange::Quick, window, cx);
    }

    pub(super) fn autosave_highlight_settings(
        &mut self,
        draft: crate::color_labels_dialog::LogColoringConfig,
        baseline: crate::color_labels_dialog::LogColoringConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.commit_highlight_change(draft, baseline, HighlightChange::Autosave, window, cx);
    }

    fn commit_highlight_change(
        &mut self,
        draft: crate::color_labels_dialog::LogColoringConfig,
        baseline: crate::color_labels_dialog::LogColoringConfig,
        change: HighlightChange,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let quick = matches!(change, HighlightChange::Quick);
        let Some(store) = self.persistence.store.clone() else {
            let error = crate::tr!(
                "状态库尚未就绪，请稍后重试",
                "Storage is not ready. Try again shortly."
            )
            .to_string();
            window.notify_message(error, cx);
            return;
        };
        let quick_sequence = if quick {
            self.log_coloring_request_sequence = self.log_coloring_request_sequence.wrapping_add(1);
            let group_id = self.log_coloring_selection().0.to_owned();
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
                let Ok((merged, settings)) = this.update_in(cx, |this, _, _| {
                    let mut merged = this.highlight_config();
                    if quick {
                        // A quick command is explicit even when its opening snapshot already had this value.
                        merged.highlight_log_levels = draft.highlight_log_levels;
                    } else {
                        merged = merge_highlight_changes(&draft, &baseline, merged);
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
                            let source = cx.entity_id();
                            let others = cx
                                .global::<WorkspaceWindowRegistry>()
                                .windows
                                .iter()
                                .filter(|entry| entry.workspace.entity_id() != source)
                                .map(|entry| entry.workspace.clone())
                                .collect::<Vec<_>>();
                            for workspace in others {
                                workspace.update(cx, |workspace, cx| {
                                    workspace.install_highlight_settings(&merged, &resolved, cx)
                                });
                            }
                        }
                        Err(error) => {
                            if !quick {
                                this.settings_save_failed = true;
                            }
                            let message = crate::tr_args!(
                                "高亮配置未能保存：{error}",
                                "Couldn’t save highlight settings: {error}"
                            );
                            window.notify_message(message, cx);
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

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::TestAppContext;

    #[gpui_kit::test]
    fn async_highlight_save_completes_without_owning_window_dismissal(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::actions::init(cx);
            Workspace::init_window_registry(cx);
            crate::notifications::init(cx);
            crate::app_icon::init(cx);
        });
        let directory = tempfile::tempdir().unwrap();
        let store = Arc::new(StateStore::open(directory.path().join("state.db")).unwrap());
        let mut workspace = None;
        let window = cx.add_window(|window, cx| {
            let entity = cx.new(|cx| Workspace::new(false, Vec::new(), window, cx));
            entity.update(cx, |this, _| {
                this.persistence._bootstrap_task = Task::ready(());
                this.persistence.store = Some(store.clone());
            });
            workspace = Some(entity.clone());
            Root::new(entity, window, cx)
        });
        let workspace = workspace.unwrap();
        cx.update_window(window.into(), |_, window, cx| {
            workspace.update(cx, |this, cx| {
                let editor = this.highlight_editor(window, cx);
                let content = editor.clone();
                window.open_dialog(cx, move |dialog, _, _| dialog.child(content.clone()));
                let baseline = editor.read(cx).config(cx).unwrap();
                this.autosave_highlight_settings(
                    editor.read(cx).config(cx).unwrap(),
                    baseline,
                    window,
                    cx,
                );
            });
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(window.into(), |_, _, cx| {
            workspace.update(cx, |this, _| {
                assert!(!this.settings_save_failed);
            });
        })
        .unwrap();
    }
}
