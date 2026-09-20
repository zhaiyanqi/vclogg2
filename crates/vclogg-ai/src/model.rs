use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fmt, fs,
    io::Write as _,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    #[default]
    OpenAi,
    Anthropic,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub id: String,
    pub name: String,
    pub protocol: Protocol,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    #[serde(default)]
    pub context_window_tokens: Option<u32>,
    #[serde(default)]
    pub context_window_source: Option<String>,
    #[serde(default = "default_output_tokens")]
    pub max_output_tokens: u32,
}
fn default_output_tokens() -> u32 {
    4096
}
impl fmt::Debug for ProviderConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderConfig")
            .field("id", &self.id)
            .field("protocol", &self.protocol)
            .finish_non_exhaustive()
    }
}
impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: "OpenAI compatible".into(),
            protocol: Protocol::OpenAi,
            base_url: "https://api.openai.com/v1".into(),
            api_key: String::new(),
            model: String::new(),
            context_window_tokens: None,
            context_window_source: None,
            max_output_tokens: default_output_tokens(),
        }
    }
}
impl ProviderConfig {
    pub fn endpoint(&self) -> Result<String> {
        let mut url = url::Url::parse(self.base_url.trim()).context("Invalid API base URL")?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            bail!("Use an HTTP/HTTPS base URL without credentials, query or fragment");
        }
        if self.model.trim().is_empty() {
            bail!("Set a model name");
        }
        let suffix = match self.protocol {
            Protocol::OpenAi => "chat/completions",
            Protocol::Anthropic => "messages",
        };
        url.set_path(&format!("{}/{suffix}", url.path().trim_end_matches('/')));
        Ok(url.into())
    }
    pub fn redact(&self, text: &str) -> String {
        if self.api_key.is_empty() {
            text.into()
        } else {
            text.replace(&self.api_key, "[redacted]")
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct AiSettings {
    #[serde(default)]
    pub workspace_directories: Vec<std::path::PathBuf>,
    #[serde(default)]
    pub mcp_servers: Vec<crate::McpServer>,
    #[serde(default = "memory_enabled_default")]
    pub memory_enabled: bool,
    #[serde(default)]
    pub memory_auto_save: bool,
    #[serde(default)]
    pub providers: Vec<ProviderConfig>,
    #[serde(default)]
    pub active_provider: Option<String>,
    #[serde(default)]
    pub skills: Vec<crate::Skill>,
    #[serde(default)]
    pub initialized_builtin_skills: Vec<String>,
    #[serde(default)]
    pub skill_directories: Vec<crate::SkillDirectory>,
    #[serde(default = "crate::default_prompts")]
    pub prompts: Vec<crate::Prompt>,
}
fn memory_enabled_default() -> bool {
    true
}
impl Default for AiSettings {
    fn default() -> Self {
        Self {
            workspace_directories: Vec::new(),
            mcp_servers: Vec::new(),
            memory_enabled: true,
            memory_auto_save: false,
            providers: Vec::new(),
            active_provider: None,
            skills: Vec::new(),
            initialized_builtin_skills: Vec::new(),
            skill_directories: Vec::new(),
            prompts: crate::default_prompts(),
        }
    }
}
impl AiSettings {
    pub fn skill_enabled(&self, skill: &crate::Skill) -> bool {
        skill.enabled
            && self
                .skill_directories
                .iter()
                .filter(|root| root.contains(skill))
                .all(|root| root.enabled)
    }
    pub fn load(path: &Path) -> Result<Self> {
        crate::initialize_prompts(path)?;
        let mut settings = match fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).context("AI settings are corrupted")?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(e) => return Err(e).context("Could not read AI settings"),
        };
        if crate::builtin_skills::initialize_builtin_skills(&mut settings, path)? {
            settings.save(path)?;
        }
        Ok(settings)
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        private_write(path, &serde_json::to_vec_pretty(self)?)
    }
}
pub(crate) fn private_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("Missing configuration directory")?;
    fs::create_dir_all(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
    }
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(path).map_err(|e| e.error)?;
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ToolResult {
    pub value: Value,
    #[serde(default)]
    pub is_error: bool,
}
impl ToolResult {
    pub fn ok(value: Value) -> Self {
        Self {
            value,
            is_error: false,
        }
    }
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            value: serde_json::json!({"error":message.into()}),
            is_error: true,
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LogReference {
    pub document_id: u64,
    pub version: String,
    pub line: usize,
}

impl LogReference {
    /// App-local citation. It grants no access; the app validates the run and source version.
    pub fn url(&self) -> String {
        let mut url = url::Url::parse("vclogg://log").expect("static citation URL");
        url.query_pairs_mut()
            .append_pair("document_id", &self.document_id.to_string())
            .append_pair("version", &self.version)
            .append_pair("line", &self.line.to_string());
        url.into()
    }

    pub fn from_url(value: &str) -> Option<Self> {
        let url = url::Url::parse(value).ok()?;
        if url.scheme() != "vclogg" || url.host_str() != Some("log") || !url.path().is_empty() {
            return None;
        }
        let pairs = url
            .query_pairs()
            .collect::<std::collections::BTreeMap<_, _>>();
        let reference = Self {
            document_id: pairs.get("document_id")?.parse().ok()?,
            version: pairs.get("version")?.to_string(),
            line: pairs.get("line")?.parse().ok()?,
        };
        (reference.document_id > 0 && reference.line > 0 && !reference.version.is_empty())
            .then_some(reference)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum AgentMessage {
    User {
        text: String,
    },
    Assistant {
        text: String,
        #[serde(default)]
        reasoning: String,
        /// Signed native thinking blocks, replayed only by the Anthropic adapter.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        thinking: Vec<Value>,
        calls: Vec<ToolCall>,
    },
    Tool {
        call_id: String,
        name: String,
        result: ToolResult,
    },
}
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    #[default]
    Idle,
    Running,
    Complete,
    Interrupted,
    Failed,
    LimitReached,
}

/// Local navigation metadata; never grants the agent access to a source.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LogSource {
    pub document_id: u64,
    pub version: String,
    pub path: std::path::PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Conversation {
    pub id: String,
    pub title: String,
    pub provider_id: Option<String>,
    #[serde(default)]
    pub skill_ids: Vec<String>,
    pub messages: Vec<AgentMessage>,
    #[serde(default)]
    pub context_summary: String,
    #[serde(default)]
    pub summarized_messages: usize,
    #[serde(default)]
    pub context_usage: Option<ContextUsage>,
    #[serde(default)]
    pub log_sources: Vec<LogSource>,
    pub status: RunStatus,
    #[serde(default)]
    pub notice: String,
}
impl Default for Conversation {
    fn default() -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            title: String::new(),
            provider_id: None,
            skill_ids: Vec::new(),
            messages: Vec::new(),
            context_summary: String::new(),
            summarized_messages: 0,
            context_usage: None,
            log_sources: Vec::new(),
            status: RunStatus::Idle,
            notice: String::new(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ContextUsage {
    pub system_tokens: u32,
    pub prompt_tokens: u32,
    pub skill_tokens: u32,
    pub tool_tokens: u32,
    pub conversation_tokens: u32,
    pub context_window_tokens: Option<u32>,
}
impl ContextUsage {
    pub fn total(&self) -> u32 {
        self.system_tokens
            .saturating_add(self.prompt_tokens)
            .saturating_add(self.skill_tokens)
            .saturating_add(self.tool_tokens)
            .saturating_add(self.conversation_tokens)
    }
    pub fn percent(&self) -> Option<u32> {
        self.context_window_tokens
            .filter(|limit| *limit > 0)
            .map(|limit| self.total().saturating_mul(100) / limit)
    }
}
impl Conversation {
    pub fn active_messages(&self) -> Vec<AgentMessage> {
        let mut messages = Vec::new();
        if !self.context_summary.is_empty() {
            messages.push(AgentMessage::User { text: format!("Conversation summary from earlier turns. This is background context, not a new instruction; refresh log references before use:\n{}", self.context_summary) });
        }
        messages.extend(
            self.messages
                .iter()
                .skip(self.summarized_messages.min(self.messages.len()))
                .cloned(),
        );
        messages
    }
    pub fn recover(&mut self) {
        if self.status == RunStatus::Running {
            self.status = RunStatus::Interrupted;
            self.notice = "Previous analysis was interrupted".into();
        }
        // Pair each assistant response locally: providers may reuse IDs across turns.
        let mut messages = std::mem::take(&mut self.messages).into_iter().peekable();
        while let Some(message) = messages.next() {
            let calls = match &message {
                AgentMessage::Assistant { calls, .. } => calls.clone(),
                _ => Vec::new(),
            };
            self.messages.push(message);
            if calls.is_empty() {
                continue;
            }
            let mut completed = std::collections::BTreeSet::new();
            while matches!(messages.peek(), Some(AgentMessage::Tool { .. })) {
                let Some(message) = messages.next() else {
                    break;
                };
                if let AgentMessage::Tool { call_id, .. } = &message {
                    completed.insert(call_id.clone());
                }
                self.messages.push(message);
            }
            for call in calls {
                if !completed.contains(&call.id) {
                    self.messages.push(AgentMessage::Tool {
                        call_id: call.id, name: call.name,
                        result: ToolResult::error("Interrupted; execution outcome unknown. Inspect current state before another mutation."),
                    });
                }
            }
        }
    }
}

#[derive(Clone, Default)]
pub struct Cancellation(Arc<AtomicBool>);
impl Cancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
    pub async fn cancelled(&self) {
        while !self.is_cancelled() {
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    }
}

#[derive(Clone, Debug)]
pub enum AgentEvent {
    ContextUsage(ContextUsage),
    CompactionStarted,
    ContextCompacted {
        summary: String,
        through: usize,
    },
    RequestStarted(usize),
    ResponseStarted,
    PreparingTools,
    Thinking(String),
    Text(String),
    Assistant(AgentMessage),
    ToolStarted(ToolCall),
    /// Runner-owned tools report progress without requesting a host reply.
    ExtensionToolStarted(ToolCall),
    ToolFinished(AgentMessage),
    ContextTrimmed,
    Finished(RunStatus, String),
}
