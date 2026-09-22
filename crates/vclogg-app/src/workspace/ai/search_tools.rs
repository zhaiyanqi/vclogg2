//! Preparation for search; disk work stays on the worker.
use super::*;

impl Workspace {
    pub(super) fn ai_prepare_search(
        &self,
        scope: SharedScope,
        call: &ToolCall,
        cx: &App,
    ) -> Result<Work> {
        let state = scope
            .lock()
            .map_err(|_| anyhow::anyhow!("Analysis state unavailable"))?;
        let args = &call.arguments;
        let value = match call.name.as_str() {
            "list_filters" => {
                json!({"filters":self.predefined_filters.iter().map(|f| json!({"id":f.id.to_string(),"name":f.name,"query":f.value,"regex":f.use_regex})).collect::<Vec<_>>()})
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
            "append_search" => {
                let doc = state.document(number(args, "document_id")?, args["version"].as_str())?;
                let expected = self.ai_search_draft(doc.id, cx)?;
                return Ok(Box::new(move || {
                    doc.verify()?;
                    Ok(Evidence::Json(expected))
                }));
            }
            _ => {
                drop(state);
                return self.ai_prepare_state_change(scope, call);
            }
        };
        let value = match call.name.as_str() {
            "list_filters" => object_page(value, "filters", args)?,
            _ => value,
        };
        Ok(Box::new(move || Ok(Evidence::Json(value))))
    }
}
