use super::*;

impl AiScope {
    fn file_target(
        &self,
        args: &Value,
    ) -> Result<(PathBuf, Option<DocumentSnapshot>, Option<PathBuf>)> {
        match (
            args["document_id"].as_u64(),
            args["file_id"].as_str(),
            args["root"].as_u64(),
            args["path"].as_str(),
        ) {
            (Some(id), None, None, None) => {
                let doc = self.document(id, Some(text(args, "version")?))?;
                Ok((doc.document.path().to_path_buf(), Some(doc), None))
            }
            (None, Some(id), None, None) if args.get("version").is_none() => {
                let root = self
                    .directory
                    .directory
                    .clone()
                    .context("Selected log directory is unavailable")?;
                Ok((
                    self.file_candidates
                        .get(id)
                        .context("File handle expired; call locate_files again")?
                        .clone(),
                    None,
                    Some(root),
                ))
            }
            (None, None, Some(root), Some(relative)) if args.get("version").is_none() => {
                let root = self.workspace_directories.get(root as usize).context(
                    "Workspace root unavailable; check the selected project directories",
                )?;
                let path = scoped_workspace_file(root, relative)?;
                Ok((path, None, Some(root.clone())))
            }
            _ => bail!(
                "Provide document_id with version, file_id, or project root with path, exclusively"
            ),
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
        if call.name == "add_source_workspace" {
            let requested = text(args, "path")?.to_owned();
            ensure_requested_source_directory(&state.source_directory_request, &requested)?;
            drop(state);
            return Ok(Box::new(move || {
                let path = canonical_source_directory(&requested)?;
                Ok(Evidence::Json(json!({"path":path.display().to_string()})))
            }));
        }
        if call.name == "list_log_directory" {
            let options = state.directory.clone();
            let cancellation = state.cancellation.clone();
            let path = args["path"].as_str().unwrap_or("").to_owned();
            let depth = args["depth"].as_u64().unwrap_or(3) as usize;
            let offset = args["offset"].as_u64().unwrap_or(0) as usize;
            let limit = args["limit"].as_u64().unwrap_or(100) as usize;
            drop(state);
            return Ok(Box::new(move || {
                list_log_directory(scope, options, &path, depth, offset, limit, cancellation)
                    .map(Evidence::Json)
            }));
        }
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
        let (path, doc, access_root) = state.file_target(args)?;
        let doc = doc.or_else(|| {
            state
                .documents
                .values()
                .find(|doc| doc.open && paths_match(doc.document.path(), &path))
                .cloned()
        });
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
                let root = approved_directory(
                    access_root
                        .as_deref()
                        .context("Captured file directory is unavailable")?,
                )?;
                if path.canonicalize()? != path || !path.starts_with(&root) || !path.is_file() {
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
        if call.name == "add_source_workspace" {
            let requested = text(args, "path")?;
            ensure_requested_source_directory(&state.source_directory_request, requested)?;
            let prepared_path = PathBuf::from(
                evidence.value()?["path"]
                    .as_str()
                    .context("Prepared workspace path is missing")?,
            );
            let path = canonical_source_directory(requested)?;
            if prepared_path != path {
                bail!("Requested project directory changed while it was being prepared");
            }
            let root = state
                .workspace_directories
                .iter()
                .position(|existing| existing == &path)
                .unwrap_or_else(|| {
                    state.workspace_directories.push(path.clone());
                    state.workspace_directories.len() - 1
                });
            return Ok(json!({
                "root": root,
                "path": path.display().to_string(),
                "access": "read_only",
                "persistence": "this_run"
            }));
        }
        if call.name == "reveal_file" {
            let (path, _, _) = state.file_target(args)?;
            self.reveal_in_sidebar(path.clone(), false, window, cx);
            return Ok(json!({"status":"revealed","path":path.display().to_string()}));
        }
        if call.name == "open_file" {
            let (path, expected, _) = state.file_target(args)?;
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

fn scoped_workspace_file(root: &Path, relative: &str) -> Result<PathBuf> {
    let relative = Path::new(relative);
    if relative.as_os_str().is_empty()
        || relative.is_absolute()
        || relative.components().any(|part| {
            !matches!(
                part,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )
        })
    {
        bail!("Use a relative file path within the selected project directory");
    }
    let root = approved_directory(root)?;
    let path = root
        .join(relative)
        .canonicalize()
        .context("Project file unavailable")?;
    if !path.starts_with(&root) || !path.is_file() {
        bail!("Project path is not a file within the selected project directory");
    }
    Ok(path)
}

fn ensure_requested_source_directory(request: &str, path: &str) -> Result<()> {
    let mentioned = request.match_indices(path).any(|(start, _)| {
        request[start + path.len()..]
            .chars()
            .next()
            .is_none_or(|next| {
                !next.is_ascii_alphanumeric() && !matches!(next, '/' | '\\' | '.' | '_' | '-' | '~')
            })
    });
    if path.trim().is_empty() || !Path::new(path).is_absolute() || !mentioned {
        bail!("Use an absolute project directory explicitly written in the current user request");
    }
    Ok(())
}

fn canonical_source_directory(path: &str) -> Result<PathBuf> {
    let path = Path::new(path)
        .canonicalize()
        .context("Requested project directory is unavailable")?;
    if !path.is_dir() || path.parent().is_none() {
        bail!("Requested project path must be a non-root directory");
    }
    Ok(path)
}

fn list_log_directory(
    scope: SharedScope,
    options: DirectorySearchOptions,
    relative: &str,
    max_depth: usize,
    offset: usize,
    limit: usize,
    cancellation: SearchCancellation,
) -> Result<Value> {
    let root = approved_directory(
        options
            .directory
            .as_deref()
            .context("Select a search directory in the app first")?,
    )?;
    let relative_path = Path::new(relative);
    if relative_path.is_absolute()
        || relative_path.components().any(|part| {
            !matches!(
                part,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )
        })
    {
        bail!("Use a relative directory path within the selected log directory");
    }
    let subtree = root
        .join(relative_path)
        .canonicalize()
        .context("Log directory path unavailable")?;
    if !subtree.starts_with(&root) || !subtree.is_dir() {
        bail!("Log directory path is outside the selected directory or is not a directory");
    }
    let enumeration =
        crate::directory_search_dialog::enumerate_directory_search_paths(&options, &cancellation)?
            .context("Log directory listing cancelled")?;
    let mut nodes = BTreeMap::<PathBuf, (bool, bool, PathBuf)>::new();
    for path in enumeration.paths {
        if cancellation.is_cancelled() {
            bail!("Log directory listing cancelled");
        }
        let Ok(path) = path.canonicalize() else {
            continue;
        };
        if !path.starts_with(&subtree) || !path.starts_with(&root) || !path.is_file() {
            continue;
        }
        let within_subtree = path.strip_prefix(&subtree)?;
        let components = within_subtree.components().collect::<Vec<_>>();
        if components.is_empty() {
            continue;
        }
        let directory_count = components.len().saturating_sub(1);
        for directory_depth in 1..=directory_count.min(max_depth) {
            let mut directory = subtree.clone();
            for component in components.iter().take(directory_depth) {
                directory.push(component.as_os_str());
            }
            let relative_to_root = directory.strip_prefix(&root)?.to_path_buf();
            let has_more = directory_depth == max_depth && directory_count >= max_depth;
            nodes
                .entry(relative_to_root)
                .and_modify(|node| node.1 |= has_more)
                .or_insert((false, has_more, directory));
        }
        if components.len() <= max_depth {
            nodes.insert(path.strip_prefix(&root)?.to_path_buf(), (true, false, path));
        }
    }
    let total = nodes.len();
    let page = nodes
        .into_iter()
        .skip(offset)
        .take(limit)
        .collect::<Vec<_>>();
    let mut state = scope
        .lock()
        .map_err(|_| anyhow::anyhow!("Analysis unavailable"))?;
    if state.cancellation.is_cancelled() {
        bail!("Log directory listing cancelled");
    }
    let mut entries = Vec::new();
    for (relative_path, (is_file, has_more, absolute_path)) in page {
        let name = relative_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        let depth = absolute_path
            .strip_prefix(&subtree)
            .map(|path| path.components().count())
            .unwrap_or(0);
        if is_file {
            let id = state
                .file_candidates
                .iter()
                .find(|(_, known)| *known == &absolute_path)
                .map(|(id, _)| id.clone())
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            if state.file_candidates.len() >= 2000 && !state.file_candidates.contains_key(&id) {
                bail!(
                    "Too many file handles in this run; narrow the directory subtree or start a new analysis"
                );
            }
            state
                .file_candidates
                .insert(id.clone(), absolute_path.clone());
            let doc = state
                .documents
                .values()
                .find(|doc| paths_match(doc.document.path(), &absolute_path));
            let metadata = absolute_path.metadata().ok();
            let modified_ms = metadata
                .as_ref()
                .and_then(|metadata| metadata.modified().ok())
                .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|duration| duration.as_millis() as u64);
            entries.push(json!({
                "kind": "file",
                "name": name,
                "path": relative_path,
                "depth": depth,
                "file_id": id,
                "size_bytes": metadata.map(|metadata| metadata.len()),
                "modified_unix_ms": modified_ms,
                "document_id": doc.map(|doc| doc.id),
                "version": doc.map(|doc| &doc.version),
                "open": doc.is_some_and(|doc| doc.open)
            }));
        } else {
            entries.push(json!({
                "kind": "directory",
                "name": name,
                "path": relative_path,
                "depth": depth,
                "has_more": has_more
            }));
        }
    }
    let next = offset.saturating_add(entries.len());
    Ok(json!({
        "root": root,
        "path": relative,
        "entries": entries,
        "total": total,
        "next_offset": (next < total).then_some(next),
        "unreadable_directories": enumeration.unreadable_directory_count,
        "filters": {
            "include_subdirectories": options.include_subdirectories,
            "include_hidden_directories": options.include_hidden_directories,
            "file_type_filter_enabled": options.file_type_filter_enabled,
            "file_type_patterns": options.file_type_patterns
        },
        "content_included": false
    }))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_scope(options: DirectorySearchOptions) -> SharedScope {
        Arc::new(Mutex::new(AiScope {
            file_candidates: BTreeMap::new(),
            workspace_directories: Vec::new(),
            source_directory_request: String::new(),
            read_document: None,
            explicit: BTreeSet::new(),
            allowed: BTreeSet::new(),
            current: None,
            documents: BTreeMap::new(),
            directory: options,
            searches: BTreeMap::new(),
            tabs: BTreeMap::new(),
            cancellation: SearchCancellation::default(),
        }))
    }

    #[test]
    fn log_directory_tree_is_filtered_bounded_and_returns_file_handles() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(directory.path().join("service/deep")).unwrap();
        std::fs::write(directory.path().join("root.log"), "root\n").unwrap();
        std::fs::write(directory.path().join("service/app.log"), "app\n").unwrap();
        std::fs::write(directory.path().join("service/deep/worker.log"), "worker\n").unwrap();
        std::fs::write(directory.path().join("service/ignored.bin"), [0_u8]).unwrap();
        let options = DirectorySearchOptions {
            directory: Some(directory.path().canonicalize().unwrap()),
            ..Default::default()
        };
        let scope = empty_scope(options.clone());

        let root = list_log_directory(
            scope.clone(),
            options.clone(),
            "",
            1,
            0,
            100,
            SearchCancellation::default(),
        )
        .unwrap();
        let entries = root["entries"].as_array().unwrap();
        assert!(
            entries
                .iter()
                .any(|entry| entry["path"] == "root.log" && entry["kind"] == "file")
        );
        assert!(entries.iter().any(|entry| {
            entry["path"] == "service" && entry["kind"] == "directory" && entry["has_more"] == true
        }));
        assert!(
            !entries
                .iter()
                .any(|entry| entry["path"] == "service/ignored.bin")
        );

        let subtree = list_log_directory(
            scope.clone(),
            options,
            "service",
            2,
            0,
            100,
            SearchCancellation::default(),
        )
        .unwrap();
        let files = subtree["entries"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|entry| entry["kind"] == "file")
            .collect::<Vec<_>>();
        assert_eq!(files.len(), 2);
        assert!(files.iter().all(|entry| entry["file_id"].is_string()));
        assert_eq!(scope.lock().unwrap().file_candidates.len(), 3);

        assert!(
            list_log_directory(
                scope,
                DirectorySearchOptions {
                    directory: Some(directory.path().canonicalize().unwrap()),
                    ..Default::default()
                },
                "../outside",
                2,
                0,
                100,
                SearchCancellation::default(),
            )
            .is_err()
        );
    }

    #[test]
    fn project_file_targets_stay_within_the_captured_workspace() {
        let workspace = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(workspace.path().join("src")).unwrap();
        std::fs::write(workspace.path().join("src/service.rs"), "fn service() {}\n").unwrap();
        let root = workspace.path().canonicalize().unwrap();
        let scope = empty_scope(DirectorySearchOptions::default());
        scope.lock().unwrap().workspace_directories = vec![root.clone()];

        let (path, document, access_root) = scope
            .lock()
            .unwrap()
            .file_target(&json!({"root":0,"path":"src/service.rs"}))
            .unwrap();
        assert_eq!(path, root.join("src/service.rs"));
        assert!(document.is_none());
        assert_eq!(access_root.as_deref(), Some(root.as_path()));

        assert!(
            scope
                .lock()
                .unwrap()
                .file_target(&json!({"root":0,"path":"../outside.log"}))
                .is_err()
        );
        assert!(
            scope
                .lock()
                .unwrap()
                .file_target(&json!({"root":1,"path":"src/service.rs"}))
                .is_err()
        );
    }

    #[test]
    fn source_workspace_can_be_added_only_from_the_current_user_request() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().canonicalize().unwrap();
        let requested = format!("请分析目录 {} 中的问题", path.display());

        ensure_requested_source_directory(&requested, path.to_str().unwrap()).unwrap();
        assert_eq!(
            canonical_source_directory(path.to_str().unwrap()).unwrap(),
            path
        );
        assert!(
            ensure_requested_source_directory("请分析另一个目录", path.to_str().unwrap()).is_err()
        );
        let child = path.join("child");
        let child_request = format!("分析 {} 的问题", child.display());
        assert!(
            ensure_requested_source_directory(&child_request, path.to_str().unwrap()).is_err(),
            "a mentioned child path must not authorize its broader parent"
        );
        assert!(ensure_requested_source_directory(&requested, "relative/project").is_err());
    }
}
