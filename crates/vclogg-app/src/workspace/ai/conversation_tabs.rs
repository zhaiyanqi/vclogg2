use super::panel::QueuedPrompt;
use super::transcript_scroll::TranscriptScroll;
use super::*;
use std::collections::VecDeque;
use vclogg_ai::Conversation;
use vclogg_data::AiConversationRecord;

/// Inactive tabs retain their draft and log capabilities independently of the
/// currently rendered transcript. Closing a tab does not delete its record.
pub(super) struct ConversationTab {
    conversation: Conversation,
    revision: u64,
    draft: String,
    logs: Vec<attachments::DraftLog>,
    queued_prompts: VecDeque<QueuedPrompt>,
    editing_message: Option<usize>,
    selected_log_ids: Option<BTreeSet<u64>>,
    selected_workspace_directories: Option<BTreeSet<PathBuf>>,
    include_search_directory: bool,
    scope: Option<SharedScope>,
    reference_scopes: Vec<SharedScope>,
    error: String,
    expanded: BTreeSet<usize>,
    thinking_expanded: BTreeSet<usize>,
    scroller: Entity<TranscriptScroll>,
    transcript_rows: Vec<usize>,
}

impl AiPanel {
    pub(super) fn conversation_tabs_busy(&self, cx: &App) -> bool {
        self.settings_busy(cx) || self.ui_busy || self.attachments_loading
    }

    fn retain_conversation_tab(&mut self, cx: &App) {
        self.inactive_conversations.insert(
            self.conversation.id.clone(),
            ConversationTab {
                conversation: self.conversation.clone(),
                revision: self.revision,
                draft: self.input.read(cx).value().to_string(),
                logs: std::mem::take(&mut self.draft_logs),
                queued_prompts: std::mem::take(&mut self.queued_prompts),
                editing_message: self.editing_message.take(),
                selected_log_ids: self.selected_log_ids.take(),
                selected_workspace_directories: self.selected_workspace_directories.take(),
                include_search_directory: self.include_search_directory,
                scope: self.scope.take(),
                reference_scopes: std::mem::take(&mut self.reference_scopes),
                error: std::mem::take(&mut self.error),
                expanded: std::mem::take(&mut self.expanded),
                thinking_expanded: std::mem::take(&mut self.thinking_expanded),
                scroller: self.scroller.clone(),
                transcript_rows: self.transcript_rows.clone(),
            },
        );
    }

    fn activate_retained_conversation(
        &mut self,
        tab: ConversationTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.generation += 1;
        self.conversation = tab.conversation;
        self.revision = tab.revision;
        self.draft_logs = tab.logs;
        self.queued_prompts = tab.queued_prompts;
        self.editing_message = tab.editing_message;
        self.selected_log_ids = tab.selected_log_ids;
        self.selected_workspace_directories = tab.selected_workspace_directories;
        self.include_search_directory = tab.include_search_directory;
        self.resume_queue_after_stop = false;
        self.scope = tab.scope;
        self.reference_scopes = tab.reference_scopes;
        self.error = tab.error;
        self.attachment_task = None;
        self.attachments_loading = false;
        self.live.clear();
        self.reasoning.clear();
        self.progress.clear();
        self.pending_tool = None;
        self.pending_question = None;
        self.input
            .update(cx, |input, cx| input.set_value(tab.draft, window, cx));
        self.scroller = tab.scroller;
        self.scroll_subscription = cx.observe(&self.scroller, |_, _, cx| cx.notify());
        self.transcript_rows = tab.transcript_rows;
        self.rebuild_tab_messages(false, cx);
        self.expanded = tab.expanded;
        self.thinking_expanded = tab.thinking_expanded;
        self.scroller.update(cx, |state, cx| state.remeasure(cx));
        if !self.open_conversations.contains(&self.conversation.id) {
            self.open_conversations.push(self.conversation.id.clone());
        }
        self.reveal_conversation_tab();
        cx.notify();
    }

    fn reset_conversation_scroller(&mut self, cx: &mut Context<Self>) {
        self.scroller = cx.new(|cx| TranscriptScroll::new(0, cx));
        self.scroll_subscription = cx.observe(&self.scroller, |_, _, cx| cx.notify());
    }

    fn reveal_conversation_tab(&self) {
        if let Some(index) = self
            .open_conversations
            .iter()
            .position(|id| id == &self.conversation.id)
        {
            self.conversation_tab_scroll.scroll_to_item(index);
        }
    }

    pub(super) fn new_conversation_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.conversation_tabs_busy(cx) {
            return;
        }
        self.retain_conversation_tab(cx);
        self.reset_conversation_scroller(cx);
        self.new_conversation(cx);
        self.open_conversations.push(self.conversation.id.clone());
        self.input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.reveal_conversation_tab();
        self.focus(window, cx);
    }

    pub(super) fn load_conversation(
        &mut self,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if id == self.conversation.id || self.conversation_tabs_busy(cx) {
            return;
        }
        if let Some(tab) = self.inactive_conversations.remove(&id) {
            self.retain_conversation_tab(cx);
            self.activate_retained_conversation(tab, window, cx);
            return;
        }
        let Some(store) = self.store.clone() else {
            return;
        };
        self.busy = true;
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let loaded_id = id.clone();
            let result = cx
                .background_spawn(async move {
                    store
                        .load_ai_conversation(&loaded_id)?
                        .map(|record| {
                            let mut conversation: Conversation =
                                serde_json::from_str(&record.payload)?;
                            conversation.recover();
                            Ok::<_, anyhow::Error>((conversation, record.revision))
                        })
                        .transpose()
                })
                .await;
            _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(Some((conversation, revision))) => {
                        this.retain_conversation_tab(cx);
                        this.activate_retained_conversation(
                            ConversationTab {
                                conversation,
                                revision,
                                draft: String::new(),
                                logs: Vec::new(),
                                queued_prompts: VecDeque::new(),
                                editing_message: None,
                                selected_log_ids: None,
                                selected_workspace_directories: None,
                                include_search_directory: true,
                                scope: None,
                                reference_scopes: Vec::new(),
                                error: String::new(),
                                expanded: BTreeSet::new(),
                                thinking_expanded: BTreeSet::new(),
                                scroller: cx.new(|cx| TranscriptScroll::new(0, cx)),
                                transcript_rows: Vec::new(),
                            },
                            window,
                            cx,
                        );
                    }
                    Ok(None) => {
                        this.history.retain(|row| row.id != id);
                        this.error = crate::tr!("会话已删除", "Conversation deleted").into();
                    }
                    Err(error) => this.error = error.to_string(),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(super) fn close_conversation_tab(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.conversation_tabs_busy(cx) {
            return;
        }
        let Some(index) = self.open_conversations.iter().position(|tab| tab == id) else {
            return;
        };
        self.open_conversations.remove(index);
        if self.conversation.id == id {
            self.retain_conversation_tab(cx);
            let next = self
                .open_conversations
                .get(index.min(self.open_conversations.len().saturating_sub(1)))
                .cloned();
            if let Some(next) = next.and_then(|id| self.inactive_conversations.remove(&id)) {
                self.activate_retained_conversation(next, window, cx);
            } else {
                self.reset_conversation_scroller(cx);
                self.new_conversation(cx);
                self.open_conversations.push(self.conversation.id.clone());
                self.input
                    .update(cx, |input, cx| input.set_value("", window, cx));
            }
        }
        // An untouched empty tab has nothing to recover from history.
        if self.inactive_conversations.get(id).is_some_and(|tab| {
            tab.revision == 0
                && tab.conversation.messages.is_empty()
                && tab.draft.trim().is_empty()
                && tab.logs.is_empty()
                && tab.queued_prompts.is_empty()
                && tab.editing_message.is_none()
        }) {
            self.inactive_conversations.remove(id);
        }
        self.reveal_conversation_tab();
        self.conversation_tab_focus.focus(window, cx);
        cx.notify();
    }

    pub(super) fn close_other_conversation_tabs(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.conversation_tabs_busy(cx) || !self.open_conversations.iter().any(|tab| tab == id) {
            return;
        }
        for other in self.open_conversations.clone() {
            if other != id {
                self.close_conversation_tab(&other, window, cx);
            }
        }
    }

    pub(super) fn delete_conversation_tab(
        &mut self,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.conversation_tabs_busy(cx) {
            return;
        }
        if leases().lock().is_ok_and(|leases| leases.contains(&id)) {
            self.error = crate::tr!(
                "会话正在其他窗口运行",
                "Conversation is running in another window"
            )
            .into();
            cx.notify();
            return;
        }
        let revision = if self.conversation.id == id {
            Some(self.revision)
        } else {
            self.inactive_conversations.get(&id).map(|tab| tab.revision)
        };
        let Some(revision) = revision else {
            return;
        };
        if revision == 0 {
            self.close_conversation_tab(&id, window, cx);
            self.inactive_conversations.remove(&id);
            return;
        }
        let Some(store) = self.store.clone() else {
            return;
        };
        self.busy = true;
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let deleted_id = id.clone();
            let result = cx
                .background_spawn(
                    async move { store.delete_ai_conversation(&deleted_id, revision) },
                )
                .await;
            _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(()) => {
                        this.close_conversation_tab(&id, window, cx);
                        this.inactive_conversations.remove(&id);
                        this.history.retain(|row| row.id != id);
                        this.history_offset = this.history_offset.saturating_sub(1);
                    }
                    Err(error) => this.error = error.to_string(),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(super) fn refresh_conversation_history(
        &mut self,
        more: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.conversation_tabs_busy(cx) {
            return;
        }
        let Some(store) = self.store.clone() else {
            return;
        };
        let offset = if more { self.history_offset } else { 0 };
        self.busy = true;
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move { store.ai_conversations(offset) })
                .await;
            _ = this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(rows) => {
                        this.history_offset = offset + rows.len();
                        this.more_history = rows.len() == 100;
                        if !more {
                            this.history.clear();
                        }
                        for row in rows {
                            this.history.retain(|old| old.id != row.id);
                            this.history.push(row);
                        }
                    }
                    Err(error) => this.error = error.to_string(),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(super) fn conversation_tab_title(&self, id: &str) -> String {
        let conversation = if self.conversation.id == id {
            Some(&self.conversation)
        } else {
            self.inactive_conversations
                .get(id)
                .map(|tab| &tab.conversation)
        };
        conversation
            .map(|conversation| conversation.title.as_str())
            .filter(|title| !title.is_empty())
            .unwrap_or(crate::tr!("新会话", "New conversation"))
            .to_owned()
    }

    pub(super) fn conversation_history_items(&self) -> Vec<AiConversationRecord> {
        let mut rows = self.history.clone();
        for (id, tab) in &self.inactive_conversations {
            if tab.revision == 0
                && (!tab.draft.is_empty()
                    || !tab.logs.is_empty()
                    || !tab.conversation.messages.is_empty())
            {
                rows.push(AiConversationRecord {
                    id: id.clone(),
                    title: self.conversation_tab_title(id),
                    payload: String::new(),
                    revision: 0,
                });
            }
        }
        for id in &self.open_conversations {
            if !rows.iter().any(|row| &row.id == id) {
                rows.push(AiConversationRecord {
                    id: id.clone(),
                    title: self.conversation_tab_title(id),
                    payload: String::new(),
                    revision: 0,
                });
            }
        }
        rows
    }
}
