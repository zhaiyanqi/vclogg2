use super::*;

pub(super) fn number(value: &Value, key: &str) -> Result<u64> {
    value[key]
        .as_u64()
        .with_context(|| format!("Missing {key}"))
}
pub(super) fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value[key]
        .as_str()
        .with_context(|| format!("Missing {key}"))
}
pub(super) fn query(value: &Value) -> Result<SearchQuery> {
    let query = SearchQuery {
        text: text(value, "query")?.into(),
        case_sensitive: value["case_sensitive"].as_bool().unwrap_or(false),
        regex: value["regex"].as_bool().unwrap_or(false),
        max_results: Some(100_000),
    };
    if query.text.trim().is_empty() {
        bail!("Search query is empty");
    }
    SearchMatcher::new(&query)?;
    Ok(query)
}
pub(super) fn log_row(doc: &DocumentSnapshot, row: usize) -> Result<Value> {
    log_excerpt(doc, row, 2048, None)
}
pub(super) fn log_excerpt(
    doc: &DocumentSnapshot,
    row: usize,
    max_chars: usize,
    matcher: Option<&SearchMatcher>,
) -> Result<Value> {
    let preview = doc
        .document
        .line_preview(row, 8192)
        .context("Source line unavailable")?;
    let source = preview.text();
    let matched = matcher.and_then(|m| m.matching_ranges(source).first().cloned());
    let center = matched
        .as_ref()
        .map_or(0, |r| source[..r.start].chars().count());
    let start = center.saturating_sub(max_chars / 3);
    let excerpt = source
        .chars()
        .skip(start)
        .take(max_chars)
        .collect::<String>();
    let truncated = preview.is_truncated() || start > 0 || excerpt.len() < source.len();
    Ok(
        json!({"reference":doc.reference(row),"url":doc.reference(row).url(),"file":doc.document.file_name(),"text":excerpt,"truncated":truncated,"excerpt_start_character":start,"match_in_excerpt":matched.is_some()}),
    )
}
#[cfg(test)]
pub(super) fn read_page(doc: &DocumentSnapshot, start: usize, limit: usize) -> Result<Value> {
    read_page_cancellable(doc, start, limit, &SearchCancellation::default())
}
pub(super) fn read_page_cancellable(
    doc: &DocumentSnapshot,
    start: usize,
    limit: usize,
    cancellation: &SearchCancellation,
) -> Result<Value> {
    doc.verify()?;
    if start >= doc.document.source_line_count() {
        bail!("Start line exceeds the file");
    }
    let mut rows = Vec::new();
    let mut next = start;
    while next < doc.document.source_line_count() && rows.len() < limit.min(100) {
        if cancellation.is_cancelled() {
            bail!("Read cancelled");
        }
        rows.push(log_row(doc, next)?);
        if serde_json::to_vec(&rows)?.len() > 16 * 1024 {
            rows.pop();
            break;
        }
        next += 1;
    }
    Ok(json!({"rows":rows,"next_line":(next<doc.document.source_line_count()).then_some(next+1)}))
}
pub(super) fn search_page(search: &SearchSnapshot, offset: usize) -> Result<Value> {
    search_page_options(search, offset, 20)
}
pub(super) fn search_page_options(
    search: &SearchSnapshot,
    offset: usize,
    limit: usize,
) -> Result<Value> {
    let total = search
        .groups
        .iter()
        .map(|(_, rows)| rows.len())
        .sum::<usize>();
    let mut skipped = 0;
    let mut rows = Vec::new();
    'groups: for (doc, matches) in &search.groups {
        if skipped + matches.len() <= offset {
            skipped += matches.len();
            continue;
        }
        doc.verify()?;
        for row in matches.iter().skip(offset.saturating_sub(skipped)) {
            rows.push(json!({"reference":doc.reference(row),"url":doc.reference(row).url(),"file":doc.document.file_name()}));
            if serde_json::to_vec(&rows)?.len() > 12 * 1024 {
                rows.pop();
                break 'groups;
            }
            if rows.len() == limit.clamp(1, 40) {
                break 'groups;
            }
        }
        skipped += matches.len();
    }
    Ok(
        json!({"total":total,"truncated":search.truncated,"representation":"references","content_included":false,"query":search.query.as_ref().map(|q| &q.text),"next_offset":(offset.saturating_add(rows.len())<total).then_some(offset.saturating_add(rows.len())),"rows":rows}),
    )
}
pub(super) fn search_logs(
    scope: SharedScope,
    mut docs: Vec<DocumentSnapshot>,
    directory: Option<DirectorySearchOptions>,
    query: SearchQuery,
    cancellation: SearchCancellation,
) -> Result<Value> {
    let mut errors = 0usize;
    let mut search = SearchSnapshot {
        groups: Vec::new(),
        query: Some(query.clone()),
        cursor: None,
        truncated: false,
        tab: None,
    };
    if let Some(options) = directory {
        let root = approved_directory(
            options
                .directory
                .as_deref()
                .context("Select a directory in the app first")?,
        )?;
        let enumeration = crate::directory_search_dialog::enumerate_directory_search_paths(
            &options,
            &cancellation,
        )?
        .context("Search cancelled")?;
        docs.clear();
        for path in enumeration.paths {
            if cancellation.is_cancelled() {
                bail!("Search cancelled");
            }
            let Ok(path) = path.canonicalize() else {
                errors += 1;
                continue;
            };
            if !path.starts_with(&root) {
                continue;
            }
            let existing = scope
                .lock()
                .map_err(|_| anyhow::anyhow!("Analysis unavailable"))?
                .documents
                .values()
                .find(|d| paths_match(d.document.path(), &path))
                .cloned();
            if let Some(doc) = existing {
                append_search(&mut search, doc, &query, &cancellation)?;
                continue;
            }
            match LogDocument::open_cancellable(&path, &cancellation) {
                Ok(Some(document)) => {
                    append_search(
                        &mut search,
                        DocumentSnapshot {
                            id: next_directory_id(),
                            version: uuid::Uuid::new_v4().to_string(),
                            document: Arc::new(document),
                            open: false,
                        },
                        &query,
                        &cancellation,
                    )?;
                }
                Ok(None) => bail!("Search cancelled"),
                Err(_) => errors += 1,
            }
        }
    }
    for doc in docs {
        append_search(&mut search, doc, &query, &cancellation)?;
    }
    let mut page = search_page(&search, 0)?;
    let id = uuid::Uuid::new_v4().to_string();
    add_search_links(&mut page, &id, 0);
    page["unreadable_files"] = json!(errors);
    let mut state = scope
        .lock()
        .map_err(|_| anyhow::anyhow!("Analysis unavailable"))?;
    if state.cancellation.is_cancelled() {
        bail!("Search cancelled");
    }
    for (doc, _) in &search.groups {
        state.documents.insert(doc.id, doc.clone());
    }
    state.searches.insert(id, search);
    Ok(page)
}
pub(super) fn mutation_documents(
    state: &AiScope,
    call: &ToolCall,
) -> Result<Vec<DocumentSnapshot>> {
    let args = &call.arguments;
    if let Some(refs) = args["references"].as_array() {
        return refs
            .iter()
            .map(|r| state.reference(r).map(|(d, _)| d))
            .collect();
    }
    if args.get("reference").is_some() {
        return Ok(vec![state.reference(&args["reference"])?.0]);
    }
    if let Some(id) = args["document_id"].as_u64() {
        return Ok(vec![state.document(id, args["version"].as_str())?]);
    }
    if matches!(
        call.name.as_str(),
        "navigate" | "search_results" | "summarize_search"
    ) && let Some(id) = args["search_id"].as_str()
    {
        return Ok(state
            .searches
            .get(id)
            .context("Search unavailable")?
            .groups
            .iter()
            .map(|(d, _)| d.clone())
            .collect());
    }
    Ok(Vec::new())
}

pub(super) fn next_directory_id() -> u64 {
    static IDS: AtomicU64 = AtomicU64::new(1 << 62);
    IDS.fetch_add(1, Ordering::Relaxed)
}
pub(super) fn object_page(mut value: Value, field: &str, args: &Value) -> Result<Value> {
    let all = value[field].take().as_array().cloned().unwrap_or_default();
    let offset = args["offset"].as_u64().unwrap_or(0) as usize;
    let mut page = Vec::new();
    for item in all.iter().skip(offset).take(100) {
        page.push(item.clone());
        if serde_json::to_vec(&page)?.len() > 48 * 1024 {
            page.pop();
            break;
        }
    }
    if page.is_empty() && offset < all.len() {
        bail!("An entry exceeds the result size limit");
    }
    value["next_offset"] = json!((offset + page.len() < all.len()).then_some(offset + page.len()));
    value["total"] = json!(all.len());
    value[field] = json!(page);
    Ok(value)
}

/// Reuse one completed index per run only when a sparse result lacks the requested
/// neighbors. Direct hits use their existing offsets and never rebuild an index.
pub(super) fn read_snapshot(
    scope: &SharedScope,
    doc: DocumentSnapshot,
    start: usize,
    end: usize,
    cancellation: &SearchCancellation,
) -> Result<DocumentSnapshot> {
    doc.verify()?;
    if (start..end.min(doc.document.source_line_count()))
        .all(|row| doc.document.contains_source_row(row))
    {
        return Ok(doc);
    }
    let cached = scope
        .lock()
        .map_err(|_| anyhow::anyhow!("Analysis unavailable"))?
        .read_document
        .clone();
    if let Some(cached) = cached
        && cached.id == doc.id
        && cached.version == doc.version
    {
        cached.verify()?;
        return Ok(cached);
    }
    let complete = complete_snapshot(doc, cancellation)?;
    let mut state = scope
        .lock()
        .map_err(|_| anyhow::anyhow!("Analysis unavailable"))?;
    if state.cancellation.is_cancelled() {
        bail!("Read cancelled");
    }
    state.read_document = Some(complete.clone());
    Ok(complete)
}

pub(super) fn complete_snapshot(
    mut doc: DocumentSnapshot,
    cancellation: &SearchCancellation,
) -> Result<DocumentSnapshot> {
    if !doc.document.has_complete_line_index() {
        let complete = LogDocument::open_cancellable(doc.document.path(), cancellation)?
            .context("Read cancelled")?;
        doc.verify()?;
        if !doc.document.same_source_snapshot(&complete) {
            bail!("Source changed; refresh references");
        }
        doc.document = Arc::new(complete);
    }
    Ok(doc)
}
fn append_search(
    search: &mut SearchSnapshot,
    doc: DocumentSnapshot,
    query: &SearchQuery,
    cancellation: &SearchCancellation,
) -> Result<()> {
    let mut doc = complete_snapshot(doc, cancellation)?;
    doc.verify()?;
    match vclogg_core::search_cancellable(&doc.document, query, cancellation)? {
        vclogg_core::SearchRun::Completed(result) => {
            search.truncated |= result.truncated;
            if !result.is_empty() {
                if !doc.open {
                    doc.document = Arc::new(doc.document.project_source_rows(&result.line_indices));
                    doc.document.release_source_handle();
                }
                search.groups.push((doc, result.line_indices));
            }
            Ok(())
        }
        vclogg_core::SearchRun::Cancelled => bail!("Search cancelled"),
        vclogg_core::SearchRun::SourceChanged => bail!("Log changed during search"),
    }
}

pub(super) fn approved_directory(path: &Path) -> Result<PathBuf> {
    let root = path.canonicalize()?;
    if root != path {
        bail!("Selected directory changed; start a new analysis to capture it again");
    }
    Ok(root)
}

pub(super) fn add_search_links(page: &mut Value, search_id: &str, offset: usize) {
    page["search_id"] = json!(search_id);
    if let Some(rows) = page["rows"].as_array_mut() {
        for (ix, row) in rows.iter_mut().enumerate() {
            let index = offset + ix + 1;
            if let Some(mut url) = row["url"].as_str().and_then(|s| url::Url::parse(s).ok()) {
                url.query_pairs_mut()
                    .append_pair("search_id", search_id)
                    .append_pair("result_index", &index.to_string());
                row["url"] = json!(url.as_str());
                row["result_index"] = json!(index);
            }
        }
    }
}
