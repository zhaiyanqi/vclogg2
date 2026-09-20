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
                            "将剪贴板中的 {byte_count} 字节文本写入临时文件，并在新标签中打开。",
                            "Write {byte_count} bytes of clipboard text to a temporary file and open it in a new tab."
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
                                .label(crate::tr!("打开临时文件", "Open temporary file")),
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
                        this.begin_open_paths(vec![path], window, cx);
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
