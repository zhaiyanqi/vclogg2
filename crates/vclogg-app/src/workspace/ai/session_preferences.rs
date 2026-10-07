//! Preference writes share the session's serial persistence boundary with runs.
use super::*;

impl ConversationSession {
    pub(super) fn select_model(&mut self, id: String, cx: &mut Context<Self>) {
        self.conversation.provider_id = Some(id);
        self.preferences_dirty = true;
        self.persist_preferences(cx);
        cx.notify();
    }

    pub(super) fn persist_preferences(&mut self, cx: &mut Context<Self>) {
        if !self.preferences_dirty || self.is_running() {
            return;
        }
        let Some(store) = self.store.clone() else {
            return;
        };
        let record = match self.record() {
            Ok(record) => record,
            Err(error) => {
                self.error = error.to_string();
                return;
            }
        };
        self.preferences_dirty = false;
        self.busy = true;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { store.save_ai_conversation(&record) })
                .await;
            _ = this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(revision) => {
                        this.revision = revision;
                        this.update_history_entry();
                        this.persist_preferences(cx);
                    }
                    Err(error) => {
                        this.preferences_dirty = true;
                        this.error = error.to_string();
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}
