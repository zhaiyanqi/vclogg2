use anyhow::Context as _;
use std::io::Write as _;

use super::*;

impl Workspace {
    pub(super) fn paste_clipboard_as_file(
        &mut self,
        _: &PasteClipboardAsFile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.open_task.is_some() || window.has_active_dialog(cx) || window.has_active_sheet(cx) {
            return;
        }
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            window.notify_message(
                crate::tr!("剪贴板中没有文本", "The clipboard has no text"),
                cx,
            );
            return;
        };
        if text.is_empty() {
            window.notify_message(
                crate::tr!("剪贴板文本为空", "The clipboard text is empty"),
                cx,
            );
            return;
        }

        if !self.app_settings.confirm_clipboard_paste {
            self.create_clipboard_file_and_open(text, window, cx);
            return;
        }

        let byte_count = text.len();
        let workspace = cx.entity();
        window.open_dialog(cx, move |dialog, window, cx| {
            let workspace = workspace.clone();
            let text = text.clone();
            let height = window.rem_size() * 13.;
            let top = ((window.viewport_size().height - height) / 2.).max(px(0.));
            dialog
                .margin_top(top)
                .h(height)
                .close_button(false)
                .overlay_closable(false)
                .title(
                    h_flex()
                        .gap_2()
                        .child(Icon::new(IconName::File))
                        .child(crate::tr!(
                            "从剪贴板打开临时文件？",
                            "Open clipboard text as a temporary file?"
                        )),
                )
                .child(
                    div().text_sm().text_color(cx.theme().muted_foreground).child(
                        crate::tr_args!(
                            "将剪贴板中的 {byte_count} 字节文本写入临时文件，并在新标签中进入编辑模式。",
                            "Write {byte_count} bytes of clipboard text to a temporary file and edit it in a new tab."
                        ),
                    ),
                )
                .footer(
                    DialogFooter::new()
                        .justify_center()
                        .child(crate::dialog_focus::dialog_cancel_action(
                            "paste-clipboard-cancel-action",
                            Button::new("paste-clipboard-cancel")
                                .label(crate::tr!("取消", "Cancel")),
                            cx,
                        ))
                        .child(crate::dialog_focus::dialog_confirm_action(
                            "paste-clipboard-confirm-action",
                            Button::new("paste-clipboard-confirm")
                                .primary()
                                .label(crate::tr!("创建并编辑", "Create and edit")),
                            cx,
                        )),
                )
                .on_ok(move |_, window, cx| {
                    workspace.update(cx, |this, cx| {
                        this.create_clipboard_file_and_open(text.clone(), window, cx);
                    });
                    true
                })
        });
    }

    fn create_clipboard_file_and_open(
        &mut self,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.open_task.is_some() {
            window.notify_message(
                crate::tr!(
                    "当前正在打开其他文件，请稍后重试",
                    "Another file is being opened. Try again shortly."
                ),
                cx,
            );
            return;
        }
        self.file_refresh_task.take();
        self.open_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move { write_clipboard_temp_file(&text) })
                .await;
            _ = this.update_in(cx, |this, window, cx| {
                this.open_task = None;
                match result {
                    Ok(path) => {
                        this.transient_paths.insert(path_match_key(&path));
                        this.pending_clipboard_edit_path = Some(path.clone());
                        this.begin_open_paths(vec![path], window, cx);
                        if this.open_task.is_none() {
                            this.finish_pending_clipboard_edit(window, cx);
                        }
                    }
                    Err(error) => {
                        window.notify_message(
                            crate::tr_args!(
                                "无法创建剪贴板临时文件：{error}",
                                "Couldn’t create a temporary file from the clipboard: {error}"
                            ),
                            cx,
                        );
                        this.open_queued_external_paths_if_idle(window, cx);
                    }
                }
            });
        }));
    }

    pub(super) fn finish_pending_clipboard_edit(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = self.pending_clipboard_edit_path.take() else {
            return;
        };
        if let Some(document_id) = self
            .documents
            .iter()
            .find(|tab| {
                paths_match(tab.document.path(), &path)
                    && tab.load_state == DocumentLoadState::Ready
            })
            .map(|tab| tab.id)
        {
            self.enter_document_edit(document_id, window, cx);
        }
    }
}

fn write_clipboard_temp_file(text: &str) -> anyhow::Result<PathBuf> {
    let root = crate::app_paths::temporary_dir().context(crate::tr!(
        "无法确定临时文件目录",
        "Couldn’t determine the temporary-file directory"
    ))?;
    std::fs::create_dir_all(&root)?;
    let mut file = tempfile::Builder::new()
        .prefix("vclogg2-clipboard-")
        .suffix(".log")
        .tempfile_in(root)?;
    file.write_all(text.as_bytes())?;
    file.flush()?;
    let (file, path) = file.keep()?;
    drop(file);
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::TestAppContext;

    #[test]
    fn clipboard_text_is_written_to_a_temp_log_file() {
        let path = write_clipboard_temp_file("first\nsecond\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first\nsecond\n");
        std::fs::remove_file(path).unwrap();
    }

    #[gpui_kit::test]
    fn completed_clipboard_open_starts_editing(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            Workspace::init_window_registry(cx);
            crate::notifications::init(cx);
            crate::app_icon::init(cx);
        });
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("clipboard.log");
        std::fs::write(&path, "pasted text\n").unwrap();
        let document = Arc::new(LogDocument::open(&path).unwrap());
        let mut workspace = None;
        let window = cx.add_window(|window, cx| {
            let entity = cx.new(|cx| Workspace::new(false, Vec::new(), window, cx));
            workspace = Some(entity.clone());
            Root::new(entity, window, cx)
        });
        let workspace = workspace.unwrap();
        cx.update_window(window.into(), |_, window, cx| {
            workspace.update(cx, |workspace, cx| {
                let prepared = PreparedDocument {
                    document: document.clone(),
                    cached_complete_document: None,
                    session: None,
                    color_labels_snapshot: None,
                    resolved_color_rules: Arc::default(),
                    search_result: SearchResult::default(),
                    search_range: SearchRange::default(),
                    search_matcher: None,
                    search_case_sensitive: false,
                    search_regex: false,
                    warning: None,
                    load_state: DocumentLoadState::Ready,
                    pending_index_cache: None,
                    upgrade_frame: None,
                };
                workspace.install_documents(
                    vec![(path.clone(), Ok(prepared))],
                    Some(&path),
                    &BTreeMap::new(),
                    None,
                    true,
                    window,
                    cx,
                );
                workspace.pending_clipboard_edit_path = Some(path.clone());
                workspace.finish_pending_clipboard_edit(window, cx);
                assert!(workspace.pending_clipboard_edit_path.is_none());
                assert!(workspace.documents[0].edit_load_task.is_some());
            });
        })
        .unwrap();
    }
}
