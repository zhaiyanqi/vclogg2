use super::*;
use vclogg_ai::AiSettings;

#[derive(Default)]
pub(super) struct SharedAiSettings {
    pub settings: Option<AiSettings>,
    pub saving: bool,
}
impl gpui_kit::Global for SharedAiSettings {}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SettingsTab {
    Models,
    Skills,
    Tools,
    Prompts,
    Mcp,
    Workspace,
    Memory,
}
struct SettingsSurface {
    panel: Entity<AiPanel>,
    _subscription: Subscription,
}
impl Render for SettingsSurface {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.panel
            .update(cx, |panel, cx| panel.render_settings(window, cx))
    }
}
impl AiPanel {
    pub(super) fn settings_busy(&self, cx: &App) -> bool {
        self.busy || self.run.is_some() || cx.global::<SharedAiSettings>().saving
    }
    pub(in crate::workspace) fn open_settings(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.show_settings {
            return;
        }
        self.load_memories(cx);
        self.show_settings = true;
        self.settings_generation += 1;
        let panel = cx.entity();
        let surface = cx.new(|cx| SettingsSurface {
            _subscription: cx.observe(&panel, |_, _, cx| cx.notify()),
            panel: panel.clone(),
        });
        window.open_dialog(cx, move |dialog, window, cx| {
            let close = panel.clone();
            let submit = panel.clone();
            let content = surface.clone();
            dialog
                .close_button(false)
                .title(
                    h_flex()
                        .w_full()
                        .min_w_0()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .line_height(relative(AI_LABEL_LINE_HEIGHT))
                                .child(crate::tr!("大模型配置", "AI configuration")),
                        )
                        // Dispatch from this mounted dialog, even when current focus
                        // belongs to a dismissed popup or changed while switching windows.
                        .child(crate::dialog_focus::dialog_cancel_action(
                            "ai-settings-close-action",
                            Button::new("ai-settings-close")
                                .small()
                                .ghost()
                                .icon(IconName::Close)
                                .tooltip(crate::tr!("关闭配置", "Close configuration")),
                            cx,
                        )),
                )
                .w((window.rem_size() * 52.).min(window.viewport_size().width * 0.92))
                .h(window.viewport_size().height * 0.82)
                .overlay_closable(false)
                .content(move |container, _, _| {
                    container
                        .p_0()
                        .min_h_0()
                        .overflow_hidden()
                        .child(content.clone())
                })
                .on_ok(move |_, window, cx| {
                    submit.update(cx, |this, cx| match this.settings_tab {
                        SettingsTab::Models => this.save_provider(window, cx),
                        SettingsTab::Prompts => this.save_prompt_editor(window, cx),
                        SettingsTab::Mcp => this.save_mcp_editor(window, cx),
                        SettingsTab::Memory => this.save_memory_editor(window, cx),
                        SettingsTab::Skills | SettingsTab::Tools | SettingsTab::Workspace => {}
                    });
                    false
                })
                .on_close(move |_, _, cx| {
                    close.update(cx, |this, cx| {
                        this.show_settings = false;
                        this.settings_generation += 1;
                        this.editor = None;
                        this.prompt_editor = None;
                        this.mcp_editor = None;
                        this.memory_editor = None;
                        if let Some(token) = this.mcp_test_cancel.take() {
                            token.cancel();
                        }
                        this.mcp_test_task = None;
                        this.mcp_testing = false;
                        cx.notify();
                    })
                })
        });
        cx.notify();
    }
    fn render_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let mut tabs = h_flex()
            .gap_2()
            .flex_shrink_0()
            .flex_wrap()
            .p_3()
            .border_b_1()
            .border_color(cx.theme().border);
        for (id, tab, title) in [
            ("models", SettingsTab::Models, crate::tr!("模型", "Models")),
            ("skills", SettingsTab::Skills, "Skills"),
            ("tools", SettingsTab::Tools, crate::tr!("工具", "Tools")),
            ("mcp", SettingsTab::Mcp, "MCP"),
            (
                "prompts",
                SettingsTab::Prompts,
                crate::tr!("提示词与 RULES", "Prompts and RULES"),
            ),
            ("memory", SettingsTab::Memory, crate::tr!("记忆", "Memory")),
            (
                "workspace",
                SettingsTab::Workspace,
                crate::tr!("工作区", "Workspace"),
            ),
        ] {
            tabs = tabs.child(
                Button::new(id)
                    .small()
                    .ghost()
                    .text_label(title)
                    .selected(self.settings_tab == tab)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings_tab = tab;
                        cx.notify();
                    })),
            );
        }
        let content = match self.settings_tab {
            SettingsTab::Models => self.render_model_settings(window, cx),
            SettingsTab::Skills => self.render_skill_settings(cx),
            SettingsTab::Tools => self.render_tool_settings(cx),
            SettingsTab::Prompts => self.render_prompt_settings(cx),
            SettingsTab::Mcp => self.render_mcp_settings(cx),
            SettingsTab::Memory => self.render_memory_settings(cx),
            SettingsTab::Workspace => self.render_workspace_settings(cx),
        };
        v_flex()
            .size_full()
            .min_h_0()
            .min_w_0()
            .child(tabs)
            .when(!self.error.is_empty(), |this| {
                this.child(div().px_3().py_2().text_sm().child(self.error.clone()))
            })
            .child(content)
            .into_any_element()
    }
}
