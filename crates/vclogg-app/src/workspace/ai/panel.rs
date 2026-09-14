use super::*;
use gpui::{ListAlignment, ListState};
use gpui_component::{
    input::{InputEvent, TextareaState},
    text::TextViewState,
};
use vclogg_ai::{AgentEvent, AiSettings, Conversation, RunHandle, RunStatus};
use vclogg_data::AiConversationRecord;

pub(in crate::workspace) struct AiPanel {
    pub(super) workspace: WeakEntity<Workspace>,
    pub(super) store: Option<Arc<StateStore>>,
    pub(super) settings: AiSettings,
    pub(super) settings_path: Option<PathBuf>,
    pub(super) conversation: Conversation,
    pub(super) revision: u64,
    pub(super) history: Vec<AiConversationRecord>,
    pub(super) more_history: bool,
    pub(super) input: Entity<TextareaState>,
    pub(super) list: ListState,
    pub(super) messages: Vec<Entity<TextViewState>>,
    pub(super) live_view: Entity<TextViewState>,
    pub(super) live: String,
    pub(super) reasoning: String,
    pub(super) progress: String,
    pub(super) expanded: BTreeSet<usize>,
    pub(super) pending_tool: Option<ToolCall>,
    pub(super) scope: Option<SharedScope>,
    pub(super) run: Option<RunHandle>,
    pub(super) editor: Option<super::settings::ConfigEditor>,
    pub(super) show_settings: bool,
    pub(super) busy: bool,
    pub(super) error: String,
    pub(super) generation: u64,
    task: Option<Task<()>>,
    live_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl AiPanel {
    pub(in crate::workspace) fn focus(&self, window: &mut Window, cx: &mut App) {
        self.input.focus_handle(cx).focus(window, cx);
    }
    pub(in crate::workspace) fn new(
        workspace: WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(3, 8)
                .submit_on_enter(true)
                .placeholder(crate::tr!(
                    "描述要分析的问题…",
                    "Describe what to investigate…"
                ))
        });
        let subscription = cx.subscribe_in(&input, window, |this, _, event, window, cx| {
            if matches!(event, InputEvent::PressEnter { shift: false, .. }) {
                this.send(false, window, cx);
            }
        });
        let mut this = Self {
            workspace,
            store: None,
            settings: AiSettings::default(),
            settings_path: crate::app_paths::application_data_dir()
                .map(|p| p.join("ai").join("settings.json")),
            conversation: Conversation::default(),
            revision: 0,
            history: Vec::new(),
            more_history: false,
            input,
            list: ListState::new(1, ListAlignment::Bottom, px(300.)),
            messages: Vec::new(),
            live_view: cx.new(|cx| TextViewState::markdown("", cx).selectable(true)),
            live: String::new(),
            reasoning: String::new(),
            progress: String::new(),
            expanded: BTreeSet::new(),
            pending_tool: None,
            scope: None,
            run: None,
            editor: None,
            show_settings: false,
            busy: true,
            error: String::new(),
            generation: 0,
            task: None,
            live_task: None,
            _subscriptions: vec![subscription],
        };
        let path = this.settings_path.clone();
        this.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let store = Arc::new(StateStore::open_default()?);
                    let settings = store.load_ai_settings(
                        path.as_deref()
                            .context("Application data directory unavailable")?,
                    );
                    let history = store.ai_conversations(0)?;
                    let current = history
                        .first()
                        .map(|r| store.load_ai_conversation(&r.id))
                        .transpose()?
                        .flatten();
                    Ok::<_, anyhow::Error>((store, settings, history, current))
                })
                .await;
            _ = this.update_in(cx, |this, _, cx| {
                this.busy = false;
                match result {
                    Ok((store, settings, history, current)) => {
                        this.store = Some(store);
                        match settings {
                            Ok(settings) => this.settings = settings,
                            Err(error) => this.error = error.to_string(),
                        }
                        this.more_history = history.len() == 100;
                        this.history = history;
                        if let Some(record) = current {
                            this.install_record(record, cx);
                        } else {
                            this.conversation.provider_id = this.settings.active_provider.clone();
                        }
                    }
                    Err(e) => this.error = e.to_string(),
                }
                cx.notify();
            });
        }));
        this
    }
    pub(super) fn install_record(&mut self, record: AiConversationRecord, cx: &mut Context<Self>) {
        match serde_json::from_str::<Conversation>(&record.payload) {
            Ok(mut conversation) => {
                conversation.recover();
                self.conversation = conversation;
                self.revision = record.revision;
                self.scope = None;
                self.live.clear();
                self.reasoning.clear();
                self.progress.clear();
                self.pending_tool = None;
                self.rebuild_messages(cx);
            }
            Err(_) => {
                self.error =
                    crate::tr!("会话数据损坏，无法读取", "Conversation data is corrupted").into()
            }
        }
    }
    fn rebuild_messages(&mut self, cx: &mut Context<Self>) {
        self.messages.clear();
        self.expanded.clear();
        for (ix, message) in self.conversation.messages.iter().enumerate() {
            self.messages.push(cx.new(|cx| {
                TextViewState::markdown(
                    &message_text(message, &self.conversation.messages[..ix]),
                    cx,
                )
                .selectable(true)
            }));
        }
        self.list.reset(self.messages.len() + 1);
    }
    pub(super) fn push_message(&mut self, message: AgentMessage, cx: &mut Context<Self>) {
        let follow = self.list.is_scrolled_to_end().unwrap_or(true);
        self.messages.push(cx.new(|cx| {
            TextViewState::markdown(&message_text(&message, &self.conversation.messages), cx)
                .selectable(true)
        }));
        self.conversation.messages.push(message);
        let count = self.messages.len() + 1;
        self.list.splice(count - 1..count - 1, 1);
        self.list.remeasure_items(count.saturating_sub(2)..count);
        if follow {
            self.list.scroll_to_reveal_item(count - 1);
        }
        cx.notify();
    }
    pub(super) fn record(&self) -> Result<AiConversationRecord> {
        Ok(AiConversationRecord {
            id: self.conversation.id.clone(),
            title: self.conversation.title.clone(),
            payload: serde_json::to_string(&self.conversation)?,
            revision: self.revision,
        })
    }

    pub(super) fn send(&mut self, continuation: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.run.is_some() {
            return;
        }
        let Some(config) = self
            .settings
            .providers
            .iter()
            .find(|p| Some(&p.id) == self.conversation.provider_id.as_ref())
            .cloned()
        else {
            self.error =
                crate::tr!("请先配置并选择模型", "Configure and select a model first").into();
            self.show_settings = true;
            cx.notify();
            return;
        };
        if let Err(e) = config.endpoint() {
            self.error = e.to_string();
            cx.notify();
            return;
        }
        let user_text = if continuation {
            crate::tr!("继续分析，先检查已有操作结果，避免重复修改。","Continue the analysis. Inspect previous operation results before making more changes.").to_owned()
        } else {
            self.input.read(cx).value().trim().to_owned()
        };
        if user_text.is_empty() {
            return;
        }
        if user_text.len() > 64 * 1024 {
            self.error = crate::tr!("输入不能超过 64 KiB", "Input cannot exceed 64 KiB").into();
            cx.notify();
            return;
        }
        let Some(store) = self.store.clone() else {
            self.error = crate::tr!("会话存储不可用", "Conversation storage unavailable").into();
            cx.notify();
            return;
        };
        let lease = match ConversationLease::acquire(&self.conversation.id) {
            Ok(lease) => lease,
            Err(e) => {
                self.error = e.to_string();
                cx.notify();
                return;
            }
        };
        let Ok(scope) = self
            .workspace
            .update(cx, |workspace, _| workspace.ai_scope())
        else {
            self.error = crate::tr!(
                "当前窗口不可用，请重新打开 AI 面板",
                "Window unavailable; reopen the AI panel"
            )
            .into();
            cx.notify();
            return;
        };
        self.scope = Some(scope.clone());
        self.generation += 1;
        let generation = self.generation;
        if self.conversation.title.is_empty() {
            self.conversation.title = config.redact(&user_text).chars().take(40).collect();
        }
        self.error.clear();
        self.live.clear();
        self.reasoning.clear();
        self.progress = crate::tr!("正在准备会话", "Preparing conversation").into();
        self.live_view.update(cx, |v, cx| v.set_text("", cx));
        self.pending_tool = None;
        self.conversation.recover();
        self.rebuild_messages(cx);
        self.push_message(
            AgentMessage::User {
                text: config.redact(&user_text),
            },
            cx,
        );
        self.conversation.status = RunStatus::Running;
        self.conversation.notice.clear();
        self.busy = true;
        self.input
            .update(cx, |input, cx| input.set_value("", window, cx));
        let messages = self.conversation.messages.clone();
        let skills = self
            .settings
            .skills
            .iter()
            .filter(|s| s.enabled && self.conversation.skill_ids.contains(&s.id))
            .cloned()
            .collect::<Vec<_>>();
        let record = match self.record() {
            Ok(r) => r,
            Err(e) => {
                self.error = e.to_string();
                self.busy = false;
                return;
            }
        };
        let workspace = self.workspace.clone();
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let _lease = lease;
            let save_store = store.clone();
            let capture = scope.clone();
            let saved = cx.background_spawn(async move {
                let directory = capture.lock().map_err(|_| anyhow::anyhow!("Analysis unavailable"))?.directory.directory.clone();
                // A directory is an optional capability, not a prerequisite for chatting
                // or inspecting the captured open documents. Never fall back to another path.
                let canonical = directory.as_ref().and_then(|path| path.canonicalize().ok()).filter(|path| path.is_dir());
                let directory_unavailable = directory.is_some() && canonical.is_none();
                capture.lock().map_err(|_| anyhow::anyhow!("Analysis unavailable"))?.directory.directory = canonical;
                save_store.save_ai_conversation(&record).map(|revision| (revision, directory_unavailable))
            }).await;
            let (revision, directory_unavailable) = match saved {
                Ok(saved) => saved,
                Err(error) => {
                    _ = this.update(cx, |this, cx| {
                        this.busy = false;
                        this.conversation.status = RunStatus::Failed;
                        this.error = error.to_string();
                        cx.notify();
                    });
                    return;
                }
            };
            if scope.lock().is_ok_and(|state| state.cancellation.is_cancelled()) {
                let record = this.update(cx, |this, cx| {
                    this.revision = revision;
                    this.conversation.status = RunStatus::Interrupted;
                    this.progress.clear();
                    this.error = crate::tr!("分析已停止", "Analysis stopped").into();
                    this.conversation.notice = this.error.clone();
                    cx.notify();
                    this.record()
                });
                if let Ok(Ok(record)) = record {
                    let saved = cx.background_spawn(async move { store.save_ai_conversation(&record) }).await;
                    _ = this.update(cx, |this, cx| {
                        match saved {
                            Ok(revision) => { this.revision = revision; this.update_history_entry(); }
                            Err(error) => this.error = error.to_string(),
                        }
                        this.busy = false;
                        cx.notify();
                    });
                }
                return;
            }
            let run = vclogg_ai::start_run(config, messages, skills,
                "Access only the current window's captured files and selected directory. Call get_context and list_logs first. All line numbers are 1-based; references are run-scoped.".into(), false);
            let events = run.events.clone();
            let replies = run.replies.clone();
            let cancellation = run.cancellation.clone();
            if this.update(cx, |this, cx| {
                this.revision = revision;
                this.busy = false;
                if directory_unavailable {
                    this.conversation.notice = crate::tr!(
                        "已选搜索目录不可用，本轮仅可访问已打开的日志。需要搜索目录时，请重新选择目录后发送。",
                        "The selected search directory is unavailable. This run can access open logs only. Select a directory again before sending to enable directory search."
                    ).into();
                }
                this.run = Some(run);
                cx.notify();
            }).is_err() { return; }
            while let Ok(event) = events.recv().await {
                if let AgentEvent::ToolStarted(call) = &event {
                    if cancellation.is_cancelled() { break; }
                    let call = call.clone();
                    _ = this.update(cx, |this, cx| {
                        this.progress = format!("{}: {}", crate::tr!("执行工具", "Running tool"), call.name);
                        this.pending_tool = Some(call.clone());
                        cx.notify();
                    });
                    let prepared = workspace.update(cx, |workspace, cx| workspace.ai_prepare(scope.clone(), &call, cx));
                    let result = match prepared {
                        Ok(Ok(work)) => {
                            let evidence = cx.background_spawn(async move { work() }).await;
                            if cancellation.is_cancelled() { break; }
                            match evidence {
                                Ok(value) => match workspace.update_in(cx, |workspace, window, cx| workspace.ai_commit(&scope, &call, value, window, cx)) {
                                    Ok(Ok(value)) => ToolResult::ok(value),
                                    Ok(Err(error)) => ToolResult::error(error.to_string()),
                                    Err(_) => ToolResult::error("Window closed"),
                                },
                                Err(error) => ToolResult::error(error.to_string()),
                            }
                        }
                        Ok(Err(error)) => ToolResult::error(error.to_string()),
                        Err(_) => break,
                    };
                    if replies.send((call.id, result)).await.is_err() { break; }
                    continue;
                }
                let finished = matches!(event, AgentEvent::Finished(..));
                let persist = matches!(event, AgentEvent::Assistant(_) | AgentEvent::ToolFinished(_) | AgentEvent::Finished(..));
                if this.update(cx, |this, cx| {
                    if this.generation == generation { this.receive_event(event, cx); }
                }).is_err() { break; }
                if persist {
                    let record = match this.update(cx, |this, _| this.record()) {
                        Ok(Ok(record)) => record,
                        _ => break,
                    };
                    let save_store = store.clone();
                    let saved = cx.background_spawn(async move { save_store.save_ai_conversation(&record) }).await;
                    match saved {
                        Ok(revision) => {
                            _ = this.update(cx, |this, cx| {
                                this.revision = revision;
                                this.update_history_entry();
                                cx.notify();
                            });
                        }
                        Err(error) => {
                            cancellation.cancel();
                            _ = this.update(cx, |this, cx| {
                                this.error = error.to_string();
                                this.busy = true;
                                this.run = None;
                                this.conversation.status = RunStatus::Failed;
                                cx.notify();
                            });
                            break;
                        }
                    }
                }
                if finished {
                    _ = this.update(cx, |this, cx| { this.busy = false; cx.notify(); });
                    return;
                }
            }
            cancellation.cancel();
            _ = workspace.update(cx, |workspace, cx| workspace.cancel_ai_searches(&scope, cx));
            _ = this.update(cx, |this, cx| {
                this.busy = true;
                this.run = None;
                this.pending_tool = None;
                this.flush_partial(cx);
                this.conversation.recover();
                this.rebuild_messages(cx);
                cx.notify();
            });
            if let Ok(Ok(record)) = this.update(cx, |this, _| this.record()) {
                let saved = cx.background_spawn(async move { store.save_ai_conversation(&record) }).await;
                _ = this.update(cx, |this, cx| {
                    match saved {
                        Ok(revision) => { this.revision = revision; this.update_history_entry(); }
                        Err(error) => this.error = error.to_string(),
                    }
                    this.busy = false;
                    cx.notify();
                });
            }
        }));
        cx.notify();
    }
    fn flush_partial(&mut self, cx: &mut Context<Self>) {
        self.live_task = None;
        if !self.live.is_empty() || !self.reasoning.is_empty() {
            let text = std::mem::take(&mut self.live);
            let reasoning = std::mem::take(&mut self.reasoning);
            self.push_message(
                AgentMessage::Assistant {
                    text,
                    reasoning,
                    thinking: Vec::new(),
                    calls: Vec::new(),
                },
                cx,
            );
        }
        self.live_view.update(cx, |view, cx| view.set_text("", cx));
    }
    fn receive_event(&mut self, event: AgentEvent, cx: &mut Context<Self>) {
        match event {
            AgentEvent::RequestStarted(request) => {
                self.progress = format!(
                    "{} ({request}/{})",
                    crate::tr!("等待模型响应", "Waiting for model"),
                    vclogg_ai::MAX_REQUESTS
                );
            }
            AgentEvent::ResponseStarted => {
                self.progress =
                    crate::tr!("已连接，等待模型输出", "Connected; waiting for output").into()
            }
            AgentEvent::PreparingTools => {
                self.progress = crate::tr!("正在接收工具参数", "Receiving tool arguments").into()
            }
            event @ (AgentEvent::Text(_) | AgentEvent::Thinking(_)) => {
                match event {
                    AgentEvent::Text(text) => {
                        self.live.push_str(&text);
                        self.progress = crate::tr!("正在生成回复", "Generating reply").into();
                    }
                    AgentEvent::Thinking(text) => {
                        self.reasoning.push_str(&text);
                        self.progress = crate::tr!("模型正在思考", "Model is thinking").into();
                    }
                    _ => unreachable!(),
                }
                if self.live_task.is_none() {
                    self.live_task = Some(cx.spawn(async move |this, cx| {
                        cx.background_executor()
                            .timer(std::time::Duration::from_millis(40))
                            .await;
                        _ = this.update(cx, |this, cx| {
                            let follow = this.list.is_scrolled_to_end().unwrap_or(true);
                            this.live_view.update(cx, |view, cx| {
                                view.set_text(&assistant_text(&this.live, &this.reasoning), cx)
                            });
                            this.list
                                .remeasure_items(this.messages.len()..this.messages.len() + 1);
                            if follow {
                                this.list.scroll_to_reveal_item(this.messages.len());
                            }
                            this.live_task = None;
                            cx.notify();
                        });
                    }));
                }
            }
            AgentEvent::Assistant(message) => {
                self.live_task = None;
                self.live.clear();
                self.reasoning.clear();
                self.live_view.update(cx, |view, cx| view.set_text("", cx));
                self.push_message(message, cx);
            }
            AgentEvent::ToolFinished(message) => {
                self.pending_tool = None;
                self.push_message(message, cx);
            }
            AgentEvent::ContextTrimmed => {
                self.conversation.notice = crate::tr!(
                    "较早轮次未发送给模型，完整历史仍保存在本地",
                    "Earlier turns were omitted from model context; full history remains local"
                )
                .into()
            }
            AgentEvent::Finished(status, error) => {
                self.conversation.status = status;
                self.progress.clear();
                if !error.is_empty() {
                    self.conversation.notice = error.clone();
                }
                self.error = error;
                self.pending_tool = None;
                self.flush_partial(cx);
                self.busy = true; // Do not switch conversations until the final record is durable.
                self.run = None;
            }
            AgentEvent::ToolStarted(_) => {}
        }
        cx.notify();
    }
    pub(super) fn update_history_entry(&mut self) {
        self.history.retain(|r| r.id != self.conversation.id);
        self.history.insert(
            0,
            AiConversationRecord {
                id: self.conversation.id.clone(),
                title: self.conversation.title.clone(),
                payload: String::new(),
                revision: self.revision,
            },
        );
    }
    pub(super) fn stop(&mut self, cx: &mut Context<Self>) {
        self.progress = crate::tr!("正在停止", "Stopping").into();
        if let Some(run) = &self.run {
            run.cancellation.cancel();
        }
        if let Some(scope) = &self.scope
            && let Ok(scope) = scope.lock()
        {
            scope.cancellation.cancel();
        }
        if let Some(scope) = &self.scope {
            _ = self
                .workspace
                .update(cx, |workspace, cx| workspace.cancel_ai_searches(scope, cx));
        }
        cx.notify();
    }
    pub(super) fn new_conversation(&mut self, cx: &mut Context<Self>) {
        if self.run.is_some() || self.busy {
            return;
        }
        self.conversation = Conversation {
            provider_id: self
                .conversation
                .provider_id
                .clone()
                .or(self.settings.active_provider.clone()),
            skill_ids: self.conversation.skill_ids.clone(),
            ..Default::default()
        };
        self.revision = 0;
        self.scope = None;
        self.error.clear();
        self.live.clear();
        self.reasoning.clear();
        self.progress.clear();
        self.rebuild_messages(cx);
        cx.notify();
    }
    pub(super) fn load_conversation(
        &mut self,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.run.is_some() || self.busy {
            return;
        }
        let Some(store) = self.store.clone() else {
            return;
        };
        self.busy = true;
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move { store.load_ai_conversation(&id) })
                .await;
            _ = this.update_in(cx, |this, _, cx| {
                this.busy = false;
                match result {
                    Ok(Some(record)) => this.install_record(record, cx),
                    Ok(None) => {
                        this.error = crate::tr!("会话已删除", "Conversation deleted").into()
                    }
                    Err(e) => this.error = e.to_string(),
                }
                cx.notify();
            });
        }));
    }
    pub(super) fn delete_conversation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if leases()
            .lock()
            .is_ok_and(|l| l.contains(&self.conversation.id))
        {
            self.error = crate::tr!(
                "会话正在其他窗口运行",
                "Conversation is running in another window"
            )
            .into();
            cx.notify();
            return;
        }
        if self.run.is_some() || self.busy {
            return;
        }
        let Some(store) = self.store.clone() else {
            return;
        };
        if self.revision == 0 {
            self.new_conversation(cx);
            return;
        }
        let id = self.conversation.id.clone();
        let revision = self.revision;
        self.busy = true;
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let saved_id = id.clone();
            let result = cx
                .background_spawn(async move { store.delete_ai_conversation(&saved_id, revision) })
                .await;
            _ = this.update_in(cx, |this, _, cx| {
                this.busy = false;
                match result {
                    Ok(()) => {
                        this.history.retain(|r| r.id != id);
                        this.new_conversation(cx);
                    }
                    Err(e) => this.error = e.to_string(),
                }
                cx.notify();
            });
        }));
    }
    pub(super) fn more_conversations(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.run.is_some() {
            return;
        }
        let Some(store) = self.store.clone() else {
            return;
        };
        let offset = self.history.len();
        self.busy = true;
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move { store.ai_conversations(offset) })
                .await;
            _ = this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(rows) => {
                        this.more_history = rows.len() == 100;
                        for row in rows {
                            if !this.history.iter().any(|r| r.id == row.id) {
                                this.history.push(row);
                            }
                        }
                    }
                    Err(e) => this.error = e.to_string(),
                }
                cx.notify();
            });
        }));
    }
}
impl Drop for AiPanel {
    fn drop(&mut self) {
        if let Some(run) = &self.run {
            run.cancellation.cancel();
        }
        if let Some(scope) = &self.scope
            && let Ok(scope) = scope.lock()
        {
            scope.cancellation.cancel();
        }
    }
}
fn assistant_text(text: &str, reasoning: &str) -> String {
    if reasoning.is_empty() {
        return safe_markdown(text);
    }
    format!(
        "### {}\n\n{}\n\n---\n\n{}",
        crate::tr!("模型思考", "Model thinking"),
        safe_markdown(reasoning),
        safe_markdown(text)
    )
}

pub(super) fn message_text(message: &AgentMessage, history: &[AgentMessage]) -> String {
    match message {
        AgentMessage::User { text } => safe_markdown(text),
        AgentMessage::Assistant {
            text, reasoning, ..
        } => assistant_text(text, reasoning),
        AgentMessage::Tool {
            call_id, result, ..
        } => {
            let call = history
                .iter()
                .rev()
                .filter_map(|message| match message {
                    AgentMessage::Assistant { calls, .. } => {
                        calls.iter().find(|call| &call.id == call_id)
                    }
                    _ => None,
                })
                .next();
            let details =
                json!({"arguments":call.map(|call| &call.arguments),"result":result.value});
            format!(
                "```json\n{}\n```",
                serde_json::to_string_pretty(&details).unwrap_or_default()
            )
        }
    }
}
// Remote images and raw HTML have no role in log analysis; do not let rendering fetch URLs.
pub(super) fn safe_markdown(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut fence: Option<(char, usize)> = None;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start_matches(' ');
        let indentation = line.len() - trimmed.len();
        let marker = trimmed.chars().next().filter(|c| *c == '`' || *c == '~');
        let width = marker.map_or(0, |marker| {
            trimmed.chars().take_while(|c| *c == marker).count()
        });
        if let Some((character, minimum)) = fence {
            output.push_str(line);
            if indentation <= 3
                && marker == Some(character)
                && width >= minimum
                && trimmed[width..].trim().is_empty()
            {
                fence = None;
            }
        } else if indentation <= 3
            && width >= 3
            && !(marker == Some('`') && trimmed[width..].contains('`'))
        {
            fence = marker.map(|marker| (marker, width));
            output.push_str(line);
        } else {
            output.push_str(
                &line
                    .replace("![", "[image: ")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;"),
            );
        }
    }
    output
}

#[cfg(test)]
mod tests;
