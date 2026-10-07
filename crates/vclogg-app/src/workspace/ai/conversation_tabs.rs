//! The sidebar owns retained sessions; selecting or closing a tab never moves a run.
use super::*;
use vclogg_ai::RunStatus;
use vclogg_data::AiConversationRecord;

struct RetainedSession {
    session: Entity<ConversationSession>,
    _subscription: Subscription,
    _events: Subscription,
}

pub(in crate::workspace) struct AiPanel {
    workspace: WeakEntity<Workspace>,
    pub(in crate::workspace) active: Entity<ConversationSession>,
    sessions: Vec<RetainedSession>,
    pub(super) open_conversations: Vec<String>,
    pub(super) conversation_tab_scroll: ScrollHandle,
    pub(super) conversation_tab_focus: FocusHandle,
    history: Vec<AiConversationRecord>,
    pub(super) more_history: bool,
    history_offset: usize,
    history_loading: bool,
    loading: BTreeSet<String>,
    deleting: BTreeSet<String>,
    navigation_generation: u64,
    pub(super) error: String,
}

impl AiPanel {
    pub(in crate::workspace) fn new(
        workspace: WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let session = cx.new(|cx| ConversationSession::new(workspace.clone(), window, cx));
        let subscription = cx.observe(&session, |this, session, cx| {
            // Initialization may restore a different ID from storage. No navigation is
            // possible until the first session has loaded its settings and history.
            if this.open_conversations.is_empty() && !session.read(cx).busy {
                this.open_conversations
                    .push(session.read(cx).conversation.id.clone());
                this.history = session.read(cx).history.clone();
                this.history_offset = session.read(cx).history_offset;
                this.more_history = session.read(cx).more_history;
            }
            cx.notify();
        });
        let events = cx.subscribe_in(
            &session,
            window,
            |this, session, event: &panel::SessionEvent, window, cx| {
                let panel::SessionEvent::Fork(through) = event;
                let id = session.read(cx).conversation.id.clone();
                this.fork_conversation(&id, Some(*through), window, cx);
            },
        );
        Self {
            workspace,
            active: session.clone(),
            sessions: vec![RetainedSession {
                session,
                _subscription: subscription,
                _events: events,
            }],
            open_conversations: Vec::new(),
            conversation_tab_scroll: ScrollHandle::new(),
            conversation_tab_focus: cx.focus_handle(),
            history: Vec::new(),
            more_history: false,
            history_offset: 0,
            history_loading: false,
            loading: BTreeSet::new(),
            deleting: BTreeSet::new(),
            navigation_generation: 0,
            error: String::new(),
        }
    }

    pub(super) fn current_id(&self, cx: &App) -> String {
        self.active.read(cx).conversation.id.clone()
    }

    pub(super) fn session(&self, id: &str, cx: &App) -> Option<Entity<ConversationSession>> {
        self.sessions
            .iter()
            .find(|entry| entry.session.read(cx).conversation.id == id)
            .map(|entry| entry.session.clone())
    }

    pub(super) fn conversation_tabs_busy(&self, _cx: &App) -> bool {
        self.open_conversations.is_empty()
    }

    fn retain(
        &mut self,
        session: Entity<ConversationSession>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let subscription = cx.observe(&session, |_, _, cx| cx.notify());
        let events = cx.subscribe_in(
            &session,
            window,
            |this, session, event: &panel::SessionEvent, window, cx| {
                let panel::SessionEvent::Fork(through) = event;
                let id = session.read(cx).conversation.id.clone();
                this.fork_conversation(&id, Some(*through), window, cx);
            },
        );
        self.sessions.push(RetainedSession {
            session,
            _subscription: subscription,
            _events: events,
        });
    }

    fn activate(&mut self, session: Entity<ConversationSession>, cx: &mut Context<Self>) {
        self.navigation_generation += 1;
        self.active.update(cx, |s, _| s.active = false);
        session.update(cx, |s, cx| {
            s.active = true;
            s.unread = false;
            s.scroller.update(cx, |scroll, cx| scroll.remeasure(cx));
            cx.notify();
        });
        let id = session.read(cx).conversation.id.clone();
        if !self.open_conversations.contains(&id) {
            self.open_conversations.push(id.clone());
        }
        self.active = session;
        if let Some(ix) = self
            .open_conversations
            .iter()
            .position(|value| value == &id)
        {
            self.conversation_tab_scroll.scroll_to_item(ix);
        }
        cx.notify();
    }

    pub(super) fn new_conversation_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.conversation_tabs_busy(cx) {
            return;
        }
        let settings = self.active.read(cx).settings.clone();
        let store = self.active.read(cx).store.clone();
        let workspace = self.workspace.clone();
        let session = cx.new(|cx| {
            let mut session = ConversationSession::empty(workspace, window, cx);
            session.conversation.provider_id = settings.active_provider.clone();
            session.settings = settings;
            session.store = store;
            session.busy = false;
            session
        });
        self.retain(session.clone(), window, cx);
        self.activate(session, cx);
        self.focus(window, cx);
    }

    pub(super) fn load_conversation(
        &mut self,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.deleting.contains(&id) {
            return;
        }
        if let Some(session) = self.session(&id, cx) {
            self.activate(session, cx);
            return;
        }
        let Some(store) = self.active.read(cx).store.clone() else {
            return;
        };
        if !self.loading.insert(id.clone()) {
            return;
        }
        self.navigation_generation += 1;
        let generation = self.navigation_generation;
        cx.spawn_in(window, async move |this, cx| {
            let loaded_id = id.clone();
            let result = cx
                .background_spawn(async move { store.load_ai_conversation(&loaded_id) })
                .await;
            _ = this.update_in(cx, |this, window, cx| {
                this.loading.remove(&id);
                match result {
                    Ok(Some(record)) => {
                        let settings = this.active.read(cx).settings.clone();
                        let store = this.active.read(cx).store.clone();
                        let workspace = this.workspace.clone();
                        let session = cx.new(|cx| {
                            let mut session = ConversationSession::empty(workspace, window, cx);
                            session.settings = settings;
                            session.store = store;
                            session.busy = false;
                            session.active = false;
                            session.install_record(record, cx);
                            session
                        });
                        this.retain(session.clone(), window, cx);
                        if generation == this.navigation_generation {
                            this.activate(session, cx);
                        }
                    }
                    Ok(None) => {
                        this.history.retain(|row| row.id != id);
                        this.error = crate::tr!("会话已删除", "Conversation deleted").into();
                    }
                    Err(error) => this.error = error.to_string(),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn close_conversation_tab(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(ix) = self.open_conversations.iter().position(|value| value == id) else {
            return;
        };
        self.open_conversations.remove(ix);
        if self.current_id(cx) == id {
            if let Some(next) = self
                .open_conversations
                .get(ix.min(self.open_conversations.len().saturating_sub(1)))
                .cloned()
                .and_then(|id| self.session(&id, cx))
            {
                self.activate(next, cx);
            } else {
                // Keep the last tab open until its replacement is created.
                self.open_conversations.push(id.to_owned());
                self.new_conversation_tab(window, cx);
                self.open_conversations.retain(|value| value != id);
            }
        }
        self.conversation_tab_focus.focus(window, cx);
        cx.notify();
    }

    pub(super) fn close_other_conversation_tabs(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(session) = self.session(id, cx) {
            self.activate(session, cx);
            self.open_conversations.retain(|value| value == id);
            self.conversation_tab_focus.focus(window, cx);
            cx.notify();
        }
    }

    pub(super) fn is_running(&self, id: &str, cx: &App) -> bool {
        self.session(id, cx)
            .is_some_and(|session| session.read(cx).is_running())
    }

    pub(super) fn delete_conversation_tab(
        &mut self,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_running(&id, cx) || leases().lock().is_ok_and(|leases| leases.contains(&id)) {
            self.error = crate::tr!(
                "请先停止该会话，再删除",
                "Stop this conversation before deleting it"
            )
            .into();
            cx.notify();
            return;
        }
        if !self.deleting.insert(id.clone()) {
            return;
        }
        let Some(store) = self.active.read(cx).store.clone() else {
            self.deleting.remove(&id);
            return;
        };
        let revision = self
            .session(&id, cx)
            .map(|s| s.read(cx).revision)
            .or_else(|| self.history.iter().find(|r| r.id == id).map(|r| r.revision))
            .unwrap_or(0);
        if let Some(session) = self.session(&id, cx) {
            session.update(cx, |s, cx| {
                s.busy = true;
                cx.notify();
            });
        }
        cx.spawn_in(window, async move |this, cx| {
            let deleted_id = id.clone();
            let result = cx
                .background_spawn(async move {
                    if revision == 0 {
                        Ok(())
                    } else {
                        store.delete_ai_conversation(&deleted_id, revision)
                    }
                })
                .await;
            _ = this.update_in(cx, |this, window, cx| {
                this.deleting.remove(&id);
                if let Some(session) = this.session(&id, cx) {
                    session.update(cx, |s, cx| {
                        s.busy = false;
                        cx.notify();
                    });
                }
                match result {
                    Ok(()) => {
                        this.close_conversation_tab(&id, window, cx);
                        this.sessions
                            .retain(|s| s.session.read(cx).conversation.id != id);
                        this.history.retain(|r| r.id != id);
                    }
                    Err(error) => this.error = error.to_string(),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn refresh_conversation_history(
        &mut self,
        more: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.history_loading {
            return;
        }
        let Some(store) = self.active.read(cx).store.clone() else {
            return;
        };
        self.history_loading = true;
        let offset = if more { self.history_offset } else { 0 };
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move { store.ai_conversations(offset) })
                .await;
            _ = this.update(cx, |this, cx| {
                this.history_loading = false;
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
        })
        .detach();
    }

    pub(super) fn conversation_tab_title(&self, id: &str, cx: &App) -> String {
        let Some(session) = self.session(id, cx) else {
            return crate::tr!("新会话", "New conversation").into();
        };
        let session = session.read(cx);
        let title = if session.conversation.title.is_empty() {
            crate::tr!("新会话", "New conversation")
        } else {
            &session.conversation.title
        };
        let status = if session.pending_question.is_some() {
            crate::tr!("待回答", "Needs input")
        } else if session.waiting_for_slot {
            crate::tr!("排队中", "Queued")
        } else if session.is_running() {
            crate::tr!("运行中", "Running")
        } else if session.conversation.status == RunStatus::Failed {
            crate::tr!("失败", "Failed")
        } else if session.unread {
            crate::tr!("未读", "Unread")
        } else {
            ""
        };
        if status.is_empty() {
            title.to_owned()
        } else {
            format!("{title} · {status}")
        }
    }

    pub(super) fn conversation_history_items(&self, cx: &App) -> Vec<AiConversationRecord> {
        let mut rows = self.history.clone();
        for entry in &self.sessions {
            let session = entry.session.read(cx);
            rows.retain(|r| r.id != session.conversation.id);
            rows.insert(
                0,
                AiConversationRecord {
                    id: session.conversation.id.clone(),
                    title: self.conversation_tab_title(&session.conversation.id, cx),
                    payload: String::new(),
                    revision: session.revision,
                },
            );
        }
        rows
    }

    pub(in crate::workspace) fn contains_transcript(
        &self,
        position: Point<Pixels>,
        cx: &App,
    ) -> bool {
        self.active.read(cx).contains_transcript(position)
    }
    pub(in crate::workspace) fn focus(&self, window: &mut Window, cx: &mut App) {
        self.active
            .update(cx, |session, cx| session.focus(window, cx));
    }
    pub(in crate::workspace) fn open_settings(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.active.update(cx, |s, cx| s.open_settings(window, cx));
    }
    pub(super) fn attach_logs(
        &mut self,
        targets: Vec<attachments::DraftLog>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.active
            .update(cx, |s, cx| s.attach_logs(targets, window, cx));
    }
}

impl Render for AiPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .min_h_0()
            .min_w_0()
            .child(self.render_conversation_tabs(window, cx))
            .when(!self.error.is_empty(), |view| {
                view.child(div().px_3().text_sm().child(self.error.clone()))
            })
            .child(self.active.clone())
    }
}
