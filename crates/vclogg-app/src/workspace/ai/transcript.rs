use super::panel::safe_markdown;
use super::*;
use gpui_kit::component::text::TextViewState;

pub(super) struct AttachedContent {
    pub label: String,
    pub reference: LogReference,
    pub preview: String,
}

pub(super) struct AttachmentView {
    pub content: AttachedContent,
    pub view: Entity<TextViewState>,
}

/// The log attachment envelope is stored in the user message for both protocols.
/// Only peel off a validated suffix; ordinary Markdown and code remain untouched.
pub(super) fn user_content(text: &str) -> (&str, Vec<AttachedContent>) {
    let mut body = text;
    let mut attachments = Vec::new();
    while let Some(start) = body.rfind("\n\n[") {
        let block = &body[start + 2..];
        let Some((link, json)) = block.split_once("\n```json\n") else {
            break;
        };
        let Some(json) = json.strip_suffix("\n```") else {
            break;
        };
        let Some((label, url)) = link
            .strip_prefix('[')
            .and_then(|s| s.strip_suffix(')'))
            .and_then(|s| s.split_once("]("))
        else {
            break;
        };
        let Ok(value) = serde_json::from_str::<Value>(json) else {
            break;
        };
        let Some(reference) = value
            .get("reference")
            .and_then(|v| serde_json::from_value::<LogReference>(v.clone()).ok())
        else {
            break;
        };
        let preview = if let Some(preview) = value["log_data"].as_str() {
            preview
        } else if value["content_included"] == false {
            crate::tr!(
                "Agent 将按需读取此行",
                "The agent will read this line as needed"
            )
        } else {
            break;
        };
        if LogReference::from_url(url).as_ref() != Some(&reference) {
            break;
        }
        attachments.push(AttachedContent {
            label: label.to_owned(),
            reference,
            preview: preview.to_owned(),
        });
        body = &body[..start];
    }
    attachments.reverse();
    (body, attachments)
}

pub(super) fn row_starts(messages: &[AgentMessage], live: bool) -> Vec<usize> {
    let mut rows = Vec::new();
    for (ix, message) in messages.iter().enumerate() {
        if ix == 0
            || matches!(message, AgentMessage::User { .. })
            || matches!(messages[ix - 1], AgentMessage::User { .. })
        {
            rows.push(ix);
        }
    }
    if live && (messages.is_empty() || matches!(messages.last(), Some(AgentMessage::User { .. }))) {
        rows.push(messages.len());
    }
    rows
}

impl AiPanel {
    pub(super) fn copy_message(&self, ix: usize, cx: &mut Context<Self>) {
        let text = match self.conversation.messages.get(ix) {
            Some(AgentMessage::User { text }) => user_content(text).0,
            Some(AgentMessage::Assistant { text, .. }) => text,
            _ => return,
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text.to_owned()));
    }

    pub(super) fn edit_message(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings_busy(cx) || self.ui_busy {
            return;
        }
        let Some(AgentMessage::User { text }) = self.conversation.messages.get(ix) else {
            return;
        };
        let (body, attachments) = user_content(text);
        let body = body.to_owned();
        let mut logs = Vec::new();
        let expected = attachments.len();
        for attachment in attachments {
            let reference = attachment.reference;
            let snapshot = self
                .scope
                .iter()
                .chain(self.reference_scopes.iter())
                .filter_map(|scope| scope.lock().ok())
                .find_map(|scope| {
                    scope
                        .documents
                        .get(&reference.document_id)
                        .filter(|doc| doc.version == reference.version)
                        .cloned()
                });
            if let Some(document) = snapshot {
                logs.push(super::attachments::DraftLog {
                    document,
                    source_row: reference.line.saturating_sub(1),
                    preview: String::new(),
                });
            }
        }
        if logs.len() != expected {
            self.error = crate::tr!(
                "原消息的日志引用已不可用，请重新附加日志",
                "Original log references are unavailable; attach the logs again"
            )
            .into();
            cx.notify();
            return;
        }
        self.draft_logs = logs;
        self.editing_message = Some(ix);
        self.queued_prompts.clear();
        self.input
            .update(cx, |input, cx| input.set_value(body, window, cx));
        self.focus(window, cx);
        cx.notify();
    }

    pub(super) fn regenerate_message(
        &mut self,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.settings_busy(cx) || self.ui_busy {
            return;
        }
        let Some(user_ix) =
            (0..ix).rfind(|&i| matches!(self.conversation.messages[i], AgentMessage::User { .. }))
        else {
            return;
        };
        self.edit_message(user_ix, window, cx);
        if self.editing_message == Some(user_ix) {
            self.send(false, window, cx);
        }
    }

    pub(super) fn remeasure_message(&self, message_ix: usize, cx: &mut Context<Self>) {
        if let Some(row) = self
            .transcript_rows
            .partition_point(|start| *start <= message_ix)
            .checked_sub(1)
        {
            self.scroller
                .update(cx, |state, cx| state.remeasure_items(row..row + 1, cx));
        }
    }

    pub(super) fn sync_transcript(&mut self, reset: bool, cx: &mut Context<Self>) {
        let old = self.transcript_rows.len();
        self.transcript_rows = row_starts(&self.conversation.messages, self.live_row);
        let count = self.transcript_rows.len();
        self.scroller.update(cx, |state, cx| {
            if reset || count < old {
                state.reset(count, cx);
            } else if count > old {
                state.append(count - old, cx);
            }
            if count > 0 {
                state.remeasure_items(count - 1..count, cx);
            }
        });
    }

    pub(super) fn cache_attachments(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(AgentMessage::User { text }) = self.conversation.messages.get(ix) else {
            return;
        };
        let (_, attachments) = user_content(text);
        let views = attachments
            .into_iter()
            .map(|content| {
                let view = self.message_view(&safe_markdown(&content.preview), ix, cx);
                AttachmentView { content, view }
            })
            .collect::<Vec<_>>();
        if !views.is_empty() {
            self.message_attachments.insert(ix, views);
        }
    }

    pub(super) fn add_text_to_composer(
        &mut self,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if text.trim().is_empty() {
            return;
        }
        let current = self.input.read(cx).value();
        let value = if current.is_empty() {
            text.to_owned()
        } else {
            format!("{current}\n\n{text}")
        };
        if value.len() > 64 * 1024 {
            self.error = crate::tr!(
                "输入内容超过 64 KiB，请缩小选择范围",
                "Input exceeds 64 KiB; select less text"
            )
            .into();
        } else {
            self.input
                .update(cx, |input, cx| input.set_value(value, window, cx));
            self.focus(window, cx);
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn attachment_envelopes_preserve_body_and_order() {
        let reference = LogReference {
            document_id: 1,
            version: "v1".into(),
            line: 2,
        };
        let data = json!({"reference":reference,"log_data":"ERROR timeout"});
        let envelope = format!("\n\n[app.log:2]({})\n```json\n{data}\n```", reference.url());
        let text = format!("分析日志{envelope}{envelope}");
        let (body, attachments) = user_content(&text);
        assert_eq!(body, "分析日志");
        assert_eq!(attachments.len(), 2);
        assert_eq!(attachments[0].preview, "ERROR timeout");
        let invalid = text.replace("vclogg://log", "https://log");
        assert_eq!(user_content(&invalid).0, invalid);
        assert_eq!(
            user_content("ordinary ```json code").0,
            "ordinary ```json code"
        );
    }
    #[test]
    fn streamed_tool_cycles_share_one_reply_row() {
        let mut messages = vec![AgentMessage::User {
            text: "question".into(),
        }];
        assert_eq!(row_starts(&messages, true), vec![0, 1]);
        messages.push(AgentMessage::Assistant {
            text: "Inspecting".into(),
            reasoning: String::new(),
            thinking: vec![],
            calls: vec![],
        });
        messages.push(AgentMessage::Tool {
            call_id: "1".into(),
            name: "read_logs".into(),
            result: ToolResult::error("closed"),
        });
        assert_eq!(row_starts(&messages, true), vec![0, 1]);
        messages.push(AgentMessage::User {
            text: "next".into(),
        });
        assert_eq!(row_starts(&messages, true), vec![0, 1, 3, 4]);
    }
}
