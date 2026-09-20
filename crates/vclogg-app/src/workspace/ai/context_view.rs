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
