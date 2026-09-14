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
    pub max_output_tokens: u32,
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
            max_output_tokens: 4096,
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
        if self.model.trim().is_empty()
            || self.max_output_tokens == 0
            || self.max_output_tokens > 131_072
        {
            bail!("Set a model and a valid output limit (1–131072)");
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

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct AiSettings {
    #[serde(default)]
    pub providers: Vec<ProviderConfig>,
    #[serde(default)]
    pub active_provider: Option<String>,
    #[serde(default)]
    pub skills: Vec<crate::Skill>,
}
impl AiSettings {
    pub fn load(path: &Path) -> Result<Self> {
        match fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).context("AI settings are corrupted"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).context("Could not read AI settings"),
        }
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Conversation {
    pub id: String,
    pub title: String,
    pub provider_id: Option<String>,
    #[serde(default)]
    pub skill_ids: Vec<String>,
    pub messages: Vec<AgentMessage>,
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
            status: RunStatus::Idle,
            notice: String::new(),
        }
    }
}
impl Conversation {
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
    RequestStarted(usize),
    ResponseStarted,
    PreparingTools,
    Thinking(String),
    Text(String),
    Assistant(AgentMessage),
    ToolStarted(ToolCall),
    ToolFinished(AgentMessage),
    ContextTrimmed,
    Finished(RunStatus, String),
}
