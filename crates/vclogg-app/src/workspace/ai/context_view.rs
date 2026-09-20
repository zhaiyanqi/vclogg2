use super::*;
use gpui_kit::Anchor;
use gpui_kit::component::progress::ProgressCircle;
use vclogg_ai::{ContextUsage, conversation_tokens};

fn tokens(value: u32) -> String {
    if value >= 1_000 {
        format!("{:.1}K", value as f64 / 1_000.0)
    } else {
        value.to_string()
    }
}

impl AiPanel {
    pub(super) fn render_context_sources_popover(&self, cx: &mut Context<Self>) -> AnyElement {
        let panel = cx.entity();
        let weak_panel = cx.weak_entity();
        let logs = self
            .workspace
            .read_with(cx, |workspace, _| {
                workspace
                    .documents
                    .iter()
                    .filter(|tab| tab.load_state == DocumentLoadState::Ready)
                    .map(|tab| tab.id)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let count = logs
            .iter()
            .filter(|id| {
                self.selected_log_ids
                    .as_ref()
                    .is_none_or(|selected| selected.contains(id))
            })
            .count();
        let label = format!(
            "{} · {} {} · {} {}",
            crate::tr!("本轮访问", "Run access"),
            count,
            crate::tr!("日志", "logs"),
            self.selected_workspace_directories
                .as_ref()
                .map_or(self.settings.workspace_directories.len(), BTreeSet::len),
            crate::tr!("项目", "projects")
        );
        Popover::new("ai-context-sources-popover")
            .anchor(Anchor::BottomLeft)
            .open(self.show_context_sources)
            .on_open_change(move |open, _, cx| {
                _ = weak_panel.update(cx, |this, cx| {
                    this.show_context_sources = *open;
                    cx.notify();
                });
            })
            .p_0()
            .trigger(
                Button::new("ai-context-sources")
                    .small()
                    .ghost()
                    .text_label(label.clone())
                    .tooltip(label),
            )
            .content(move |_, window, popover_cx| {
                let width = (window.rem_size() * 25.).min(window.viewport_size().width * 0.8);
                div()
                    .w(width)
                    .child(panel.update(popover_cx, |this, cx| this.render_context_sources(cx)))
            })
            .into_any_element()
    }

    fn render_context_sources(&self, cx: &mut Context<Self>) -> AnyElement {
        let logs = self
            .workspace
            .read_with(cx, |workspace, _| {
                workspace
                    .documents
                    .iter()
                    .filter(|tab| tab.load_state == DocumentLoadState::Ready)
                    .map(|tab| (tab.id, tab.document.file_name().to_owned()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let all_log_ids = logs.iter().map(|(id, _)| *id).collect::<BTreeSet<_>>();
        let disabled = self.settings_busy(cx) || self.ui_busy;
        let mut content = v_flex()
            .gap_2()
            .p_3()
            .child(div().text_sm().font_semibold().child(crate::tr!(
                "发送前选择可访问的来源",
                "Choose sources before sending"
            )))
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(crate::tr!(
                        "选择对下一轮生效；附加的日志行始终包含在本轮中。",
                        "Choices apply to the next run; attached log lines are always included."
                    )),
            )
            .child(
                Button::new("ai-scope-reset")
                    .small()
                    .ghost()
                    .text_label(crate::tr!("恢复全部访问", "Restore all access"))
                    .disabled(disabled)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.selected_log_ids = None;
                        this.selected_workspace_directories = None;
                        this.include_search_directory = true;
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .text_xs()
                    .font_semibold()
                    .child(crate::tr!("已打开日志", "Open logs")),
            );
        for (id, name) in logs {
            let selected = self
                .selected_log_ids
                .as_ref()
                .is_none_or(|ids| ids.contains(&id));
            let all = all_log_ids.clone();
            content = content.child(
                Button::new(SharedString::from(format!("ai-scope-log-{id}")))
                    .small()
                    .ghost()
                    .justify_start()
                    .w_full()
                    .text_label(format!("{} {name}", if selected { "✓" } else { "○" }))
                    .disabled(disabled)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let selected = this.selected_log_ids.get_or_insert_with(|| all.clone());
                        if !selected.insert(id) {
                            selected.remove(&id);
                        }
                        cx.notify();
                    })),
            );
        }
        if all_log_ids.is_empty() {
            content = content.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(crate::tr!("没有已打开的日志", "No open logs")),
            );
        }
        let directory = self
            .workspace
            .read_with(cx, |workspace, _| {
                workspace.global_search.directory_options.directory.clone()
            })
            .ok()
            .flatten();
        if let Some(path) = directory {
            content = content.child(
                div()
                    .pt_2()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(
                        Button::new("ai-scope-directory")
                            .small()
                            .ghost()
                            .justify_start()
                            .w_full()
                            .text_label(format!(
                                "{} {}: {}",
                                if self.include_search_directory {
                                    "✓"
                                } else {
                                    "○"
                                },
                                crate::tr!("搜索目录", "Search folder"),
                                path.display()
                            ))
                            .disabled(disabled)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.include_search_directory = !this.include_search_directory;
                                cx.notify();
                            })),
                    ),
            );
        }
        content = content.child(
            div()
                .pt_2()
                .border_t_1()
                .border_color(cx.theme().border)
                .text_xs()
                .font_semibold()
                .child(crate::tr!("源码项目", "Source projects")),
        );
        let all_projects = self
            .settings
            .workspace_directories
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        for path in &self.settings.workspace_directories {
            let selected = self
                .selected_workspace_directories
                .as_ref()
                .is_none_or(|paths| paths.contains(path));
            let path = path.clone();
            let all = all_projects.clone();
            let label = format!("{} {}", if selected { "✓" } else { "○" }, path.display());
            content = content.child(
                Button::new(SharedString::from(format!(
                    "ai-scope-project-{}",
                    path.display()
                )))
                .small()
                .ghost()
                .justify_start()
                .w_full()
                .text_label(label)
                .disabled(disabled)
                .on_click(cx.listener(move |this, _, _, cx| {
                    let selected = this
                        .selected_workspace_directories
                        .get_or_insert_with(|| all.clone());
                    if !selected.insert(path.clone()) {
                        selected.remove(&path);
                    }
                    cx.notify();
                })),
            );
        }
        if all_projects.is_empty() {
            content = content.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(crate::tr!(
                        "未配置源码项目",
                        "No source projects configured"
                    )),
            );
        }
        div()
            .id("ai-context-sources-scroll")
            .max_h(gpui_kit::rems(25.))
            .overflow_y_scroll()
            .child(content)
            .into_any_element()
    }

    fn current_usage(&self) -> ContextUsage {
        let mut usage = self.conversation.context_usage.clone().unwrap_or_default();
        usage.conversation_tokens = conversation_tokens(&self.conversation.active_messages());
        if let Some(provider) = self
            .settings
            .providers
            .iter()
            .find(|provider| Some(&provider.id) == self.conversation.provider_id.as_ref())
        {
            usage.context_window_tokens = provider.context_window_tokens;
        }
        usage
    }

    pub(super) fn context_usage_label(&self) -> String {
        let usage = self.current_usage();
        match usage.percent() {
            Some(percent) => format!("{} {percent}%", crate::tr!("上下文", "Context")),
            None => crate::tr!("上下文用量", "Context usage").into(),
        }
    }

    pub(super) fn render_context_usage_popover(&self, cx: &mut Context<Self>) -> AnyElement {
        let percent = self.current_usage().percent().unwrap_or(0) as f32;
        let label = self.context_usage_label();
        let panel = cx.entity();
        let weak_panel = cx.weak_entity();
        Popover::new("ai-context-usage-popover")
            .anchor(Anchor::BottomRight)
            .open(self.show_context_usage)
            .on_open_change(move |open, _, cx| {
                _ = weak_panel.update(cx, |this, cx| {
                    this.show_context_usage = *open;
                    cx.notify();
                });
            })
            .p_0()
            .trigger(
                Button::new("ai-context-usage")
                    .small()
                    .ghost()
                    .icon(
                        ProgressCircle::new("ai-context-usage-ring")
                            .value(percent)
                            .accessibility_label(label.clone()),
                    )
                    .accessibility_label(label.clone())
                    .tooltip(label),
            )
            .content(move |_, window, popover_cx| {
                let width = (window.rem_size() * 22.).min(window.viewport_size().width * 0.8);
                div()
                    .w(width)
                    .child(panel.update(popover_cx, |this, cx| this.render_context_usage(cx)))
            })
            .into_any_element()
    }

    pub(super) fn render_context_usage(&self, cx: &Context<Self>) -> AnyElement {
        let usage = self.current_usage();
        let total = usage.total();
        let percent = usage.percent();
        let headline = match (percent, usage.context_window_tokens) {
            (Some(percent), Some(limit)) => format!(
                "{} {percent}% · ~{} / {} tokens",
                crate::tr!("上下文", "Context"),
                tokens(total),
                tokens(limit)
            ),
            _ => format!(
                "{} ~{} tokens · {}",
                crate::tr!("上下文", "Context"),
                tokens(total),
                crate::tr!("模型上限未知", "Model limit unknown")
            ),
        };
        let mut card = v_flex()
            .gap_2()
            .p_3()
            .child(div().text_sm().font_semibold().child(headline));
        let categories = [
            (
                crate::tr!("系统与工作区", "System and workspace"),
                usage.system_tokens,
                cx.theme().chart_1,
            ),
            (
                crate::tr!("提示词与 RULES", "Prompts and RULES"),
                usage.prompt_tokens,
                cx.theme().chart_2,
            ),
            ("Skills", usage.skill_tokens, cx.theme().chart_3),
            (
                crate::tr!("工具定义", "Tool definitions"),
                usage.tool_tokens,
                cx.theme().chart_4,
            ),
            (
                crate::tr!("对话", "Conversation"),
                usage.conversation_tokens,
                cx.theme().chart_5,
            ),
        ];
        if let Some(limit) = usage.context_window_tokens.filter(|limit| *limit > 0) {
            let fraction = (total as f32 / limit as f32).clamp(0.0, 1.0);
            let mut filled = h_flex()
                .h_full()
                .w(relative(fraction))
                .overflow_hidden()
                .rounded(cx.theme().radius_full());
            if total > 0 {
                for (_, value, color) in categories {
                    filled = filled.child(
                        div()
                            .h_full()
                            .w(relative(value as f32 / total as f32))
                            .bg(color),
                    );
                }
            }
            card = card.child(
                div()
                    .h_2()
                    .w_full()
                    .rounded(cx.theme().radius_full())
                    .bg(cx.theme().muted)
                    .child(filled),
            );
        }
        for (label, value, color) in categories {
            card = card.child(
                h_flex()
                    .gap_2()
                    .child(
                        div()
                            .w_2()
                            .h_2()
                            .rounded(cx.theme().radius_full())
                            .bg(color),
                    )
                    .child(div().flex_1().text_xs().child(label))
                    .child(div().text_xs().child(tokens(value))),
            );
        }
        card = card.child(div().text_xs().text_color(cx.theme().muted_foreground).child(crate::tr!("用量根据当前请求内容估算；模型实际计数可能不同。", "Usage is estimated from the current request; the model may count tokens differently.")));
        if let Some(provider) = self
            .settings
            .providers
            .iter()
            .find(|provider| Some(&provider.id) == self.conversation.provider_id.as_ref())
            && usage.context_window_tokens.is_some()
        {
            card = card.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        if provider.context_window_source.as_deref() == Some("provider") {
                            crate::tr!(
                                "上下文上限由模型服务提供",
                                "Context limit provided by model service"
                            )
                        } else {
                            crate::tr!("上下文上限为手动设置", "Context limit set manually")
                        },
                    ),
            );
        }
        card.into_any_element()
    }
}
