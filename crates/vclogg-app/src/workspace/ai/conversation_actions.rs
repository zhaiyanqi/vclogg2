use super::*;
use gpui_kit::component::{
    dialog::DialogFooter,
    input::{Input, InputState},
};

struct ConversationName {
    input: Entity<InputState>,
    invalid: bool,
}
impl ConversationName {
    fn new(title: &str, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            input: cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(title)
                    .placeholder(crate::tr!("会话名称", "Conversation name"))
            }),
            invalid: false,
        }
    }
    fn input(&self) -> Entity<InputState> {
        self.input.clone()
    }
    fn title(&self, cx: &App) -> Option<String> {
        let value = self.input.read(cx).value().trim().to_owned();
        (!value.is_empty()).then_some(value)
    }
    fn show_validation_error(&mut self, cx: &mut Context<Self>) {
        self.invalid = true;
        cx.notify();
    }
}
impl Render for ConversationName {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .w_96()
            .max_w_full()
            .gap_2()
            .child(Input::new(&self.input))
            .when(self.invalid, |view| {
                view.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().danger)
                        .child(crate::tr!(
                            "会话名称不能为空",
                            "Conversation name cannot be empty"
                        )),
                )
            })
    }
}

impl AiPanel {
    pub(super) fn rename_conversation(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session(id, cx) else {
            return;
        };
        let title = session.read(cx).conversation.title.clone();
        let rename = cx.new(|cx| ConversationName::new(&title, window, cx));
        let input = rename.read(cx).input();
        window.defer(cx, move |window, cx| {
            input.focus_handle(cx).focus(window, cx);
            input.update(cx, |input, cx| input.select_all(window, cx));
        });
        window.open_dialog(cx, move |dialog, _, cx| {
            let submit = rename.clone();
            let session = session.clone();
            dialog
                .title(crate::tr!("重命名会话", "Rename conversation"))
                .child(rename.clone())
                .footer(
                    DialogFooter::new()
                        .child(crate::dialog_focus::dialog_cancel_action(
                            "ai-rename-cancel",
                            Button::new("ai-rename-cancel-button")
                                .label(crate::tr!("取消", "Cancel")),
                            cx,
                        ))
                        .child(crate::dialog_focus::dialog_confirm_action(
                            "ai-rename-save",
                            Button::new("ai-rename-save-button")
                                .primary()
                                .label(crate::tr!("保存", "Save")),
                            cx,
                        )),
                )
                .on_ok(move |_, _, cx| {
                    let Some(title) = submit.read(cx).title(cx) else {
                        submit.update(cx, |rename, cx| rename.show_validation_error(cx));
                        return false;
                    };
                    session.update(cx, |session, cx| {
                        session.conversation.title = title;
                        session.preferences_dirty = true;
                        session.persist_preferences(cx);
                        cx.notify();
                    });
                    true
                })
        });
    }

    pub(super) fn fork_conversation(
        &mut self,
        id: &str,
        through: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(source) = self.session(id, cx) else {
            return;
        };
        let mut conversation = source.read(cx).conversation_with_log_sources();
        if let Some(through) = through {
            conversation.messages.truncate(through);
            conversation.run_models.retain(|start, _| *start < through);
            conversation.context_summary.clear();
            conversation.summarized_messages = 0;
        }
        conversation.recover();
        conversation.status = vclogg_ai::RunStatus::Idle;
        conversation.notice.clear();
        conversation.context_usage = None;
        conversation.title = format!("{} · {}", conversation.title, crate::tr!("分支", "Branch"));
        self.new_conversation_tab(window, cx);
        self.active.update(cx, |session, cx| {
            conversation.id = session.conversation.id.clone();
            session.conversation = conversation;
            session.rebuild_messages(cx);
            session.preferences_dirty = true;
            session.persist_preferences(cx);
            cx.notify();
        });
    }

    pub(super) fn stop_conversation(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(session) = self.session(id, cx) {
            session.update(cx, |session, cx| session.stop(cx));
        }
    }

    pub(super) fn export_conversation(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.session(id, cx) else {
            return;
        };
        let conversation = session.read(cx).conversation_with_log_sources();
        let text = transcript_markdown(&conversation);
        let prompt = cx.prompt_for_new_path(Path::new("."), Some("conversation.md"));
        cx.spawn_in(window, async move |this, cx| {
            let result = match prompt.await {
                Ok(Ok(Some(path))) => {
                    cx.background_spawn(async move {
                        std::fs::write(&path, text).map_err(anyhow::Error::from)
                    })
                    .await
                }
                Ok(Ok(None)) => return,
                Ok(Err(error)) => Err(error),
                Err(error) => Err(anyhow::anyhow!(error)),
            };
            _ = this.update(cx, |this, cx| {
                if let Err(error) = result {
                    this.error = error.to_string();
                }
                cx.notify();
            });
        })
        .detach();
    }
}

fn transcript_markdown(conversation: &vclogg_ai::Conversation) -> String {
    let mut text = format!("# {}\n\n", conversation.title);
    for (ix, message) in conversation.messages.iter().enumerate() {
        let role = match message {
            AgentMessage::User { .. } => "User",
            AgentMessage::Assistant { .. } => "Assistant",
            AgentMessage::Tool { .. } => "Tool",
        };
        text.push_str(&format!("## {role}\n\n"));
        if let Some(model) = conversation.run_models.get(&ix) {
            text.push_str(&format!("Model: {model}\n\n"));
        }
        text.push_str(&panel::message_text(message, &conversation.messages[..ix]));
        text.push_str("\n\n");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exported_transcript_keeps_roles_and_actual_models() {
        let mut conversation = vclogg_ai::Conversation {
            title: "Investigation".into(),
            ..Default::default()
        };
        conversation.messages.push(AgentMessage::User {
            text: "Find the error".into(),
        });
        conversation.run_models.insert(0, "model-a".into());
        let markdown = transcript_markdown(&conversation);
        assert!(markdown.contains("# Investigation\n"));
        assert!(markdown.contains("## User\n"));
        assert!(markdown.contains("Model: model-a\n"));
        assert!(markdown.contains("Find the error"));
    }
}
