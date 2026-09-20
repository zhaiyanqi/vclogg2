use super::*;

impl AiPanel {
    fn add_workspace_directory(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings_busy(cx) {
            return;
        }
        self.busy = true;
        cx.spawn_in(window, async move |this, cx| {
            let selected = rfd::AsyncFileDialog::new().pick_folder().await;
            let canonical = selected.and_then(|folder| folder.path().canonicalize().ok());
            _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                if let Some(path) = canonical
                    && !this.settings.workspace_directories.contains(&path)
                {
                    this.settings.workspace_directories.push(path);
                    this.save_settings(window, cx);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn render_workspace_settings(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let disabled = self.settings_busy(cx);
        let mut content = v_flex().gap_3().p_3()
            .child(Button::new("ai-add-workspace-directory").small()
                .text_label(crate::tr!("添加项目目录…", "Add project folder…"))
                .disabled(disabled)
                .on_click(cx.listener(|this, _, window, cx| this.add_workspace_directory(window, cx))))
            .child(div().text_sm().text_color(cx.theme().muted_foreground)
                .child(crate::tr!(
                    "分析日志时，AI 可使用 rg 在以下项目目录中查找代码，并读取相关源码。目录仅供只读分析，变更在下一轮对话生效。",
                    "When analyzing logs, AI can use rg to search these project folders and read related source code. Access is read-only and changes apply to the next run."
                )));
        if self.settings.workspace_directories.is_empty() {
            content = content.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(crate::tr!("尚未添加项目目录", "No project folders added")),
            );
        }
        for (index, path) in self
            .settings
            .workspace_directories
            .clone()
            .into_iter()
            .enumerate()
        {
            let label = path.display().to_string();
            content = content.child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .pt_3()
                    .child(div().flex_1().min_w_0().truncate().text_sm().child(label))
                    .child(
                        Button::new(SharedString::from(format!("ai-remove-workspace-{index}")))
                            .small()
                            .ghost()
                            .text_label(crate::tr!("移除", "Remove"))
                            .disabled(disabled)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if index < this.settings.workspace_directories.len() {
                                    this.settings.workspace_directories.remove(index);
                                    this.save_settings(window, cx);
                                }
                            })),
                    ),
            );
        }
        div()
            .id("ai-workspaces-scroll")
            .size_full()
            .overflow_y_scroll()
            .child(content)
            .into_any_element()
    }
}
