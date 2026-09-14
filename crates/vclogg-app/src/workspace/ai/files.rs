use super::*;

impl AiScope {
    fn file_target(&self, args: &Value) -> Result<(PathBuf, Option<DocumentSnapshot>)> {
        match (args["document_id"].as_u64(), args["file_id"].as_str()) {
            (Some(id), None) => {
                let doc = self.document(id, Some(text(args, "version")?))?;
                Ok((doc.document.path().to_path_buf(), Some(doc)))
            }
            (None, Some(id)) if args.get("version").is_none() => Ok((
                self.file_candidates
                    .get(id)
                    .context("File handle expired; call locate_files again")?
                    .clone(),
                None,
            )),
            _ => bail!("Provide document_id with version, or file_id, exclusively"),
        }
    }

    pub(super) fn forget_file(&mut self, path: &Path) {
        if self
            .read_document
            .as_ref()
            .is_some_and(|doc| paths_match(doc.document.path(), path))
        {
            self.read_document = None;
        }
        self.documents
            .retain(|_, doc| !paths_match(doc.document.path(), path));
        self.searches.retain(|_, search| {
            !search
                .groups
                .iter()
                .any(|(doc, _)| paths_match(doc.document.path(), path))
        });
    }
}

impl Workspace {
    pub(super) fn ai_prepare_file(&self, scope: SharedScope, call: &ToolCall) -> Result<Work> {
        let state = scope
            .lock()
            .map_err(|_| anyhow::anyhow!("Analysis unavailable"))?;
        let args = &call.arguments;
        if call.name == "locate_files" {
            let options = state.directory.clone();
            let cancellation = state.cancellation.clone();
            let query = text(args, "query")?.trim().to_lowercase();
            if query.is_empty() {
                bail!("Specify a filename or path fragment");
            }
            let offset = args["offset"].as_u64().unwrap_or(0) as usize;
            drop(state);
            return Ok(Box::new(move || {
                locate_files(scope, options, &query, offset, cancellation).map(Evidence::Json)
            }));
        }
        let (path, doc) = state.file_target(args)?;
        let doc = doc.or_else(|| {
            state
                .documents
                .values()
                .find(|doc| doc.open && paths_match(doc.document.path(), &path))
                .cloned()
        });
        let directory = state.directory.directory.clone();
        let cancellation = state.cancellation.clone();
        let reveal = call.name == "reveal_file";
        let store = self.persistence.store.clone();
        let options = SearchPreparationOptions {
            case_sensitive: self.app_settings.default_case_sensitive,
            regex: self.app_settings.default_use_regex,
            max_results: self.app_settings.search_result_limit(),
        };
        let labels = self.color_labels.clone();
        Ok(Box::new(move || {
            if let Some(doc) = &doc {
                doc.verify()?;
            } else {
                let root =
                    approved_directory(directory.as_deref().context("Select a directory first")?)?;
                if path.canonicalize()? != path || !path.starts_with(root) || !path.is_file() {
                    bail!("File moved or escaped the captured directory; locate it again");
                }
            }
            if cancellation.is_cancelled() {
                bail!("Analysis stopped");
            }
            if reveal || doc.as_ref().is_some_and(|d| d.open) {
                return Ok(Evidence::Json(json!({})));
            }
            let complete =
                LogDocument::open_cancellable(&path, &cancellation)?.context("Analysis stopped")?;
            let prepared = super::super::document_tasks::prepare_document_in_range(
                &path,
                Some(Arc::new(complete)),
                store.as_deref(),
                None,
                options,
                &labels,
                SearchRange::default(),
            )?;
            if let Some(doc) = &doc {
                doc.verify()?;
                if !result_snapshot_matches_document(&path, &doc.document, &prepared.document) {
                    bail!("Source changed; refresh references");
                }
            }
            if cancellation.is_cancelled() {
                bail!("Analysis stopped");
            }
            Ok(Evidence::Open(Box::new(prepared)))
        }))
    }

    pub(super) fn ai_commit_file(
        &mut self,
        state: &mut AiScope,
        call: &ToolCall,
        evidence: Evidence,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Value> {
        let args = &call.arguments;
        if call.name == "reveal_file" {
            let (path, _) = state.file_target(args)?;
            self.reveal_in_sidebar(path.clone(), false, window, cx);
            return Ok(json!({"status":"revealed","path":path.display().to_string()}));
        }
        if call.name == "open_file" {
            let (path, expected) = state.file_target(args)?;
            if let Some(tab) = self
                .documents
                .iter()
                .find(|t| paths_match(t.document.path(), &path))
            {
                if tab.load_state != DocumentLoadState::Ready {
                    bail!("File is still loading; refresh list_logs before retrying");
                }
                if expected.as_ref().is_some_and(|d| {
                    !result_snapshot_matches_document(&path, &d.document, &tab.document)
                }) {
                    bail!("File changed; refresh references");
                }
                if let Evidence::Open(prepared) = &evidence
                    && !result_snapshot_matches_document(&path, &prepared.document, &tab.document)
                {
                    bail!("File changed while opening; locate it again");
                }
            } else {
                if self.open_task.is_some() {
                    bail!("Another file operation is in progress; retry after completion");
                }
                let Evidence::Open(prepared) = evidence else {
                    bail!("File closed while preparing; refresh references");
                };
                if prepared
                    .color_labels_snapshot
                    .as_ref()
                    .is_some_and(|labels| labels != &self.color_labels)
                {
                    bail!("Color settings changed while opening; retry");
                }
                self.install_documents(
                    vec![(path.clone(), Ok(*prepared))],
                    Some(&path),
                    &BTreeMap::new(),
                    None,
                    true,
                    window,
                    cx,
                );
            }
            let ix = self
                .documents
                .iter()
                .position(|t| {
                    paths_match(t.document.path(), &path)
                        && t.load_state == DocumentLoadState::Ready
                })
                .context("File could not be opened")?;
            let tab = &self.documents[ix];
            let doc = state
                .documents
                .get(&tab.id)
                .filter(|d| Arc::ptr_eq(&d.document, &tab.document))
                .cloned()
                .unwrap_or_else(|| DocumentSnapshot {
                    id: tab.id,
                    version: uuid::Uuid::new_v4().to_string(),
                    document: tab.document.clone(),
                    open: true,
                });
            state.allowed.insert(doc.id);
            state.documents.insert(doc.id, doc.clone());
            state.current = Some(doc.id);
            self.activate_tab(ix, window, cx);
            return Ok(file_metadata(&doc, "opened"));
        }
        let doc = state.document(number(args, "document_id")?, Some(text(args, "version")?))?;
        let ix = self
            .documents
            .iter()
            .position(|t| t.id == doc.id && Arc::ptr_eq(&t.document, &doc.document))
            .context("Target file is not open")?;
        if call.name == "switch_file" {
            self.activate_tab(ix, window, cx);
            state.current = Some(doc.id);
            return Ok(file_metadata(&doc, "active"));
        }
        self.request_close_workspace_tabs(
            BTreeSet::from([WorkspaceTabId::Document(doc.id)]),
            window,
            cx,
        );
        if self.documents.iter().any(|t| t.id == doc.id) {
            Ok(file_metadata(&doc, "confirmation_pending"))
        } else {
            state.forget_file(doc.document.path());
            state.current = self
                .active_document()
                .map(|t| t.id)
                .filter(|id| state.documents.contains_key(id));
            Ok(json!({"document_id":doc.id,"status":"closed"}))
        }
    }
}

fn file_metadata(doc: &DocumentSnapshot, status: &str) -> Value {
    json!({"document_id":doc.id,"version":doc.version,"name":doc.document.file_name(),"path":doc.document.path().display().to_string(),"lines":doc.document.source_line_count(),"open":doc.open,"status":status,"content_included":false})
}

fn locate_files(
    scope: SharedScope,
    options: DirectorySearchOptions,
    query: &str,
    offset: usize,
    cancellation: SearchCancellation,
) -> Result<Value> {
    let root = approved_directory(
        options
            .directory
            .as_deref()
            .context("Select a search directory in the app first")?,
    )?;
    let enumeration =
        crate::directory_search_dialog::enumerate_directory_search_paths(&options, &cancellation)?
            .context("File lookup cancelled")?;
    let mut paths = Vec::new();
    for path in enumeration.paths {
        if cancellation.is_cancelled() {
            bail!("File lookup cancelled");
        }
        // Compare the relative path so a matching parent directory does not select every file.
        if !path
            .strip_prefix(&root)
            .unwrap_or(&path)
            .to_string_lossy()
            .to_lowercase()
            .contains(query)
        {
            continue;
        }
        if let Ok(path) = path.canonicalize()
            && path.starts_with(&root)
            && path.is_file()
        {
            paths.push(path);
        }
    }
    paths.sort();
    paths.dedup();
    let mut state = scope
        .lock()
        .map_err(|_| anyhow::anyhow!("Analysis unavailable"))?;
    if state.cancellation.is_cancelled() {
        bail!("File lookup cancelled");
    }
    let mut files = Vec::new();
    for path in paths.iter().skip(offset).take(40) {
        let id = state
            .file_candidates
            .iter()
            .find(|(_, known)| *known == path)
            .map(|(id, _)| id.clone())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let doc = state
            .documents
            .values()
            .find(|d| paths_match(d.document.path(), path));
        let file = json!({"file_id":id,"name":path.file_name().unwrap_or_default().to_string_lossy(),"path":path.display().to_string(),"document_id":doc.map(|d|d.id),"version":doc.map(|d|&d.version),"open":doc.is_some_and(|d|d.open)});
        files.push(file);
        if serde_json::to_vec(&files)?.len() > 32 * 1024 {
            files.pop();
            break;
        }
        if state.file_candidates.len() >= 2000 && !state.file_candidates.contains_key(&id) {
            bail!(
                "Too many file handles in this run; narrow the filename query or start a new analysis"
            );
        }
        state.file_candidates.insert(id, path.clone());
    }
    let next = offset.saturating_add(files.len());
    Ok(
        json!({"files":files,"total":paths.len(),"next_offset":(next<paths.len()).then_some(next),"unreadable_directories":enumeration.unreadable_directory_count,"content_included":false}),
    )
}
