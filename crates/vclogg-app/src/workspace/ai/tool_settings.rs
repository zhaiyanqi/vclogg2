use super::*;

fn category(name: &str) -> (&'static str, &'static str) {
    match name {
        "list_source_workspaces"
        | "rg_list_files"
        | "rg_search"
        | "rg_count"
        | "find_source_files"
        | "find_symbols"
        | "source_outline"
        | "locate_log_origin"
        | "find_definition"
        | "find_references"
        | "read_source" => ("source", crate::tr!("源码定位", "Source code")),
        "search_memory" | "save_memory" | "delete_memory" => {
            ("memory", crate::tr!("记忆", "Memory"))
        }
        "list_mcp_servers" | "list_mcp_tools" | "call_mcp_tool" => ("mcp", "MCP"),
        "get_context" | "list_logs" | "locate_files" | "list_log_directory" | "read_logs"
        | "search_logs" | "search_results" | "summarize_search" | "read_log_context"
        | "read_log_segment" => (
            "logs",
            crate::tr!("日志查找与读取", "Log search and reading"),
        ),
        _ => ("actions", crate::tr!("视图与操作", "Views and actions")),
    }
}

impl AiPanel {
    pub(super) fn render_tool_settings(&self, cx: &Context<Self>) -> AnyElement {
        let tools = vclogg_ai::tool_definitions();
        let mut content = v_flex().gap_3().p_3();
        content = content.child(div().text_xs().text_color(cx.theme().muted_foreground).child(crate::tr!("内置工具只在分析需要时调用。外部 MCP 工具请在 MCP 页面查看和配置。", "Built-in tools are called when needed. View and configure external MCP tools in the MCP tab.")));
        for (key, title) in [
            (
                "logs",
                crate::tr!("日志查找与读取", "Log search and reading"),
            ),
            ("source", crate::tr!("源码定位", "Source code")),
            ("context", crate::tr!("上下文", "Context")),
            ("actions", crate::tr!("视图与操作", "Views and actions")),
            ("memory", crate::tr!("记忆", "Memory")),
            ("mcp", "MCP"),
        ] {
            let group = tools
                .iter()
                .filter(|tool| category(tool.name).0 == key)
                .collect::<Vec<_>>();
            content = content.child(
                div()
                    .font_semibold()
                    .text_sm()
                    .child(format!("{title} ({})", group.len())),
            );
            for tool in group {
                content = content.child(
                    v_flex()
                        .gap_1()
                        .p_2()
                        .rounded(cx.theme().radius_lg)
                        .border_1()
                        .border_color(cx.theme().border)
                        .child(
                            h_flex()
                                .gap_2()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_semibold()
                                        .child(super::view::tool_label(tool.name)),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(tool.name),
                                ),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(tool.description),
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
