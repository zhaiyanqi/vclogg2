use super::super::search_tabs::{SearchTabId, SearchTabOwner, SearchTabState};
use super::*;
use crate::search_context::{PersistedSearchTab, SearchTabQuery};

impl Workspace {
    pub(super) fn cancel_ai_searches(&mut self, scope: &SharedScope, cx: &mut Context<Self>) {
        if let Ok(mut state) = scope.lock() {
            state.cancellation.cancel();
            for (owner, id, revision) in state.tabs.values_mut() {
                if self
                    .search_tabs
                    .state(*owner, *id)
                    .is_some_and(|t| t.revision == *revision)
                {
                    self.search_tabs.cancel(*owner, *id);
                    *revision = self
                        .search_tabs
                        .state(*owner, *id)
                        .map_or(*revision, |t| t.revision);
                }
            }
        }
        cx.notify();
    }
    pub(super) fn ai_commit(
        &mut self,
        scope: &SharedScope,
        call: &ToolCall,
        evidence: Evidence,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Value> {
        let mut state = scope
            .lock()
            .map_err(|_| anyhow::anyhow!("Analysis unavailable"))?;
        if state.cancellation.is_cancelled() {
            bail!("Analysis stopped");
        }
        self.validate_ai_search(&state, call)?;
        let mut checked = mutation_documents(&state, call)?;
        if matches!(call.name.as_str(), "search_logs" | "control_search")
            && let Some(id) = evidence.value()?["search_id"].as_str()
            && let Some(search) = state.searches.get(id)
        {
            checked.extend(search.groups.iter().map(|(doc, _)| doc.clone()));
        }
        for doc in checked {
            if doc.open
                && !self
                    .documents
                    .iter()
                    .any(|tab| tab.id == doc.id && Arc::ptr_eq(&tab.document, &doc.document))
            {
                bail!("File changed or closed; reacquire references");
            }
        }
        let args = &call.arguments;
        if matches!(
            call.name.as_str(),
            "open_file" | "close_file" | "switch_file" | "reveal_file"
        ) {
            return self.ai_commit_file(&mut state, call, evidence, window, cx);
        }
        match call.name.as_str() {
            "append_search" => self.ai_append_search(
                number(args, "document_id")?,
                text(args, "text")?,
                evidence.value()?,
                window,
                cx,
            ),
            "set_marks" => {
                let mut targets = BTreeMap::<u64, CompressedRows>::new();
                for reference in args["references"]
                    .as_array()
                    .context("Missing references")?
                {
                    let (doc, row) = state.reference(reference)?;
                    if !doc.open {
                        bail!("Open the directory result before marking it");
                    }
                    targets.entry(doc.id).or_default().insert(row);
                }
                let marked = args["marked"].as_bool().unwrap_or(false);
                for (id, rows) in targets {
                    self.set_document_marks(id, &rows, marked, window, cx);
                }
                Ok(
                    json!({"marked":marked,"count":args["references"].as_array().map_or(0,Vec::len),"references":args["references"]}),
                )
            }
            "highlight_keyword" => {
                let Evidence::Color(prepared) = evidence else {
                    bail!("Missing prepared highlight");
                };
                let tab = self
                    .documents
                    .iter()
                    .find(|tab| tab.id == prepared.document_id)
                    .context("File closed")?;
                if tab.file.keyword_color_rules != prepared.expected_rules
                    || self.color_labels != prepared.expected_labels
                {
                    bail!("Color settings changed; inspect and retry");
                }
                self.finish_color_rule_update(*prepared, window, cx);
                Ok(json!({"keyword":args["keyword"],"action":args["action"]}))
            }
            "text_mark" => {
                let (doc, row) = state.reference(&args["reference"])?;
                if !doc.open {
                    bail!("Open the file before adding annotations");
                }
                let id = self.apply_ai_text_mark(
                    doc.id,
                    doc.document,
                    row,
                    args,
                    evidence.value()?["source"].as_str().unwrap_or_default(),
                    window,
                    cx,
                )?;
                Ok(json!({"mark_id":id,"reference":args["reference"],"action":args["action"]}))
            }
            "navigate" => {
                let (mut doc, row, cursor) = state.navigation_target(args)?;
                if let Some(reference) = args.get("reference") {
                    let (expected, expected_row) = state.reference(reference)?;
                    if expected.id != doc.id || expected_row != row {
                        bail!("Citation does not match this result");
                    }
                }
                if let Some(search_id) = args["search_id"].as_str() {
                    let search = state.searches.get(search_id).context("Search expired")?;
                    if self.ai_select_search_hit(search, &doc, row, window, cx)? {
                        if let Some(search) = state.searches.get_mut(search_id) {
                            search.cursor = cursor;
                        }
                        return Ok(
                            json!({"reference":doc.reference(row),"url":doc.reference(row).url(),"region":"results"}),
                        );
                    }
                }
                if !doc.open {
                    if self.open_task.is_some()
                        || self
                            .documents
                            .iter()
                            .any(|tab| paths_match(tab.document.path(), doc.document.path()))
                    {
                        bail!(
                            "Another file operation changed this target; call get_context and retry"
                        );
                    }
                    let Evidence::Open(prepared) = evidence else {
                        bail!("Missing prepared file");
                    };
                    if prepared
                        .color_labels_snapshot
                        .as_ref()
                        .is_some_and(|labels| labels != &self.color_labels)
                    {
                        bail!("Color settings changed while opening; retry navigation");
                    }
                    let path = doc.document.path().to_path_buf();
                    self.install_documents(
                        vec![(path.clone(), Ok(*prepared))],
                        Some(&path),
                        &BTreeMap::new(),
                        None,
                        true,
                        window,
                        cx,
                    );
                    let tab = self
                        .documents
                        .iter()
                        .find(|tab| paths_match(tab.document.path(), &path))
                        .context("File could not be opened")?;
                    doc = DocumentSnapshot {
                        id: tab.id,
                        version: uuid::Uuid::new_v4().to_string(),
                        document: tab.document.clone(),
                        open: true,
                    };
                    state.allowed.insert(doc.id);
                    state.documents.insert(doc.id, doc.clone());
                }
                let ix = self
                    .documents
                    .iter()
                    .position(|tab| tab.id == doc.id && Arc::ptr_eq(&tab.document, &doc.document))
                    .context("Log reference is stale")?;
                self.activate_tab(ix, window, cx);
                state.current = Some(doc.id);
                if !self.activate_document_log_row_atomically(ix, row, window, cx) {
                    bail!("Could not navigate to the line");
                }
                if let Some(cursor) = cursor
                    && let Some(search) = args["search_id"]
                        .as_str()
                        .and_then(|id| state.searches.get_mut(id))
                {
                    search.cursor = Some(cursor);
                }
                self.remember_user_log_region(LogRegion::Body);
                Ok(json!({"reference":doc.reference(row)}))
            }
            "show_search" => {
                let owner = match text(args, "scope")? {
                    "current" => {
                        let id = args["document_id"]
                            .as_u64()
                            .or(state.current)
                            .context("No active file")?;
                        let doc = state.document(id, None)?;
                        if !doc.open {
                            bail!("File is not open");
                        }
                        SearchTabOwner::File(id)
                    }
                    "open" => SearchTabOwner::AllOpen,
                    _ => {
                        if state.directory.directory.is_none() {
                            bail!("Select a search directory first");
                        }
                        SearchTabOwner::Directory
                    }
                };
                let mut parameters = args.clone();
                if let Some(id) = args["filter_id"].as_str() {
                    let filter = self
                        .predefined_filters
                        .iter()
                        .find(|f| f.id.to_string() == id)
                        .context("Filter no longer exists")?;
                    parameters["query"] = json!(filter.value);
                    parameters["regex"] = json!(filter.use_regex);
                }
                let query = query(&parameters)?;
                self.capture_active_search_tab(cx);
                self.ensure_search_tab_group(owner, cx);
                let group = self
                    .search_tabs
                    .groups
                    .get_mut(&owner)
                    .context("Search group unavailable")?;
                let id = SearchTabId(group.next_id);
                group.next_id += 1;
                let saved = PersistedSearchTab {
                    id: id.0,
                    name: Some(format!(
                        "AI · {}",
                        query.text.chars().take(28).collect::<String>()
                    )),
                    draft: SearchTabQuery::from_query(&query),
                    directory: Self::persisted_directory_options(&state.directory),
                    selected_paths: state
                        .documents
                        .values()
                        .filter(|d| d.open)
                        .map(|d| encode_persisted_path(d.document.path()))
                        .collect(),
                    targets_configured: true,
                    ..Default::default()
                };
                group.tabs.push(SearchTabState::restored(saved));
                self.enqueue_search_tab(owner, id, false, window, cx);
                let revision = self.search_tabs.state(owner, id).map_or(0, |s| s.revision);
                let key = uuid::Uuid::new_v4().to_string();
                state.tabs.insert(key.clone(), (owner, id, revision));
                if let SearchTabOwner::File(target) = owner
                    && Some(target) != self.active_document().map(|tab| tab.id)
                {
                    let ix = self
                        .documents
                        .iter()
                        .position(|tab| tab.id == target)
                        .context("Target file closed")?;
                    self.activate_tab(ix, window, cx);
                }
                self.set_search_scope(owner.scope(), window, cx);
                self.commit_search_tab_activation(owner, id, window, cx);
                self.persist_search_tabs(window, cx);
                Ok(
                    json!({"search_tab":key,"status":"searching","name":"AI","document_id":if let SearchTabOwner::File(id)=owner {Some(id)} else {None}}),
                )
            }
            "control_search" => {
                let key = text(args, "search_tab")?;
                let (owner, id, revision) = *state
                    .tabs
                    .get(key)
                    .context("This is not an AI-owned search tab in this run")?;
                let tab = self
                    .search_tabs
                    .state(owner, id)
                    .context("Search tab closed")?;
                if tab.revision != revision {
                    bail!("Search tab changed since the AI request");
                }
                if args["action"] == "results" {
                    return evidence.into_value();
                }
                match text(args, "action")? {
                    "cancel" => {
                        self.search_tabs.cancel(owner, id);
                    }
                    "clear" => {
                        self.search_tabs.cancel(owner, id);
                        let tab = self
                            .search_tabs
                            .state_mut(owner, id)
                            .context("Search tab closed")?;
                        tab.saved.draft.text.clear();
                        tab.saved.completed = None;
                        tab.saved.submitted = None;
                        tab.local_result = None;
                        tab.context = SearchSessionState::default();
                        tab.needs_restore = false;
                        if self.search_tabs.installed == Some((owner, id)) {
                            self.install_search_tab(owner, id, window, cx);
                        }
                        self.persist_search_tabs(window, cx);
                    }
                    _ => {}
                }
                let tab = self
                    .search_tabs
                    .state(owner, id)
                    .context("Search tab closed")?;
                state.tabs.insert(key.into(), (owner, id, tab.revision));
                let running = self
                    .search_tabs
                    .running
                    .as_ref()
                    .is_some_and(|(o, i, _, _)| *o == owner && *i == id)
                    || self
                        .search_tabs
                        .queue
                        .iter()
                        .any(|j| j.owner == owner && j.id == id);
                Ok(
                    json!({"searching":running,"completed":tab.saved.completed.is_some(),"local_match_count":tab.local_result.as_ref().map(|(_,r,_)|r.len())}),
                )
            }
            _ => evidence.into_value(),
        }
    }
}
