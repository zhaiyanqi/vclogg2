//! Preparation for logs; disk work stays on the worker.
use super::*;

impl Workspace {
    pub(super) fn ai_prepare_logs(
        &self,
        scope: SharedScope,
        call: &ToolCall,
        _cx: &App,
    ) -> Result<Work> {
        let state = scope
            .lock()
            .map_err(|_| anyhow::anyhow!("Analysis state unavailable"))?;
        let args = &call.arguments;
        let value = match call.name.as_str() {
            "list_logs" => {
                json!({"files":state.documents.values().map(|d| Ok(json!({"document_id":d.id,"version":d.version,"name":d.document.file_name(),"path":d.absolute_path()?.display().to_string(),"lines":d.document.source_line_count(),"open":d.open}))).collect::<Result<Vec<_>>>()?})
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
            _ => bail!("Unexpected log tool"),
        };
        let value = match call.name.as_str() {
            "list_logs" => object_page(value, "files", args)?,
            _ => value,
        };
        Ok(Box::new(move || Ok(Evidence::Json(value))))
    }
}
