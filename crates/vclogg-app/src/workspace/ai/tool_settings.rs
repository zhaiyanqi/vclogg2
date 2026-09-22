use super::*;
use vclogg_ai::{ToolGroup, ToolRisk};

impl AiPanel {
    pub(super) fn render_tool_settings(&self, cx: &Context<Self>) -> AnyElement {
        let tools = vclogg_ai::tool_descriptors();
        let mut content = v_flex().gap_3().p_3().child(div().text_xs().text_color(cx.theme().muted_foreground).child(crate::tr!(
            "工具按能力组加载。应用内操作自动执行；外部操作根据实际风险自动执行、请求确认或拒绝。Skills 只提供工作指导。",
            "Tools load by capability. In-app actions run automatically; external actions are assessed for automatic execution, confirmation or denial. Skills provide guidance."
        )));
        for group in std::iter::once(ToolGroup::Core).chain(ToolGroup::OPTIONAL) {
            let available = match group {
                ToolGroup::Memory => self.settings.memory_enabled,
                ToolGroup::Mcp => self.settings.mcp_servers.iter().any(|s| s.enabled()),
                ToolGroup::Skills => self
                    .settings
                    .skills
                    .iter()
                    .any(|s| self.settings.skill_enabled(s)),
                ToolGroup::SourceSearch | ToolGroup::SourceSymbols | ToolGroup::Shell => {
                    !self.settings.workspace_directories.is_empty()
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
