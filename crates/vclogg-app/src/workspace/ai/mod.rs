//! Window-owned AI presentation and a capability-limited bridge into Workspace commands.
use super::*;
use anyhow::{Context as _, bail};
use serde_json::{Value, json};
use vclogg_ai::{AgentMessage, LogReference, ToolCall, ToolResult};

mod attachments;
mod commands;
mod evidence;
mod files;
mod logs;
mod search;
mod transcript;
use logs::*;
mod configuration;
mod conversation_tab_view;
mod conversation_tabs;
mod mcp_settings;
mod memory;
mod panel;
mod prompt_settings;
mod settings;
mod skill_settings;
#[cfg(test)]
mod tests;
mod view;
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
        if matches!(
            call.name.as_str(),
            "locate_files" | "open_file" | "reveal_file"
        ) {
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
        let args = &call.arguments;
        let value = match call.name.as_str() {
            "list_logs" => {
                json!({"files":state.documents.values().map(|d| json!({"document_id":d.id,"version":d.version,"name":d.document.file_name(),"path":d.document.path().display().to_string(),"lines":d.document.source_line_count(),"open":d.open})).collect::<Vec<_>>()})
            }
            "get_context" => {
                let tab = self.active_document();
                let region = match self.active_log_region {
                    LogRegion::Body => "body",
                    LogRegion::CurrentResults => "current_results",
                    LogRegion::GlobalResults => "global_results",
                };
                let mut refs = Vec::new();
                let mut context_documents = state.documents.clone();
                if let Some(tab) = tab.and_then(|t| state.documents.get(&t.id).map(|d| (t, d))) {
                    let (tab, doc) = tab;
                    let table = if self.active_log_region == LogRegion::CurrentResults {
                        &tab.result_table
                    } else {
                        &tab.log_table
                    };
                    let table = table.read(cx);
                    for ix in table.viewport().visible_range().take(100) {
                        if let Some(LogRowKey::Row { source_row, .. }) =
                            table.delegate().row_key(ix)
                        {
                            refs.push(doc.reference(source_row));
                        }
                    }
                }
                if self.active_log_region == LogRegion::GlobalResults {
                    refs.clear();
                    let table = self.global_table.read(cx);
                    for ix in table.viewport().visible_range().take(100) {
                        if let Some(LogRowKey::Row {
                            document_id,
                            source_row,
                        }) = table.delegate().row_key(ix)
                            && let Some(reference) = self.ai_context_reference(
                                &mut context_documents,
                                &state.directory,
                                document_id,
                                source_row,
                            )
                        {
                            refs.push(reference);
                        }
                    }
                }
                let mut selected = self
                    .active_document()
                    .and_then(|t| state.documents.get(&t.id).map(|d| (t, d)))
                    .map(|(t, d)| {
                        t.selected_source_rows_compressed(cx)
                            .iter()
                            .take(100)
                            .map(|r| d.reference(r))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                if self.active_log_region == LogRegion::GlobalResults {
                    selected.clear();
                    for (id, rows) in self.global_table.read(cx).delegate().selection_snapshot() {
                        for row in rows.iter() {
                            if selected.len() == 100 {
                                break;
                            }
                            if let Some(reference) = self.ai_context_reference(
                                &mut context_documents,
                                &state.directory,
                                id,
                                row,
                            ) {
                                selected.push(reference);
                            }
                        }
                    }
                }
                let metadata = json!({"current_document_id":tab.map(|t|t.id),"send_document_id":state.current,"region":region,"selected":selected,"query":self.query.read(cx).value().to_string(),"case_sensitive":self.case_sensitive,"regex":self.regex,"results_visible":self.global_search.results_visible,"searching":self.search_tabs.running.is_some(),"directory":state.directory.directory.as_ref().map(|p|p.display().to_string())});
                let directory = state.directory.directory.clone();
                let explicit = state.explicit.clone();
                let docs = context_documents;
                let checked = refs
                    .iter()
                    .chain(selected.iter())
                    .map(|r| r.document_id)
                    .collect::<BTreeSet<_>>();
                drop(state);
                return Ok(Box::new(move || {
                    for id in &checked {
                        let doc = &docs[id];
                        doc.verify()?;
                        if !doc.open && !explicit.contains(id) {
                            let root = approved_directory(
                                directory.as_deref().context("Select a directory first")?,
                            )?;
                            if !doc.document.path().canonicalize()?.starts_with(root) {
                                bail!("Visible result is outside this run's directory");
                            }
                        }
                    }

                    let mut value = metadata;
                    value["visible"] = json!(refs);
                    value["content_included"] = json!(false);
                    let mut state = scope
                        .lock()
                        .map_err(|_| anyhow::anyhow!("Analysis unavailable"))?;
                    if state.cancellation.is_cancelled() {
                        bail!("Analysis stopped");
                    }
                    for id in checked {
                        state.documents.insert(id, docs[&id].clone());
                    }
                    Ok(Evidence::Json(value))
                }));
            }
            "list_filters" => {
                json!({"filters":self.predefined_filters.iter().map(|f| json!({"id":f.id.to_string(),"name":f.name,"query":f.value,"regex":f.use_regex})).collect::<Vec<_>>()})
            }
            "list_colors" => {
                json!({"colors":self.color_labels.iter().map(|l| json!({"id":l.id,"name":l.name})).collect::<Vec<_>>(),"text_mark_color":"neutral"})
            }
            "read_logs" => {
                let doc = state.document(number(args, "document_id")?, args["version"].as_str())?;
                let start = number(args, "start_line")? as usize - 1;
                let limit = args["limit"].as_u64().unwrap_or(10) as usize;
                let cancellation = state.cancellation.clone();
                drop(state);
                return Ok(Box::new(move || {
                    let doc = read_snapshot(
                        &scope,
                        doc,
                        start,
                        start.saturating_add(limit),
                        &cancellation,
                    )?;
                    read_page_cancellable(&doc, start, limit, &cancellation).map(Evidence::Json)
                }));
            }
            "read_log_context" => {
                let (doc, row) = state.reference(&args["reference"])?;
                let before = args["before"].as_u64().unwrap_or(3) as usize;
                let after = args["after"].as_u64().unwrap_or(3) as usize;
                let cancellation = state.cancellation.clone();
                drop(state);
                return Ok(Box::new(move || {
                    let doc = read_snapshot(
                        &scope,
                        doc,
                        row.saturating_sub(before),
                        row.saturating_add(after).saturating_add(1),
                        &cancellation,
                    )?;
                    evidence::read_context(&doc, row, before, after, &cancellation)
                        .map(Evidence::Json)
                }));
            }
            "read_log_segment" => {
                let (doc, row) = state.reference(&args["reference"])?;
                if args.get("search_id").is_some() && args.get("start_character").is_some() {
                    bail!("Use search_id or start_character, exclusively");
                }
                let query = if let Some(id) = args["search_id"].as_str() {
                    let search = state
                        .searches
                        .get(id)
                        .context("Search not found in this run")?;
                    if !search.groups.iter().any(|(source, rows)| {
                        source.id == doc.id && source.version == doc.version && rows.contains(row)
                    }) {
                        bail!("The reference is not a hit in this search");
                    }
                    Some(search.query.clone().context("Search query unavailable")?)
                } else {
                    None
                };
                let start = args["start_character"].as_u64().unwrap_or(0) as usize;
                let limit = args["max_characters"].as_u64().unwrap_or(2048) as usize;
                let cancellation = state.cancellation.clone();
                drop(state);
                return Ok(Box::new(move || {
                    let doc =
                        read_snapshot(&scope, doc, row, row.saturating_add(1), &cancellation)?;
                    evidence::read_segment(&doc, row, start, limit, query.as_ref(), &cancellation)
                        .map(Evidence::Json)
                }));
            }
            "summarize_search" => {
                let search = state
                    .searches
                    .get(text(args, "search_id")?)
                    .context("Search not found in this run")?
                    .clone();
                let offset = args["offset"].as_u64().unwrap_or(0) as usize;
                return Ok(Box::new(move || {
                    evidence::summarize_search(&search, offset).map(Evidence::Json)
                }));
            }
            "search_logs" => {
                let query = query(args)?;
                let kind = args["scope"].as_str().unwrap_or("current");
                let docs = if kind == "current" {
                    let id = args["document_id"]
                        .as_u64()
                        .or(state.current)
                        .context("No current file; specify document_id from list_logs")?;
                    vec![state.document(id, None)?]
                } else {
                    if args.get("document_id").is_some() {
                        bail!("document_id requires current scope");
                    }
                    state
                        .documents
                        .values()
                        .filter(|d| d.open)
                        .cloned()
                        .collect::<Vec<_>>()
                };
                let directory = (kind == "directory").then(|| state.directory.clone());
                let cancellation = state.cancellation.clone();
                drop(state);
                return Ok(Box::new(move || {
                    search_logs(scope, docs, directory, query, cancellation).map(Evidence::Json)
                }));
            }
            "search_results" => {
                let search = state
                    .searches
                    .get(text(args, "search_id")?)
                    .context("Search not found in this run")?
                    .clone();
                let offset = args["offset"].as_u64().unwrap_or(0) as usize;
                let limit = args["limit"].as_u64().unwrap_or(20) as usize;
                let search_id = text(args, "search_id")?.to_owned();
                return Ok(Box::new(move || {
                    let mut page = search_page_options(&search, offset, limit)?;
                    add_search_links(&mut page, &search_id, offset);
                    Ok(Evidence::Json(page))
                }));
            }
            "control_search" if args["action"] == "results" => {
                let key = text(args, "search_tab")?;
                let (owner, id, revision) =
                    *state.tabs.get(key).context("Not an AI-owned search tab")?;
                let tab = self
                    .search_tabs
                    .state(owner, id)
                    .context("Search tab closed")?;
                if tab.revision != revision {
                    bail!("Search changed; reacquire results");
                }
                if tab.saved.completed.is_none() {
                    bail!("Search has not completed; check status first");
                }
                let mut search = SearchSnapshot {
                    groups: Vec::new(),
                    query: tab.saved.completed.as_ref().map(|q| SearchQuery {
                        text: q.text.clone(),
                        case_sensitive: q.case_sensitive,
                        regex: q.regex,
                        max_results: None,
                    }),
                    cursor: None,
                    truncated: false,
                    tab: Some((owner, id, revision)),
                };
                let mut sources = Vec::new();
                if let Some((document, result, _)) = &tab.local_result {
                    sources.push((document.clone(), result.clone()));
                } else {
                    sources.extend(
                        tab.context
                            .results
                            .values()
                            .map(|r| (r.document.clone(), r.search_result.clone())),
                    );
                }
                for (document, result) in sources {
                    let snapshot = state
                        .documents
                        .values()
                        .find(|d| Arc::ptr_eq(&d.document, &document))
                        .cloned()
                        .unwrap_or_else(|| DocumentSnapshot {
                            id: next_directory_id(),
                            version: uuid::Uuid::new_v4().to_string(),
                            document,
                            open: false,
                        });
                    search.truncated |= result.truncated;
                    search.groups.push((snapshot, result.line_indices));
                }
                let directory = state.directory.directory.clone();
                let offset = args["offset"].as_u64().unwrap_or(0) as usize;
                let search_id = format!("tab:{key}:{revision}");
                drop(state);
                return Ok(Box::new(move || {
                    for (doc, _) in &search.groups {
                        if !doc.open {
                            let root = approved_directory(
                                directory
                                    .as_deref()
                                    .context("Result outside captured files")?,
                            )?;
                            if !doc.document.path().canonicalize()?.starts_with(root) {
                                bail!("Result outside captured directory");
                            }
                        }
                    }
                    let mut page = search_page(&search, offset)?;
                    add_search_links(&mut page, &search_id, offset);
                    let mut state = scope
                        .lock()
                        .map_err(|_| anyhow::anyhow!("Analysis unavailable"))?;
                    if state.cancellation.is_cancelled() {
                        bail!("Analysis stopped");
                    }
                    for (doc, _) in &search.groups {
                        state.documents.insert(doc.id, doc.clone());
                    }
                    state.searches.insert(search_id, search);
                    Ok(Evidence::Json(page))
                }));
            }
            "navigate" => {
                let (doc, _, _) = state.navigation_target(args)?;
                if doc.open {
                    return Ok(Box::new(move || {
                        doc.verify()?;
                        Ok(Evidence::Json(json!({})))
                    }));
                }
                let store = self.persistence.store.clone();
                let options = SearchPreparationOptions {
                    case_sensitive: self.app_settings.default_case_sensitive,
                    regex: self.app_settings.default_use_regex,
                    max_results: self.app_settings.search_result_limit(),
                };
                let labels = self.color_labels.clone();
                let cancellation = state.cancellation.clone();
                return Ok(Box::new(move || {
                    doc.verify()?;
                    let complete =
                        LogDocument::open_cancellable(doc.document.path(), &cancellation)?
                            .context("Analysis stopped")?;
                    let prepared = super::document_tasks::prepare_document_in_range(
                        doc.document.path(),
                        Some(Arc::new(complete)),
                        store.as_deref(),
                        None,
                        options,
                        &labels,
                        SearchRange::default(),
                    )?;
                    doc.verify()?;
                    if !result_snapshot_matches_document(
                        doc.document.path(),
                        &doc.document,
                        &prepared.document,
                    ) {
                        bail!("Source changed; refresh references");
                    }
                    Ok(Evidence::Open(Box::new(prepared)))
                }));
            }
            "append_search" => {
                let doc = state.document(number(args, "document_id")?, args["version"].as_str())?;
                let expected = self.ai_search_draft(doc.id, cx)?;
                return Ok(Box::new(move || {
                    doc.verify()?;
                    Ok(Evidence::Json(expected))
                }));
            }
            "highlight_keyword" => {
                let doc = state.document(number(args, "document_id")?, args["version"].as_str())?;
                let tab = self
                    .documents
                    .iter()
                    .find(|tab| tab.id == doc.id)
                    .context("Open the file before highlighting")?;
                let keyword = text(args, "keyword")?.trim().to_owned();
                if keyword.is_empty() {
                    bail!("Keyword is empty");
                }
                let case_sensitive = args["case_sensitive"].as_bool().unwrap_or(false);
                let expected = tab.file.keyword_color_rules.clone();
                let mut rules = expected.clone();
                rules.retain(|r| !(r.keyword == keyword && r.case_sensitive == case_sensitive));
                let action = text(args, "action")?;
                if action == "set" {
                    let label = self
                        .color_labels
                        .iter()
                        .find(|l| Some(l.id.as_str()) == args["color_label_id"].as_str())
                        .context("Unknown color label")?;
                    rules.push(KeywordColorRule {
                        label_id: Some(label.id.clone()),
                        keyword: keyword.clone(),
                        color: label.background_color,
                        alpha: label.background_alpha,
                        case_sensitive,
                        enabled: true,
                    });
                }
                let labels = self.color_labels.clone();
                let last_color_label_id = self.last_color_label_id.clone();
                let outcome = if action == "set" {
                    ColorRuleOutcome::Applied
                } else {
                    ColorRuleOutcome::Removed
                };
                return Ok(Box::new(move || {
                    doc.verify()?;
                    let resolved = resolve_color_rules(&rules, &labels);
                    Ok(Evidence::Color(Box::new(PreparedColorRuleUpdate {
                        document_id: doc.id,
                        document: doc.document,
                        expected_rules: expected,
                        expected_labels: labels,
                        rules,
                        resolved: Some(resolved),
                        propagated_files: Vec::new(),
                        search_session: None,
                        last_color_label_id,
                        outcome,
                    })))
                }));
            }
            "text_mark" => {
                let (doc, row) = state.reference(&args["reference"])?;
                return Ok(Box::new(move || {
                    doc.verify()?;
                    let preview = doc
                        .document
                        .line_preview(row, crate::virtual_log_lines::DEFAULT_MAX_LINE_SOURCE_BYTES)
                        .context("Source unavailable")?;
                    Ok(Evidence::Json(json!({"source":preview.text()})))
                }));
            }
            "list_marks" => {
                let doc = state.document(number(args, "document_id")?, args["version"].as_str())?;
                let tab = self
                    .documents
                    .iter()
                    .find(|t| t.id == doc.id)
                    .context("Open this file to inspect marks")?;
                let start = args["start_line"].as_u64().unwrap_or(1).max(1) as usize - 1;
                let rows = tab
                    .file
                    .marked_rows
                    .iter()
                    .chain(tab.file.row_tags.source_rows())
                    .filter(|r| *r >= start)
                    .collect::<BTreeSet<_>>();
                let entries = rows.iter().take(100).map(|row| json!({"reference":doc.reference(*row),"marked":tab.file.marked_rows.contains(*row),"text_marks":tab.file.row_tags.row(*row).map(|(id,t)|json!({"id":id,"text":t.label})).collect::<Vec<_>>()})).collect::<Vec<_>>();
                json!({"marks":entries,"next_line":if rows.len()>100 {rows.iter().nth(100).map(|r|r+1)} else {None}})
            }
            _ => {
                // Validate disk evidence on the worker; validate Arc identity again at commit.
                let docs = mutation_documents(&state, call)?;
                let directory = (call.name == "show_search" && args["scope"] == "directory")
                    .then(|| state.directory.directory.clone());
                return Ok(Box::new(move || {
                    if let Some(directory) = directory {
                        approved_directory(
                            directory.as_deref().context("Select a directory first")?,
                        )?;
                    }
                    for doc in docs {
                        doc.verify()?;
                    }
                    Ok(Evidence::Json(json!({})))
                }));
            }
        };
        let value = match call.name.as_str() {
            "list_logs" => object_page(value, "files", args)?,
            "list_filters" => object_page(value, "filters", args)?,
            "list_colors" => object_page(value, "colors", args)?,
            _ => value,
        };
        Ok(Box::new(move || Ok(Evidence::Json(value))))
    }
}

impl Evidence {
    fn value(&self) -> Result<&Value> {
        match self {
            Self::Json(value) => Ok(value),
            Self::Color(_) | Self::Open(_) => bail!("Unexpected prepared evidence"),
        }
    }
    fn into_value(self) -> Result<Value> {
        match self {
            Self::Json(value) => Ok(value),
            Self::Color(_) | Self::Open(_) => bail!("Unexpected prepared evidence"),
        }
    }
}
