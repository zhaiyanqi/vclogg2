use super::transcript_scroll::TranscriptScroll;
use super::*;
use gpui_kit::component::{
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
    pub(super) history_offset: usize,
    pub(super) open_conversations: Vec<String>,
    pub(super) inactive_conversations: BTreeMap<String, super::conversation_tabs::ConversationTab>,
    pub(super) conversation_tab_scroll: ScrollHandle,
    pub(super) conversation_tab_focus: FocusHandle,
    pub(super) input: Entity<TextareaState>,
    pub(super) draft_logs: Vec<super::attachments::DraftLog>,
    pub(super) attachments_loading: bool,
    pub(super) attachment_task: Option<Task<()>>,
    pub(super) scroller: Entity<TranscriptScroll>,
    pub(super) scroll_subscription: Subscription,
    pub(super) live_row: bool,
    pub(super) transcript_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    message_subscriptions: Vec<Subscription>,
    pub(super) messages: Vec<Entity<TextViewState>>,
    pub(super) live_view: Entity<TextViewState>,
    pub(super) live_reasoning_view: Entity<TextViewState>,
    pub(super) reasoning_views: Vec<Option<Entity<TextViewState>>>,
    pub(super) thinking_expanded: BTreeSet<usize>,
    pub(super) transcript_rows: Vec<usize>,
    pub(super) message_attachments: BTreeMap<usize, Vec<super::transcript::AttachmentView>>,
    pub(super) live: String,
    pub(super) reasoning: String,
    pub(super) progress: String,
    pub(super) expanded: BTreeSet<usize>,
    pub(super) message_menu: Option<(Entity<PopupMenu>, gpui_kit::Point<gpui_kit::Pixels>)>,
    pub(super) message_menu_subscription: Option<Subscription>,
    pub(super) pending_tool: Option<ToolCall>,
    pub(super) scope: Option<SharedScope>,
    pub(super) reference_scopes: Vec<SharedScope>,
    pub(super) run: Option<RunHandle>,
    pub(super) editor: Option<super::settings::ConfigEditor>,
    pub(super) show_settings: bool,
    pub(super) show_context_usage: bool,
    pub(super) settings_generation: u64,
    pub(super) settings_tab: super::configuration::SettingsTab,
    pub(super) prompt_editor: Option<super::prompt_settings::PromptEditor>,
    pub(super) mcp_editor: Option<super::mcp_settings::McpEditor>,
    pub(super) mcp_testing: bool,
    pub(super) mcp_status: String,
    pub(super) mcp_test_cancel: Option<vclogg_ai::Cancellation>,
    pub(super) mcp_test_task: Option<Task<()>>,
    pub(super) memories: Vec<vclogg_data::AiMemoryRecord>,
    pub(super) memory_editor: Option<super::memory::MemoryEditor>,
    pub(super) memory_loading: bool,
    pub(super) memory_task: Option<Task<()>>,
    pub(super) busy: bool,
    pub(super) ui_busy: bool,
    pub(super) ui_task: Option<Task<()>>,
    pub(super) error: String,
    pub(super) generation: u64,
    pub(super) task: Option<Task<()>>,
    live_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl AiPanel {
    pub(in crate::workspace) fn contains_transcript(&self, position: Point<Pixels>) -> bool {
        !self.show_settings
            && self
                .transcript_bounds
                .get()
                .is_some_and(|bounds| bounds.contains(&position))
    }

    pub(in crate::workspace) fn focus(&self, window: &mut Window, cx: &mut App) {
        self.input.focus_handle(cx).focus(window, cx);
    }
    pub(in crate::workspace) fn new(
        workspace: WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        if !cx.has_global::<super::configuration::SharedAiSettings>() {
            cx.set_global(super::configuration::SharedAiSettings::default());
        }
        let settings_subscription =
            cx.observe_global::<super::configuration::SharedAiSettings>(|this, cx| {
                if let Some(settings) = cx
                    .global::<super::configuration::SharedAiSettings>()
                    .settings
                    .clone()
                {
                    this.settings = settings;
                }
                cx.notify();
            });
        if !cx.has_global::<super::memory::SharedAiMemory>() {
            cx.set_global(super::memory::SharedAiMemory);
        }
        let memory_subscription =
            cx.observe_global::<super::memory::SharedAiMemory>(|this, cx| this.load_memories(cx));
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
            } else if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        let scroller = cx.new(|cx| TranscriptScroll::new(0, cx));
        let scroll_subscription = cx.observe(&scroller, |_, _, cx| cx.notify());
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
            history_offset: 0,
            open_conversations: Vec::new(),
            inactive_conversations: BTreeMap::new(),
            conversation_tab_scroll: ScrollHandle::new(),
            conversation_tab_focus: cx.focus_handle(),
            input,
            draft_logs: Vec::new(),
            attachments_loading: false,
            attachment_task: None,
            scroller,
            scroll_subscription,
            live_row: false,
            transcript_bounds: Rc::new(Cell::new(None)),
            message_subscriptions: Vec::new(),
            messages: Vec::new(),
            live_view: cx.new(|cx| TextViewState::markdown("", cx).selectable(true)),
            live_reasoning_view: cx.new(|cx| TextViewState::markdown("", cx).selectable(true)),
            reasoning_views: Vec::new(),
            thinking_expanded: BTreeSet::new(),
            transcript_rows: Vec::new(),
            message_attachments: BTreeMap::new(),
            live: String::new(),
            reasoning: String::new(),
            progress: String::new(),
            expanded: BTreeSet::new(),
            message_menu: None,
            message_menu_subscription: None,
            pending_tool: None,
            scope: None,
            reference_scopes: Vec::new(),
            run: None,
            editor: None,
            show_settings: false,
            show_context_usage: false,
            settings_generation: 0,
            settings_tab: super::configuration::SettingsTab::Models,
            prompt_editor: None,
            mcp_editor: None,
            mcp_testing: false,
            mcp_status: String::new(),
            mcp_test_cancel: None,
            mcp_test_task: None,
            memories: Vec::new(),
            memory_editor: None,
            memory_loading: false,
            memory_task: None,
            busy: true,
            ui_busy: false,
            ui_task: None,
            error: String::new(),
            generation: 0,
            task: None,
            live_task: None,
            _subscriptions: vec![subscription, settings_subscription, memory_subscription],
        };
        this.open_conversations.push(this.conversation.id.clone());
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
                            Ok(settings) => {
                                this.settings = cx
                                    .global::<super::configuration::SharedAiSettings>()
                                    .settings
                                    .clone()
                                    .unwrap_or(settings);
                                if cx
                                    .global::<super::configuration::SharedAiSettings>()
                                    .settings
                                    .is_none()
                                {
                                    let settings = this.settings.clone();
                                    cx.update_global::<super::configuration::SharedAiSettings, _>(
                                        |shared, _| shared.settings = Some(settings),
                                    );
                                }
                            }
                            Err(error) => this.error = error.to_string(),
                        }
                        this.history_offset = history.len();
                        this.more_history = history.len() == 100;
                        this.history = history;
                        if let Some(record) = current {
                            this.install_record(record, cx);
                            this.open_conversations = vec![this.conversation.id.clone()];
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
                self.reference_scopes.clear();
                self.draft_logs.clear();
                self.attachment_task = None;
                self.attachments_loading = false;
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
    pub(super) fn message_view(
        &mut self,
        text: &str,
        ix: usize,
        cx: &mut Context<Self>,
    ) -> Entity<TextViewState> {
        let view = cx.new(|cx| TextViewState::markdown(text, cx).selectable(true));
        // Markdown parsing also completes asynchronously, after a stream update.
        // Remeasure that row when its rendered document becomes ready.
        self.message_subscriptions
            .push(cx.observe(&view, move |this, _, cx| {
                this.remeasure_message(ix, cx);
            }));
        view
    }
    pub(super) fn rebuild_messages(&mut self, cx: &mut Context<Self>) {
        self.rebuild_tab_messages(true, cx);
    }
    pub(super) fn rebuild_tab_messages(&mut self, reset_scroll: bool, cx: &mut Context<Self>) {
        self.message_menu = None;
        self.message_menu_subscription = None;
        self.messages.clear();
        self.reasoning_views.clear();
        self.thinking_expanded.clear();
        self.message_attachments.clear();
        self.message_subscriptions.clear();
        self.expanded.clear();
        self.live_row = false;
        for ix in 0..self.conversation.messages.len() {
            let text = message_text(
                &self.conversation.messages[ix],
                &self.conversation.messages[..ix],
            );
            let view = self.message_view(&text, ix, cx);
            self.messages.push(view);
            let reasoning = match &self.conversation.messages[ix] {
                AgentMessage::Assistant { reasoning, .. } if !reasoning.is_empty() => {
                    Some(safe_markdown(reasoning))
                }
                _ => None,
            };
            let view = reasoning.map(|text| self.message_view(&text, ix, cx));
            self.reasoning_views.push(view);
        }
        for ix in 0..self.messages.len() {
            self.cache_attachments(ix, cx);
        }
        self.sync_transcript(reset_scroll, cx);
    }
    fn recover_messages(&mut self, cx: &mut Context<Self>) {
        let count = self.conversation.messages.len();
        self.conversation.recover();
        // Ordinary sends must keep the reader's anchor and tail-follow choice.
        // Recovery only changes the transcript when it inserts missing tool results.
        if count != self.conversation.messages.len() {
            self.rebuild_messages(cx);
        }
    }
    fn ensure_live_row(&mut self, cx: &mut Context<Self>) {
        if self.live_row || (self.live.is_empty() && self.reasoning.is_empty()) {
            return;
        }
        self.live_view = self.message_view(&safe_markdown(&self.live), self.messages.len(), cx);
        self.live_reasoning_view =
            self.message_view(&safe_markdown(&self.reasoning), self.messages.len(), cx);
        self.live_row = true;
        if self.conversation.messages.is_empty()
            || matches!(
                self.conversation.messages.last(),
                Some(AgentMessage::User { .. })
            )
        {
            self.thinking_expanded.insert(self.messages.len());
        }
        self.sync_transcript(false, cx);
    }
    pub(super) fn push_message(&mut self, message: AgentMessage, cx: &mut Context<Self>) {
        let text = message_text(&message, &self.conversation.messages);
        let ix = self.messages.len();
        if !self.live_row
            && matches!(message, AgentMessage::Assistant { .. })
            && (self.conversation.messages.is_empty()
                || matches!(
                    self.conversation.messages.last(),
                    Some(AgentMessage::User { .. })
                ))
        {
            self.thinking_expanded.insert(ix);
        }
        if self.live_row && matches!(message, AgentMessage::Assistant { .. }) {
            // Commit the existing streamed row, including its Markdown entity
            // and scroll identity. There is no remove/append at the live edge.
            self.live_view
                .update(cx, |view, cx| view.set_text(&text, cx));
            self.messages.push(self.live_view.clone());
            let reasoning = match &message {
                AgentMessage::Assistant { reasoning, .. } => reasoning.as_str(),
                _ => "",
            };
            self.live_reasoning_view
                .update(cx, |view, cx| view.set_text(&safe_markdown(reasoning), cx));
            self.reasoning_views
                .push((!reasoning.is_empty()).then(|| self.live_reasoning_view.clone()));
            self.live_row = false;
        } else {
            let view = self.message_view(&text, ix, cx);
            self.messages.push(view);
            let reasoning = match &message {
                AgentMessage::Assistant { reasoning, .. } if !reasoning.is_empty() => {
                    Some(safe_markdown(reasoning))
                }
                _ => None,
            };
            let reasoning_view = reasoning.map(|text| self.message_view(&text, ix, cx));
            self.reasoning_views.push(reasoning_view);
        }
        self.conversation.messages.push(message);
        self.cache_attachments(ix, cx);
        self.sync_transcript(false, cx);
        cx.notify();
    }
    pub(super) fn record(&self) -> Result<AiConversationRecord> {
        Ok(AiConversationRecord {
            id: self.conversation.id.clone(),
            title: self.conversation.title.clone(),
            payload: serde_json::to_string(&self.conversation_with_log_sources())?,
            revision: self.revision,
        })
    }

    pub(super) fn send(&mut self, continuation: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings_busy(cx) || self.ui_busy || self.attachments_loading {
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
            self.open_settings(window, cx);
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
        if user_text.is_empty() && self.draft_logs.is_empty() {
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
        let user_text = match self.message_with_attachments(&user_text, &scope) {
            Ok(text) => text,
            Err(error) => {
                self.error = error.to_string();
                cx.notify();
                return;
            }
        };
        let sent_logs = self.draft_logs.clone();
        let attached_documents = self
            .draft_logs
            .iter()
            .map(|log| log.document.clone())
            .collect::<Vec<_>>();
        if let Some(previous) = self.scope.take() {
            self.reference_scopes.push(previous);
        }
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
        self.pending_tool = None;
        self.recover_messages(cx);
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
        let messages = self.conversation.active_messages();
        let skills = self
            .settings
            .skills
            .iter()
            .filter(|s| self.settings.skill_enabled(s))
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
        let settings_path = self.settings_path.clone();
        let prompts = self.settings.prompts.clone();
        let extensions = vclogg_ai::RunExtensions::from_settings(&self.settings)
            .with_conversation(&self.conversation);
        let workspace = self.workspace.clone();
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let _lease = lease;
            let save_store = store.clone();
            let capture = scope.clone();
            let saved = cx.background_spawn(async move {
                for doc in attached_documents { doc.verify()?; }
                let directory = capture.lock().map_err(|_| anyhow::anyhow!("Analysis unavailable"))?.directory.directory.clone();
                // A directory is an optional capability, not a prerequisite for chatting
                // or inspecting the captured open documents. Never fall back to another path.
                let canonical = directory.as_ref().and_then(|path| path.canonicalize().ok()).filter(|path| path.is_dir());
                let directory_unavailable = directory.is_some() && canonical.is_none();
                capture.lock().map_err(|_| anyhow::anyhow!("Analysis unavailable"))?.directory.directory = canonical;
                let instructions = vclogg_ai::agent_instructions(settings_path.as_deref().context("AI configuration directory unavailable")?, &prompts)?;
                save_store.save_ai_conversation(&record).map(|revision| (revision, directory_unavailable, instructions))
            }).await;
            let (revision, directory_unavailable, instructions) = match saved {
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
            _ = this.update(cx, |this, cx| {
                this.draft_logs.retain(|log| !sent_logs.iter().any(|sent| sent.source_row == log.source_row && Arc::ptr_eq(&sent.document.document, &log.document.document)));
                cx.notify();
            });
            let run = vclogg_ai::start_run_with_extensions(config, messages, skills, instructions, false, extensions);
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
                    if matches!(call.name.as_str(), "search_memory" | "save_memory" | "delete_memory") {
                        // Recheck the live switch as well as the run's captured policy.
                        let enabled = this.update(cx, |this, _| this.settings.memory_enabled).unwrap_or(false);
                        let result = if enabled {
                            let memory_store = store.clone();
                            let memory_call = call.clone();
                            let token = cancellation.clone();
                            cx.background_spawn(async move { super::memory::execute_memory_tool(&memory_store, &memory_call, &token) }).await
                        } else { Err(anyhow::anyhow!("Memory is disabled")) };
                        if result.is_ok() && call.name != "search_memory" {
                            gpui_kit::AsyncApp::update_global::<super::memory::SharedAiMemory, _>(cx, |_, _| {});
                        }
                        let result = match result { Ok(value) => ToolResult::ok(value), Err(error) => ToolResult::error(error.to_string()) };
                        if replies.send((call.id, result)).await.is_err() { break; }
                        continue;
                    }
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
                let persist = matches!(event, AgentEvent::Assistant(_) | AgentEvent::ToolFinished(_) | AgentEvent::ContextCompacted { .. } | AgentEvent::Finished(..));
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
                this.recover_messages(cx);
                this.collapse_current_thinking(cx);
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
    }
    fn collapse_current_thinking(&mut self, cx: &mut Context<Self>) {
        if let Some(&start) = self.transcript_rows.last()
            && self.thinking_expanded.remove(&start)
        {
            self.remeasure_message(start, cx);
        }
    }
    fn receive_event(&mut self, event: AgentEvent, cx: &mut Context<Self>) {
        if matches!(
            event,
            AgentEvent::ToolFinished(_) | AgentEvent::Finished(..)
        ) {
            self.conversation.log_sources = self.conversation_with_log_sources().log_sources;
        }
        match event {
            AgentEvent::ContextUsage(usage) => {
                self.conversation.context_usage = Some(usage);
            }
            AgentEvent::CompactionStarted => {
                self.progress = crate::tr!("正在压缩对话", "Compacting conversation").into();
            }
            AgentEvent::ContextCompacted { summary, through } => {
                self.conversation.context_summary = summary;
                self.conversation.summarized_messages =
                    through.min(self.conversation.messages.len());
                let active_tokens =
                    vclogg_ai::conversation_tokens(&self.conversation.active_messages());
                if let Some(usage) = self.conversation.context_usage.as_mut() {
                    usage.conversation_tokens = active_tokens;
                }
                self.conversation.notice = crate::tr!(
                    "对话已压缩，完整记录仍保存在本地",
                    "Conversation compacted; full transcript remains local"
                )
                .into();
            }
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
                self.ensure_live_row(cx);
                if self.live_row && self.live_task.is_none() {
                    self.live_task = Some(cx.spawn(async move |this, cx| {
                        cx.background_executor()
                            .timer(std::time::Duration::from_millis(40))
                            .await;
                        _ = this.update(cx, |this, cx| {
                            this.live_view.update(cx, |view, cx| {
                                view.set_text(&safe_markdown(&this.live), cx)
                            });
                            this.live_reasoning_view.update(cx, |view, cx| {
                                view.set_text(&safe_markdown(&this.reasoning), cx)
                            });
                            this.remeasure_message(this.messages.len(), cx);
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
                self.collapse_current_thinking(cx);
                self.busy = true; // Do not switch conversations until the final record is durable.
                self.run = None;
            }
            AgentEvent::ExtensionToolStarted(call) => {
                self.progress =
                    format!("{}: {}", crate::tr!("执行工具", "Running tool"), call.name);
                self.pending_tool = Some(call);
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
        if self.run.is_some() || self.busy || self.ui_busy {
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
        self.draft_logs.clear();
        self.attachment_task = None;
        self.attachments_loading = false;
        self.scope = None;
        self.reference_scopes.clear();
        self.error.clear();
        self.live.clear();
        self.reasoning.clear();
        self.progress.clear();
        self.rebuild_messages(cx);
        cx.notify();
    }
}
impl Drop for AiPanel {
    fn drop(&mut self) {
        if let Some(token) = &self.mcp_test_cancel {
            token.cancel();
        }
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
pub(super) fn message_text(message: &AgentMessage, history: &[AgentMessage]) -> String {
    match message {
        AgentMessage::User { text } => safe_markdown(super::transcript::user_content(text).0),
        AgentMessage::Assistant { text, .. } => safe_markdown(text),
        AgentMessage::Tool {
            call_id,
            name,
            result,
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
            let details = json!({"arguments":call.map(|call| &call.arguments),"result":vclogg_ai::log_reference_metadata(name, &result.value)});
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

#[cfg(test)]
mod analysis_tests;
