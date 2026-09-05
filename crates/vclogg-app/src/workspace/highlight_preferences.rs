use super::*;

impl Workspace {
    pub(super) fn open_color_labels_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.color_labels_saving {
            return;
        }
        let editor = cx.new(|cx| {
            ColorLabelsDialog::new(
                self.app_settings.highlight_log_levels,
                self.app_settings.log_level_color_rules.clone(),
                self.color_labels.clone(),
                window,
                cx,
            )
            .with_selection_styles(
                self.app_settings.selection_styles.clone(),
                window,
                cx,
            )
        });
        let workspace = cx.entity();
        let (dialog_size, margin_top) = management_dialog_geometry(window);
        window.open_dialog(cx, move |dialog, _, cx| {
            let saving = editor.read(cx).is_saving();
            let content = editor.clone();
            let cancel = editor.clone();
            let editor = editor.clone();
            let workspace = workspace.clone();
            dialog
                .w(dialog_size.width)
                .h(dialog_size.height)
                .margin_top(margin_top)
                .title(crate::tr!("高亮配置", "Highlight settings"))
                .close_button(false)
                .keyboard(!saving)
                .overlay_closable(!saving)
                .content(move |area, _, _| area.min_h_0().overflow_hidden().child(content.clone()))
                .footer(
                    DialogFooter::new()
                        .child(crate::dialog_focus::dialog_cancel_action(
                            "highlight-settings-cancel-action",
                            Button::new("highlight-settings-cancel")
                                .label(crate::tr!("取消", "Cancel"))
                                .disabled(saving),
                            cx,
                        ))
                        .child(crate::dialog_focus::dialog_confirm_action_when(
                            "highlight-settings-save-action",
                            Button::new("highlight-settings-save")
                                .primary()
                                .label(crate::tr!("保存", "Save"))
                                .loading(saving)
                                .disabled(saving),
                            !saving,
                            cx,
                        )),
                )
                .on_cancel(move |_, _, cx| !cancel.read(cx).is_saving())
                .on_ok(move |_, window, cx| {
                    workspace.update(cx, |this, cx| {
                        this.save_highlight_settings_dialog(editor.clone(), window, cx)
                    });
                    // The editor closes only after the database acknowledges this draft.
                    false
                })
        });
    }

    fn save_highlight_settings_dialog(
        &mut self,
        editor: Entity<ColorLabelsDialog>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if editor.read(cx).is_saving() {
            return;
        }
        let Some(store) = self.persistence.store.clone() else {
            editor.update(cx, |editor, cx| {
                editor.save_failed(
                    crate::tr!(
                        "状态库尚未就绪，请稍后重试",
                        "Storage is not ready. Try again shortly."
                    )
                    .into(),
                    cx,
                )
            });
            return;
        };
        let draft = match editor.read(cx).config(cx) {
            Ok(draft) => draft,
            Err(error) => {
                editor.update(cx, |editor, cx| editor.save_failed(error, cx));
                return;
            }
        };
        let mut settings = self.app_settings.clone();
        settings.selection_styles = draft.selection_styles.clone();
        settings.highlight_log_levels = draft.highlight_log_levels;
        settings.log_level_color_rules = draft.log_level_rules.clone();
        editor.update(cx, |editor, cx| editor.begin_save(cx));
        self.color_labels_saving = true;
        window.refresh();
        let previous_save = self.persistence.app_settings_save_task.take();
        // The completion channel closes on success, failure or task cancellation. Keeping the
        // sender until after UI installation serializes acknowledgements across windows too.
        let (completion, receiver) = async_channel::bounded::<()>(1);
        let previous_highlight_save =
            cx.update_global::<WorkspaceWindowRegistry, _>(|registry, _| {
                registry
                    .highlight_settings_save_completion
                    .replace(receiver)
            });
        self.persistence.app_settings_save_task =
            Some(cx.spawn_in(window, async move |this, cx| {
                if let Some(previous_save) = previous_save {
                    previous_save.await;
                }
                if let Some(previous_highlight_save) = previous_highlight_save {
                    _ = previous_highlight_save.recv().await;
                }
                let to_save = draft.clone();
                let result = cx
                    .background_spawn(async move {
                        store.save_highlight_settings(settings, &to_save.labels)
                    })
                    .await;
                _ = this.update_in(cx, |this, window, cx| {
                    this.color_labels_saving = false;
                    window.refresh();
                    match result {
                        Ok(()) => {
                            // Merge into the latest settings in every window, never the dialog-opening snapshot.
                            this.install_highlight_settings(&draft, cx);
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
                                    workspace.install_highlight_settings(&draft, cx);
                                });
                            }
                            window.close_dialog(cx);
                            window.refresh();
                            cx.notify();
                        }
                        Err(error) => editor.update(cx, |editor, cx| {
                            editor.save_failed(
                                crate::tr_args!(
                                    "高亮配置未能保存：{error}",
                                    "Couldn’t save highlight settings: {error}"
                                ),
                                cx,
                            )
                        }),
                    }
                });
                completion.close();
            }));
    }
    fn install_highlight_settings(
        &mut self,
        draft: &crate::color_labels_dialog::LogColoringConfig,
        cx: &mut Context<Self>,
    ) {
        self.app_settings.selection_styles = draft.selection_styles.clone();
        self.app_settings.highlight_log_levels = draft.highlight_log_levels;
        self.app_settings.log_level_color_rules = draft.log_level_rules.clone();
        for tab in &mut self.documents {
            tab.refresh_appearance(&self.app_settings, cx);
            tab.refresh_log_level_highlighting(draft.highlight_log_levels, cx);
        }
        self.global_table.update(cx, |table, cx| {
            table.delegate_mut().set_appearance(&self.app_settings);
            table
                .delegate_mut()
                .set_highlight_log_levels(draft.highlight_log_levels);
            table.refresh(cx);
            cx.notify();
        });
        self.refresh_active_log_search_presentation(cx);
        self.apply_color_labels(draft.labels.clone(), cx);
        cx.notify();
    }
}
