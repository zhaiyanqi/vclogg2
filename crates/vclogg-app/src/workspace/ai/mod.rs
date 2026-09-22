//! Window-owned AI presentation and a capability-limited bridge into Workspace commands.
use super::*;
use anyhow::{Context as _, bail};
use serde_json::{Value, json};
use vclogg_ai::{AgentMessage, LogReference, ToolCall, ToolResult};

mod annotation_tools;
mod attachments;
mod commands;
mod context_tools;
mod evidence;
mod files;
mod log_tools;
mod logs;
mod navigation;
mod navigation_tools;
mod search;
mod search_tools;
mod transcript;
mod transcript_scroll;
use logs::*;
mod configuration;
mod context_view;
mod conversation_tab_view;
mod conversation_tabs;
mod mcp_settings;
mod memory;
mod panel;
mod prompt_settings;
mod question;
mod settings;
mod skill_settings;
#[cfg(test)]
mod tests;
mod tool_settings;
mod view;
mod workspace_settings;
pub(super) use panel::AiPanel;

const AI_LABEL_LINE_HEIGHT: f32 = 1.25;

trait AiButtonExt {
    fn text_label(self, label: impl Into<SharedString>) -> Self;
}

impl AiButtonExt for Button {
    fn text_label(self, label: impl Into<SharedString>) -> Self {
        let label = label.into();
        // Button's built-in label clips glyphs to 1em. Keep its sizing and
        // behavior, but give the text its own line box and accessible name.
        crate::button_accessibility::with_label(self, label.clone()).child(
            div()
                .min_w_0()
                .truncate()
                .line_height(relative(AI_LABEL_LINE_HEIGHT))
                .child(label),
        )
    }
}

#[derive(Clone)]
struct DocumentSnapshot {
    id: u64,
    version: String,
    document: Arc<LogDocument>,
    open: bool,
}
impl DocumentSnapshot {
    fn absolute_path(&self) -> Result<PathBuf> {
        std::path::absolute(self.document.path()).context("Cannot resolve absolute file path")
    }
    fn reference(&self, row: usize) -> LogReference {
        LogReference {
            document_id: self.id,
            version: self.version.clone(),
            line: row + 1,
        }
    }
    fn verify(&self) -> Result<()> {
        if self.document.source_changed()? {
            bail!("Log content changed; refresh the file and call list_logs again");
        }
        Ok(())
    }
}
#[derive(Clone)]
struct SearchSnapshot {
    groups: Vec<(DocumentSnapshot, CompressedRows)>,
    query: Option<SearchQuery>,
    cursor: Option<usize>,
    truncated: bool,
    tab: Option<(search_tabs::SearchTabOwner, search_tabs::SearchTabId, u64)>,
}

#[derive(Clone)]
struct AiScope {
    file_candidates: BTreeMap<String, PathBuf>,
    project_directories: Vec<PathBuf>,
    source_directory_request: String,
    read_document: Option<DocumentSnapshot>,
    explicit: BTreeSet<u64>,
    allowed: BTreeSet<u64>,
    current: Option<u64>,
    documents: BTreeMap<u64, DocumentSnapshot>,
    directory: DirectorySearchOptions,
    searches: BTreeMap<String, SearchSnapshot>,
    tabs: BTreeMap<String, (search_tabs::SearchTabOwner, search_tabs::SearchTabId, u64)>,
    cancellation: SearchCancellation,
}
impl Drop for AiScope {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}
type SharedScope = Arc<Mutex<AiScope>>;
enum Evidence {
    Json(Value),
    Context(Value, Vec<DocumentSnapshot>),
    Color(Box<PreparedColorRuleUpdate>),
    Open(Box<PreparedDocument>),
}
type Work = Box<dyn FnOnce() -> Result<Evidence> + Send>;

struct ConversationLease(String);
fn leases() -> &'static Mutex<BTreeSet<String>> {
    static LEASES: std::sync::OnceLock<Mutex<BTreeSet<String>>> = std::sync::OnceLock::new();
    LEASES.get_or_init(Mutex::default)
}
impl ConversationLease {
    fn acquire(id: &str) -> Result<Self> {
        if !leases()
            .lock()
            .map_err(|_| anyhow::anyhow!("Conversation lock unavailable"))?
            .insert(id.to_owned())
        {
            bail!("This conversation is running in another window");
        }
        Ok(Self(id.to_owned()))
    }
}
impl Drop for ConversationLease {
    fn drop(&mut self) {
        if let Ok(mut leases) = leases().lock() {
            leases.remove(&self.0);
        }
    }
}

impl AiScope {
    fn document(&self, id: u64, version: Option<&str>) -> Result<DocumentSnapshot> {
        let document = self
            .documents
            .get(&id)
            .context("File is outside this run or has been closed")?;
        if version.is_some_and(|v| v != document.version) {
            bail!("Stale log reference; call list_logs again");
        }
        Ok(document.clone())
    }
    fn navigation_target(&self, args: &Value) -> Result<(DocumentSnapshot, usize, Option<usize>)> {
        let action = text(args, "action")?;
        match action {
            "line" => self
                .reference(&args["reference"])
                .map(|(doc, row)| (doc, row, None)),
            "start" | "end" => {
                let doc = self.document(number(args, "document_id")?, None)?;
                let row = if action == "start" {
                    0
                } else {
                    doc.document.source_line_count().saturating_sub(1)
                };
                Ok((doc, row, None))
            }
            _ => {
                let search = self
                    .searches
                    .get(text(args, "search_id")?)
                    .context("Search unavailable")?;
                let total = search
                    .groups
                    .iter()
                    .map(|(_, rows)| rows.len())
                    .sum::<usize>();
                if total == 0 {
                    bail!("No search hits");
                }
                let cursor = if action == "result" {
                    let index = number(args, "result_index")? as usize;
                    if index == 0 || index > total {
                        bail!("Result index out of range");
                    }
                    index - 1
                } else if action == "next" {
                    search.cursor.map_or(0, |c| (c + 1) % total)
                } else {
                    search.cursor.map_or(total - 1, |c| (c + total - 1) % total)
                };
                let mut offset = cursor;
                for (doc, rows) in &search.groups {
                    if offset < rows.len() {
                        return Ok((
                            doc.clone(),
                            rows.get(offset).context("Search hit unavailable")?,
                            Some(cursor),
                        ));
                    }
                    offset -= rows.len();
                }
                bail!("Search hit unavailable")
            }
        }
    }
    fn reference(&self, value: &Value) -> Result<(DocumentSnapshot, usize)> {
        let reference: LogReference = serde_json::from_value(value.clone())?;
        let document = self.document(reference.document_id, Some(&reference.version))?;
        let row = reference.line.checked_sub(1).context("Lines start at 1")?;
        if row >= document.document.source_line_count() {
            bail!("Source line unavailable");
        }
        Ok((document, row))
    }
}

impl Workspace {
    fn ai_scope(&self) -> SharedScope {
        let documents = self
            .documents
            .iter()
            .filter(|tab| tab.load_state == DocumentLoadState::Ready)
            .map(|tab| {
                (
                    tab.id,
                    DocumentSnapshot {
                        id: tab.id,
                        version: uuid::Uuid::new_v4().to_string(),
                        document: tab.document.clone(),
                        open: true,
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        Arc::new(Mutex::new(AiScope {
            file_candidates: BTreeMap::new(),
            project_directories: Vec::new(),
            source_directory_request: String::new(),
            read_document: None,
            explicit: BTreeSet::new(),
            allowed: documents.keys().copied().collect(),
            current: self.active_document().map(|t| t.id),
            documents,
            directory: self.global_search.directory_options.clone(),
            searches: BTreeMap::new(),
            tabs: BTreeMap::new(),
            cancellation: SearchCancellation::default(),
        }))
    }

    fn ai_context_reference(
        &self,
        documents: &mut BTreeMap<u64, DocumentSnapshot>,
        directory: &DirectorySearchOptions,
        id: u64,
        row: usize,
    ) -> Option<LogReference> {
        if let Some(doc) = documents.get(&id) {
            return Some(doc.reference(row));
        }
        let result = self.global_search.results.get(&id)?;
        if let Some(doc) = documents
            .values()
            .find(|d| result_snapshot_matches_document(&result.path, &result.document, &d.document))
        {
            return Some(doc.reference(row));
        }
        // Canonical path containment is verified by the worker before returning any content.
        if !result.path.starts_with(directory.directory.as_ref()?) {
            return None;
        }
        let doc = DocumentSnapshot {
            id: next_directory_id(),
            version: uuid::Uuid::new_v4().to_string(),
            document: result.document.clone(),
            open: false,
        };
        let reference = doc.reference(row);
        documents.insert(doc.id, doc);
        Some(reference)
    }
    fn validate_ai_search(&self, state: &AiScope, call: &ToolCall) -> Result<()> {
        if let Some(id) = call.arguments["search_id"].as_str() {
            let search = state.searches.get(id).context("Search unavailable")?;
            if let Some((owner, id, revision)) = search.tab
                && !self
                    .search_tabs
                    .state(owner, id)
                    .is_some_and(|tab| tab.revision == revision && tab.saved.completed.is_some())
            {
                bail!("Search results expired; acquire a new result reference");
            }
        }
        Ok(())
    }
    fn ai_prepare(&self, scope: SharedScope, call: &ToolCall, cx: &App) -> Result<Work> {
        vclogg_ai::validate_call(call)?;
        let route = vclogg_ai::tool_descriptor(&call.name)
            .context("Unknown tool")?
            .route();
        if route == vclogg_ai::ToolRoute::Files
            && !matches!(call.name.as_str(), "close_file" | "switch_file")
        {
            return self.ai_prepare_file(scope, call);
        }
        let mut state = scope
            .lock()
            .map_err(|_| anyhow::anyhow!("Analysis state unavailable"))?;
        self.validate_ai_search(&state, call)?;
        // Refresh explicitly, while preserving the run's original set of allowed open files.
        if matches!(call.name.as_str(), "list_logs" | "get_context") {
            let directory_opened = self
                .documents
                .iter()
                .filter(|tab| {
                    tab.load_state == DocumentLoadState::Ready
                        && state.documents.values().any(|doc| {
                            !doc.open && paths_match(doc.document.path(), tab.document.path())
                        })
                })
                .map(|tab| tab.id)
                .collect::<Vec<_>>();
            state.allowed.extend(directory_opened);
            let allowed = state.allowed.clone();
            for id in allowed {
                if let Some(tab) = self
                    .documents
                    .iter()
                    .find(|tab| tab.id == id && tab.load_state == DocumentLoadState::Ready)
                {
                    let unchanged = state
                        .documents
                        .get(&id)
                        .is_some_and(|old| Arc::ptr_eq(&old.document, &tab.document));
                    if !unchanged {
                        state.documents.insert(
                            id,
                            DocumentSnapshot {
                                id,
                                version: uuid::Uuid::new_v4().to_string(),
                                document: tab.document.clone(),
                                open: true,
                            },
                        );
                    }
                } else if let Some(doc) = state.documents.get(&id).cloned() {
                    state.forget_file(doc.document.path());
                }
            }
        }
        drop(state);
        use vclogg_ai::ToolRoute;
        match route {
            ToolRoute::Context => self.ai_prepare_context(scope, call, cx),
            ToolRoute::Logs => self.ai_prepare_logs(scope, call, cx),
            ToolRoute::SearchView => self.ai_prepare_search(scope, call, cx),
            ToolRoute::Annotations => self.ai_prepare_annotations(scope, call, cx),
            ToolRoute::Navigation => self.ai_prepare_navigation(scope, call, cx),
            ToolRoute::Files => self.ai_prepare_state_change(scope, call),
            _ => bail!("Tool is not handled by the workspace"),
        }
    }
}

impl Workspace {
    fn ai_prepare_state_change(&self, scope: SharedScope, call: &ToolCall) -> Result<Work> {
        let state = scope
            .lock()
            .map_err(|_| anyhow::anyhow!("Analysis state unavailable"))?;
        let args = &call.arguments;

        // Validate disk evidence on the worker; validate Arc identity again at commit.
        let docs = mutation_documents(&state, call)?;
        let directory = (call.name == "show_search" && args["scope"] == "directory")
            .then(|| state.directory.directory.clone());
        Ok(Box::new(move || {
            if let Some(directory) = directory {
                approved_directory(directory.as_deref().context("Select a directory first")?)?;
            }
            for doc in docs {
                doc.verify()?;
            }
            Ok(Evidence::Json(json!({})))
        }))
    }
}

impl Evidence {
    fn value(&self) -> Result<&Value> {
        match self {
            Self::Json(value) | Self::Context(value, _) => Ok(value),
            Self::Color(_) | Self::Open(_) => bail!("Unexpected prepared evidence"),
        }
    }
    fn into_value(self) -> Result<Value> {
        match self {
            Self::Json(value) | Self::Context(value, _) => Ok(value),
            Self::Color(_) | Self::Open(_) => bail!("Unexpected prepared evidence"),
        }
    }
}
