use super::*;
use vclogg_ai::{AgentMode, StepStatus};

fn mode_label(mode: AgentMode) -> &'static str {
    match mode {
        AgentMode::Ask => "Ask",
        AgentMode::Plan => "Plan",
        AgentMode::Execute => "Agent",
    }
}
impl ConversationSession {
    pub(super) fn render_composer_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let owner = cx.entity();
        Button::new("ai-add-context")
            .small()
            .ghost()
            .icon(IconName::Plus)
            .tooltip(crate::tr!("模式与附件", "Mode and attachments"))
            .dropdown_menu(move |mut menu, window, cx| {
                for mode in [AgentMode::Ask, AgentMode::Plan, AgentMode::Execute] {
                    menu = menu.item(
                        PopupMenuItem::new(mode_label(mode))
                            .checked(owner.read(cx).conversation.mode == mode)
                            .on_click(window.listener_for(&owner, move |this, _, _, cx| {
                                this.conversation.mode = mode;
                                this.preferences_dirty = true;
                                this.persist_preferences(cx);
                                cx.notify();
                            })),
                    );
                }
                menu.separator().item(
                    PopupMenuItem::new(crate::tr!("附加所选日志", "Attach selected logs"))
                        .disabled({
                            let session = owner.read(cx);
                            session.attachments_loading
                                || ((session.busy || session.ui_busy) && session.run.is_none())
                        })
                        .on_click(window.listener_for(&owner, |this, _, window, cx| {
                            let targets = this
                                .workspace
                                .read_with(cx, |workspace, cx| {
                                    workspace.ai_attachment_targets(workspace.active_log_region, cx)
                                })
                                .unwrap_or_default();
                            if targets.is_empty() {
                                this.error = crate::tr!(
                                    "先在日志区域选择要附加的行",
                                    "Select log rows to attach first"
                                )
                                .into();
                                cx.notify();
                            } else {
                                this.attach_logs(targets, window, cx);
                            }
                        })),
                )
            })
            .into_any_element()
    }
    pub(super) fn render_plan(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.conversation.plan.is_empty() {
            return div().into_any_element();
        }
        let completed = self
            .conversation
            .plan
            .iter()
            .filter(|s| s.status == StepStatus::Complete)
            .count();
        v_flex()
            .gap_1()
            .child(
                Button::new("ai-plan-toggle")
                    .small()
                    .ghost()
                    .text_label(format!(
                        "{} {completed}/{}",
                        crate::tr!("任务步骤", "Task steps"),
                        self.conversation.plan.len()
                    ))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_plan = !this.show_plan;
                        cx.notify();
                    })),
            )
            .when(self.show_plan, |view| {
                view.child(
                    div()
                        .id("ai-plan-steps")
                        .max_h_32()
                        .overflow_y_scroll()
                        .children(self.conversation.plan.iter().map(|step| {
                            let status = match step.status {
                                StepStatus::Pending => crate::tr!("待处理", "Pending"),
                                StepStatus::InProgress => crate::tr!("进行中", "In progress"),
                                StepStatus::Complete => crate::tr!("已完成", "Complete"),
                            };
                            div().text_xs().child(format!("{status} · {}", step.text))
                        })),
                )
            })
            .into_any_element()
    }
}
