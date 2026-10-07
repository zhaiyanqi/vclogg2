//! A submitted follow-up keeps its model and source choices even if the draft changes.
use super::*;
use vclogg_ai::AiSettings;

#[derive(Clone)]
pub(super) struct RunPreferences {
    settings: AiSettings,
    mode: vclogg_ai::AgentMode,
    provider_id: Option<String>,
    selected_log_ids: Option<BTreeSet<u64>>,
    selected_project_directories: Option<BTreeSet<PathBuf>>,
    include_search_directory: bool,
}
impl RunPreferences {
    pub(super) fn capture(session: &ConversationSession) -> Self {
        Self {
            settings: session.settings.clone(),
            mode: session.conversation.mode,
            provider_id: session.conversation.provider_id.clone(),
            selected_log_ids: session.selected_log_ids.clone(),
            selected_project_directories: session.selected_project_directories.clone(),
            include_search_directory: session.include_search_directory,
        }
    }
    pub(super) fn model(&self) -> &str {
        self.settings
            .providers
            .iter()
            .find(|p| Some(&p.id) == self.provider_id.as_ref())
            .map_or("", |p| p.model.as_str())
    }
    // send() captures all execution inputs synchronously before spawning the run.
    // Swap back immediately afterwards so the next draft retains the user's choices.
    pub(super) fn swap(&mut self, session: &mut ConversationSession) {
        std::mem::swap(&mut self.mode, &mut session.conversation.mode);
        std::mem::swap(&mut self.settings, &mut session.settings);
        std::mem::swap(&mut self.provider_id, &mut session.conversation.provider_id);
        std::mem::swap(&mut self.selected_log_ids, &mut session.selected_log_ids);
        std::mem::swap(
            &mut self.selected_project_directories,
            &mut session.selected_project_directories,
        );
        std::mem::swap(
            &mut self.include_search_directory,
            &mut session.include_search_directory,
        );
    }
}

impl ConversationSession {
    pub(super) fn move_queued_up(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(ix) = self.queued_prompts.iter().position(|p| p.id == id)
            && ix > 0
        {
            self.queued_prompts.swap(ix, ix - 1);
            cx.notify();
        }
    }
    pub(super) fn edit_queued(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.queue_editing.is_some() {
            return;
        }
        let Some(prompt) = self.queued_prompts.iter().find(|p| p.id == id) else {
            return;
        };
        let text = prompt.text.clone();
        let input = cx.new(|cx| {
            gpui_kit::component::input::TextareaState::new(window, cx)
                .default_value(text)
                .auto_grow(3, 10)
        });
        self.queue_editing = Some(id.clone());
        let session = cx.entity();
        window.open_dialog(cx, move |dialog, _, cx| {
            let submit = input.clone();
            let owner = session.clone();
            let close = session.clone();
            let id = id.clone();
            dialog
                .title(crate::tr!("编辑待发送消息", "Edit queued message"))
                .child(gpui_kit::component::input::Textarea::new(&input))
                .footer(
                    gpui_kit::component::dialog::DialogFooter::new()
                        .child(crate::dialog_focus::dialog_cancel_action(
                            "queue-edit-cancel",
                            Button::new("queue-edit-cancel-button")
                                .label(crate::tr!("取消", "Cancel")),
                            cx,
                        ))
                        .child(crate::dialog_focus::dialog_confirm_action(
                            "queue-edit-save",
                            Button::new("queue-edit-save-button")
                                .primary()
                                .label(crate::tr!("保存", "Save")),
                            cx,
                        )),
                )
                .on_ok(move |_, window, cx| {
                    let text = submit.read(cx).value().trim().to_owned();
                    if text.is_empty() || text.len() > 64 * 1024 {
                        window.notify_message(
                            crate::tr!(
                                "消息不能为空，且不能超过 64 KiB",
                                "Message must be nonempty and at most 64 KiB"
                            ),
                            cx,
                        );
                        return false;
                    }
                    owner.update(cx, |this, cx| {
                        if let Some(prompt) = this.queued_prompts.iter_mut().find(|p| p.id == id) {
                            prompt.text = text;
                        }
                        cx.notify();
                    });
                    true
                })
                .on_close(move |_, window, cx| {
                    close.update(cx, |this, cx| {
                        this.queue_editing = None;
                        if !this.is_running()
                            && this.conversation.status == vclogg_ai::RunStatus::Complete
                        {
                            this.start_next_queued(window, cx);
                        }
                        cx.notify();
                    })
                })
        });
    }
}
