use super::*;
use vclogg_ai::{ToolGroup, ToolRisk};

impl ConversationSession {
    pub(super) fn render_tool_settings(&self, cx: &Context<Self>) -> AnyElement {
        let tools = vclogg_ai::tool_descriptors();
        let mut content = v_flex().gap_3().p_3().child(div().text_xs().text_color(cx.theme().muted_foreground).child(crate::tr!(
            "工具按能力组加载。应用内操作自动执行；外部操作根据实际风险自动执行、请求确认或拒绝。Skills 只提供工作指导。",
            "Tools load by capability. In-app actions run automatically; external actions are assessed for automatic execution, confirmation or denial. Skills provide guidance."
        )));
        content = content.child(
            v_flex().gap_2()
                .child(h_flex().gap_2()
                    .child(div().text_sm().font_semibold().child(crate::tr!("已允许的命令", "Allowed commands")))
                    .child(Button::new("ai-approvals-refresh").small().ghost()
                        .text_label(crate::tr!("刷新", "Refresh"))
                        .disabled(self.approvals_loading)
                        .on_click(cx.listener(|this, _, _, cx| this.load_command_approvals(None, cx)))))
                .child(div().text_xs().text_color(cx.theme().muted_foreground).child(crate::tr!(
                    "仅匹配完整命令和工作目录，跨会话保存。撤销后，下次执行将重新询问。",
                    "Exact command and directory; saved across conversations. Revoking asks again on the next execution."
                )))
                .when(self.command_approvals.is_empty(), |view| view.child(div().text_xs().child(
                    if self.approvals_loading { crate::tr!("正在加载…", "Loading…") } else { crate::tr!("暂无允许的命令", "No allowed commands") }
                )))
                .children(self.command_approvals.iter().map(|(directory, command)| {
                    let rule = (directory.clone(), command.clone());
                    let id = format!("ai-revoke-command-{directory:?}-{command:?}");
                    v_flex().gap_1().py_2().border_b_1().border_color(cx.theme().border)
                        .child(div().text_sm().child(command.clone()))
                        .child(div().text_xs().text_color(cx.theme().muted_foreground).child(directory.display().to_string()))
                        .child(Button::new(id).small().ghost().text_label(crate::tr!("撤销允许", "Revoke allowance"))
                            .disabled(self.approvals_loading)
                            .on_click(cx.listener(move |this, _, _, cx| this.load_command_approvals(Some(rule.clone()), cx))))
                }))
        );
        for group in std::iter::once(ToolGroup::Core).chain(ToolGroup::OPTIONAL) {
            let available = match group {
                ToolGroup::Memory => self.settings.memory_enabled,
                ToolGroup::Mcp => self.settings.mcp_servers.iter().any(|s| s.enabled()),
                ToolGroup::Skills => self
                    .settings
                    .skills
                    .iter()
                    .any(|s| self.settings.skill_enabled(s)),
                ToolGroup::SourceSearch | ToolGroup::SourceSymbols => {
                    !self.settings.project_directories.is_empty()
                }
                _ => true,
            };
            content = content.child(div().font_semibold().text_sm().child(format!(
                "{} · {}",
                group.id(),
                if available {
                    crate::tr!("可用", "Available")
                } else {
                    crate::tr!(
                        "需要配置或本轮提供范围",
                        "Requires configuration or a run scope"
                    )
                }
            )));
            for tool in tools.iter().filter(|tool| tool.group() == group) {
                let policy = match tool.risk() {
                    ToolRisk::Observe => crate::tr!("只读 · 自动", "Read · Automatic"),
                    ToolRisk::Navigate => crate::tr!("界面操作 · 自动", "Navigation · Automatic"),
                    ToolRisk::PersistLocal => {
                        crate::tr!("本地状态 · 自动", "Local state · Automatic")
                    }
                    ToolRisk::External => crate::tr!(
                        "外部操作 · 按风险自动 / 确认 / 拒绝",
                        "External · Automatic / Confirm / Deny by risk"
                    ),
                };
                content = content.child(
                    v_flex()
                        .gap_1()
                        .py_2()
                        .border_b_1()
                        .border_color(cx.theme().border)
                        .child(
                            h_flex()
                                .gap_2()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_semibold()
                                        .child(super::view::tool_label(tool.name())),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(policy),
                                ),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(tool.definition().description),
                        ),
                );
            }
        }
        div()
            .id("ai-tools-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .child(content)
            .into_any_element()
    }
}

impl ConversationSession {
    pub(super) fn load_command_approvals(
        &mut self,
        revoke: Option<(PathBuf, String)>,
        cx: &mut Context<Self>,
    ) {
        if self.approvals_loading {
            return;
        }
        let Some(store) = self.store.clone() else {
            return;
        };
        self.approvals_loading = true;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    if let Some((directory, command)) = revoke {
                        store.revoke_ai_command(&directory, &command)?;
                    }
                    store.ai_command_approvals()
                })
                .await;
            _ = this.update(cx, |this, cx| {
                this.approvals_loading = false;
                match result {
                    Ok(rules) => this.command_approvals = rules,
                    Err(error) => this.error = error.to_string(),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}
