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
                    "这些目录是 AI 源码分析的默认范围。你也可以在问题中明确写出另一个绝对目录路径，AI 会在该轮临时切换或加入该目录，并可打开其中的文件。访问始终只读；这里的变更从下一轮生效。",
                    "These folders are the default scope for AI source analysis. You can also name another absolute folder path in your question; AI can add or switch to it for that run and open files from it. Access stays read-only, and changes here apply to the next run."
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
