use super::*;

#[test]
fn log_jump_labels_use_file_name_and_source_line() {
    let reference = LogReference {
        document_id: 7,
        version: "snapshot-1".into(),
        line: 42,
    };
    let sources = [vclogg_ai::LogSource {
        document_id: 7,
        version: "snapshot-1".into(),
        path: PathBuf::from("/logs/source.log"),
    }];
    assert_eq!(
        view::reference_label(&reference, Some("/other/current.log"), &sources),
        "current.log:42"
    );
    assert_eq!(
        view::reference_label(&reference, None, &sources),
        "source.log:42"
    );

    let value = json!({"rows": [
        {"reference": reference},
        {"reference": reference, "file": "source.log"}
    ]});
    let mut references = Vec::new();
    view::collect_references(&value, &mut references);
    assert_eq!(references, vec![(reference, Some("source.log".into()))]);
}

fn fixture() -> (tempfile::TempDir, DocumentSnapshot, SharedScope) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("test.log");
    std::fs::write(
        &path,
        "INFO start\nERROR network timeout\nINFO retry\nERROR connection closed\n",
    )
    .unwrap();
    let doc = DocumentSnapshot {
        id: 7,
        version: "snapshot-1".into(),
        document: Arc::new(LogDocument::open(path).unwrap()),
        open: true,
    };
    let scope = Arc::new(Mutex::new(AiScope {
        file_candidates: BTreeMap::new(),
        workspace_directories: Vec::new(),
        source_directory_request: String::new(),
        read_document: None,
        explicit: BTreeSet::new(),
        allowed: [7].into(),
        current: Some(7),
        documents: [(7, doc.clone())].into(),
        directory: DirectorySearchOptions::default(),
        searches: BTreeMap::new(),
        tabs: BTreeMap::new(),
        cancellation: SearchCancellation::default(),
    }));
    (directory, doc, scope)
}
#[test]
fn search_pages_use_source_coordinates_and_reject_changed_sources() {
    let (_dir, doc, scope) = fixture();
    let query = SearchQuery {
        text: "ERROR".into(),
        ..Default::default()
    };
    let result = search_logs(
        scope.clone(),
        vec![doc.clone()],
        None,
        query,
        SearchCancellation::default(),
    )
    .unwrap();
    assert_eq!(result["total"], 2);
    assert_eq!(result["rows"][0]["reference"]["line"], 2);
    assert_eq!(result["rows"][1]["reference"]["line"], 4);
    assert!(
        scope
            .lock()
            .unwrap()
            .reference(&json!({"document_id":7,"version":"old","line":2}))
            .is_err()
    );
    std::fs::write(doc.document.path(), "changed\n").unwrap();
    assert!(read_page(&doc, 0, 100).is_err());
}
#[test]
fn cancellation_and_page_limits_bound_log_access() {
    let (_dir, doc, scope) = fixture();
    let cancel = SearchCancellation::default();
    cancel.cancel();
    assert!(
        search_logs(
            scope,
            vec![doc.clone()],
            None,
            SearchQuery {
                text: "ERROR".into(),
                ..Default::default()
            },
            cancel
        )
        .is_err()
    );
    let page = read_page(&doc, 0, 1).unwrap();
    assert_eq!(page["rows"].as_array().unwrap().len(), 1);
    assert_eq!(page["next_line"], 2);
}
#[test]
fn conversation_lease_prevents_concurrent_agents() {
    let id = uuid::Uuid::new_v4().to_string();
    let lease = ConversationLease::acquire(&id).unwrap();
    assert!(ConversationLease::acquire(&id).is_err());
    drop(lease);
    assert!(ConversationLease::acquire(&id).is_ok());
}

#[gpui_kit::test]
fn log_agent_commands_share_real_workspace_state(cx: &mut gpui_kit::TestAppContext) {
    if std::env::var_os("VCLOGG2_AI_TEST_CHILD").is_none() {
        return;
    }
    cx.update(|cx| {
        gpui_kit::component::init(cx);
        Workspace::init_window_registry(cx);
        crate::notifications::init(cx);
        crate::app_icon::init(cx);
    });
    let (_dir, doc, _) = fixture();
    let document = doc.document.clone();
    let mut workspace = None;
    let window = cx.add_window(|window, cx| {
        let entity = cx.new(|cx| Workspace::new(false, Vec::new(), window, cx));
        workspace = Some(entity.clone());
        Root::new(entity, window, cx)
    });
    let workspace = workspace.unwrap();
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            exercise_workspace(workspace, document, window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            let document_id = workspace.documents[0].id;
            let document = workspace.documents[0].document.clone();
            let global_query = SearchQuery {
                text: "ERROR".into(),
                ..Default::default()
            };
            workspace.global_search.scope = SearchScope::AllOpenFiles;
            workspace.global_search.result_scope = Some(SearchScope::AllOpenFiles);
            workspace.global_search.query = global_query.clone();
            workspace.global_search.matcher = SearchMatcher::new(&global_query).unwrap();
            workspace.global_search.selected_documents = [document_id].into();
            workspace.global_search.results = [(
                document_id,
                GlobalSearchDocumentResult {
                    title: document.file_name().into(),
                    path: document.path().to_path_buf(),
                    document: document.clone(),
                    search_result: vclogg_core::search(&document, &global_query).unwrap(),
                    failure: None,
                },
            )]
            .into_iter()
            .collect();
            workspace.global_search.results_visible = true;
            workspace.refresh_global_result_rows(window, cx);
        })
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(window.into(), |_, _, cx| workspace.update(cx, |workspace, cx| {
        for (table, row_ix) in [(&workspace.documents[0].log_table, 1), (&workspace.documents[0].result_table, 0)] {
            let delegate = table.read(cx).delegate();
            if let Some(request) = delegate.stage_visible_rows(0..4) {
                let document = workspace.documents[0].document.clone();
                let loaded = request.load(|row, max_bytes| document.line_preview(*row, max_bytes));
                delegate.install_staged_visible_lines(loaded);
            }
            let row = delegate.wrapped_row(row_ix).expect("file projection contains marked source row");
            assert_eq!(row.source_row, 1);
            assert!(row.marked && !row.highlights.is_empty());
        }
        let global = workspace.global_table.read(cx).delegate();
        global.load_visible_rows(0..4);
        assert!(matches!(global.wrapped_row(1), Some(crate::global_search_table::WrappedGlobalRow::Match { source_row: 1, marked: true, highlights, .. }) if !highlights.is_empty()));
    })).unwrap();
    cx.update_window(window.into(), |_, _, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.global_table.update(cx, |table, cx| {
                table.delegate().settle_table_selection(1);
                table.set_active_log_row(1, cx);
            });
            let attached = workspace.ai_attachment_targets(LogRegion::GlobalResults, cx);
            assert_eq!(attached.len(), 1);
            assert_eq!(attached[0].source_row, 1);
            assert_eq!(attached[0].document.id, workspace.documents[0].id);
        })
    })
    .unwrap();
}

pub(super) fn install_test_document(
    workspace: &mut Workspace,
    document: Arc<LogDocument>,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let prepared = PreparedDocument {
        document: document.clone(),
        cached_complete_document: None,
        session: None,
        color_labels_snapshot: None,
        resolved_color_rules: Arc::default(),
        search_result: SearchResult::default(),
        search_range: SearchRange::default(),
        search_matcher: None,
        search_case_sensitive: false,
        search_regex: false,
        warning: None,
        load_state: DocumentLoadState::Ready,
        pending_index_cache: None,
        upgrade_frame: None,
    };
    workspace.install_documents(
        vec![(document.path().to_path_buf(), Ok(prepared))],
        Some(document.path()),
        &BTreeMap::new(),
        None,
        true,
        window,
        cx,
    );
}

fn invoke(
    workspace: &mut Workspace,
    scope: &SharedScope,
    name: &str,
    args: Value,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> Value {
    let call = ToolCall {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.into(),
        arguments: args,
    };
    let evidence = workspace.ai_prepare(scope.clone(), &call, cx).unwrap()().unwrap();
    workspace
        .ai_commit(scope, &call, evidence, window, cx)
        .unwrap()
}

fn exercise_workspace(
    workspace: &mut Workspace,
    document: Arc<LogDocument>,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    install_test_document(workspace, document.clone(), window, cx);
    let second_path = document.path().parent().unwrap().join("second.log");
    std::fs::write(&second_path, "INFO second\nERROR peer disconnected\n").unwrap();
    let second = Arc::new(LogDocument::open(&second_path).unwrap());
    install_test_document(workspace, second.clone(), window, cx);
    workspace.activate_tab(0, window, cx);
    workspace.persistence.store = Some(Arc::new(
        StateStore::open(document.path().parent().unwrap().join("state.db")).unwrap(),
    ));
    let scope = workspace.ai_scope();
    let snapshot = scope.lock().unwrap().documents[&workspace.documents[0].id].clone();
    let other = scope.lock().unwrap().documents[&workspace.documents[1].id].clone();
    let reference = json!(snapshot.reference(1));
    let context = invoke(workspace, &scope, "get_context", json!({}), window, cx);
    assert_eq!(context["current_document_id"], snapshot.id);
    let result = invoke(
        workspace,
        &scope,
        "search_logs",
        json!({"scope":"current","query":"ERROR"}),
        window,
        cx,
    );
    assert_eq!(result["total"], 2);
    let all = invoke(
        workspace,
        &scope,
        "search_logs",
        json!({"scope":"open","query":"ERROR"}),
        window,
        cx,
    );
    assert_eq!(all["total"], 3);
    assert_eq!(all["rows"][2]["reference"]["document_id"], other.id);
    invoke(
        workspace,
        &scope,
        "navigate",
        json!({"action":"line","reference":other.reference(1)}),
        window,
        cx,
    );
    assert_eq!(workspace.active_document().unwrap().id, other.id);
    // Explicit agent navigation updates the target of subsequent current-scope searches.
    let current = invoke(
        workspace,
        &scope,
        "search_logs",
        json!({"scope":"current","query":"ERROR"}),
        window,
        cx,
    );
    assert_eq!(current["total"], 1);
    assert_eq!(current["rows"][0]["reference"]["document_id"], other.id);
    // Appending targets the captured file and preserves its existing draft.
    workspace.activate_tab(0, window, cx);
    workspace
        .query
        .update(cx, |input, cx| input.set_value("ERROR", window, cx));
    workspace.capture_active_search_tab(cx);
    workspace.activate_tab(1, window, cx);
    let appended = invoke(
        workspace,
        &scope,
        "append_search",
        json!({"document_id":snapshot.id,"version":snapshot.version,"text":"timeout"}),
        window,
        cx,
    );
    assert_eq!(appended["query"], "ERROR timeout");
    assert_eq!(appended["executed"], false);
    assert_eq!(workspace.active_document().unwrap().id, snapshot.id);
    let call = ToolCall {
        id: "stale-draft".into(),
        name: "append_search".into(),
        arguments: json!({"document_id":snapshot.id,"version":snapshot.version,"text":"retry"}),
    };
    let evidence = workspace.ai_prepare(scope.clone(), &call, cx).unwrap()().unwrap();
    workspace
        .query
        .update(cx, |input, cx| input.set_value("user edit", window, cx));
    assert!(
        workspace
            .ai_commit(&scope, &call, evidence, window, cx)
            .is_err()
    );
    assert_eq!(workspace.query.read(cx).value().as_ref(), "user edit");
    for _ in 0..2 {
        invoke(
            workspace,
            &scope,
            "set_marks",
            json!({"references":[reference],"marked":true}),
            window,
            cx,
        );
    }
    assert_eq!(workspace.documents[0].file.marked_rows.len(), 1);
    assert!(workspace.documents[1].file.marked_rows.is_empty());
    let label = workspace.color_labels[0].id.clone();
    invoke(
        workspace,
        &scope,
        "highlight_keyword",
        json!({"document_id":snapshot.id,"version":snapshot.version,"keyword":"ERROR","color_label_id":label,"action":"set"}),
        window,
        cx,
    );
    assert!(
        workspace.documents[0]
            .file
            .keyword_color_rules
            .iter()
            .any(|r| r.keyword == "ERROR")
    );
    assert!(workspace.documents[1].file.keyword_color_rules.is_empty());
    let mark = invoke(
        workspace,
        &scope,
        "text_mark",
        json!({"reference":reference,"action":"add","text":"网络故障"}),
        window,
        cx,
    );
    let mark_id = mark["mark_id"].as_str().unwrap();
    assert_eq!(
        workspace.documents[0]
            .file
            .row_tags
            .get(1, mark_id)
            .unwrap()
            .label,
        "网络故障"
    );
    invoke(
        workspace,
        &scope,
        "text_mark",
        json!({"reference":reference,"action":"update","mark_id":mark_id,"text":"待排查网络"}),
        window,
        cx,
    );
    invoke(
        workspace,
        &scope,
        "navigate",
        json!({"action":"next","search_id":result["search_id"]}),
        window,
        cx,
    );
    assert_eq!(workspace.selected_source_row, Some(1));
    invoke(
        workspace,
        &scope,
        "navigate",
        json!({"action":"next","search_id":result["search_id"]}),
        window,
        cx,
    );
    assert_eq!(workspace.selected_source_row, Some(3));
    invoke(
        workspace,
        &scope,
        "navigate",
        json!({"action":"previous","search_id":result["search_id"]}),
        window,
        cx,
    );
    assert_eq!(workspace.selected_source_row, Some(1));
    assert_eq!(
        workspace.documents[0]
            .log_table
            .read(cx)
            .delegate()
            .selected_rows_count(),
        1
    );
    let search = invoke(
        workspace,
        &scope,
        "show_search",
        json!({"scope":"current","document_id":snapshot.id,"query":"ERROR"}),
        window,
        cx,
    );
    let (owner, id, _) = scope.lock().unwrap().tabs[search["search_tab"].as_str().unwrap()];
    // Install the core result as if the background completion had arrived.
    let found = vclogg_core::search(
        &document,
        &SearchQuery {
            text: "ERROR".into(),
            ..Default::default()
        },
    )
    .unwrap();
    let tab = workspace.search_tabs.state_mut(owner, id).unwrap();
    tab.local_result = Some((document.clone(), found, None));
    tab.saved.completed = Some(tab.saved.draft.clone());
    workspace.install_search_tab(owner, id, window, cx);
    let page = invoke(
        workspace,
        &scope,
        "control_search",
        json!({"search_tab":search["search_tab"],"action":"results","offset":1}),
        window,
        cx,
    );
    assert_eq!(page["rows"][0]["reference"]["line"], 4);
    assert_eq!(page["rows"][0]["result_index"], 2);
    assert!(
        page["rows"][0]["url"]
            .as_str()
            .unwrap()
            .contains("result_index=2")
    );
    invoke(
        workspace,
        &scope,
        "navigate",
        json!({"action":"result","search_id":page["search_id"],"result_index":2,"reference":page["rows"][0]["reference"]}),
        window,
        cx,
    );
    assert_eq!(workspace.active_log_region, LogRegion::CurrentResults);
    assert_eq!(workspace.selected_source_row, Some(3));
    assert_eq!(
        workspace.documents[0]
            .result_table
            .read(cx)
            .active_log_row(),
        Some(1)
    );
    invoke(
        workspace,
        &scope,
        "control_search",
        json!({"search_tab":search["search_tab"],"action":"clear"}),
        window,
        cx,
    );
    let stale = ToolCall {
        id: "stale".into(),
        name: "search_results".into(),
        arguments: json!({"search_id":page["search_id"]}),
    };
    assert!(workspace.ai_prepare(scope.clone(), &stale, cx).is_err());
    invoke(
        workspace,
        &scope,
        "text_mark",
        json!({"reference":reference,"action":"remove","mark_id":mark_id}),
        window,
        cx,
    );
    assert!(workspace.documents[0].file.row_tags.is_empty());
    // Directory navigation prepares the full document on the worker and installs only at commit.
    let directory_path = document.path().parent().unwrap().join("directory.log");
    std::fs::write(&directory_path, "INFO directory\nERROR remote failure\n").unwrap();
    let directory_doc = DocumentSnapshot {
        id: next_directory_id(),
        version: "directory-version".into(),
        document: Arc::new(LogDocument::open(&directory_path).unwrap()),
        open: false,
    };
    scope
        .lock()
        .unwrap()
        .documents
        .insert(directory_doc.id, directory_doc.clone());
    let open = ToolCall {
        id: "directory-open".into(),
        name: "navigate".into(),
        arguments: json!({"action":"line","reference":directory_doc.reference(1)}),
    };
    let evidence = workspace.ai_prepare(scope.clone(), &open, cx).unwrap()().unwrap();
    scope.lock().unwrap().cancellation.cancel();
    assert!(
        workspace
            .ai_commit(&scope, &open, evidence, window, cx)
            .is_err()
    );
    assert_eq!(workspace.documents.len(), 2);
    scope.lock().unwrap().cancellation = SearchCancellation::default();
    let opened = invoke(workspace, &scope, "navigate", open.arguments, window, cx);
    assert_eq!(workspace.documents.len(), 3);
    assert_eq!(
        workspace.active_document().unwrap().document.path(),
        directory_path
    );
    assert_eq!(workspace.selected_source_row, Some(1));
    assert_ne!(opened["reference"]["document_id"], directory_doc.id);
    invoke(
        workspace,
        &scope,
        "set_marks",
        json!({"references":[opened["reference"]],"marked":true}),
        window,
        cx,
    );
    assert!(workspace.documents[2].file.marked_rows.contains(1));
    workspace.activate_tab(0, window, cx);
    // A file closing between preparation and commit must not mutate another tab.
    let close = ToolCall {
        id: "closed".into(),
        name: "set_marks".into(),
        arguments: json!({"references":[other.reference(1)],"marked":true}),
    };
    let evidence = workspace.ai_prepare(scope.clone(), &close, cx).unwrap()().unwrap();
    let removed = workspace.documents.remove(1);
    assert!(
        workspace
            .ai_commit(&scope, &close, evidence, window, cx)
            .is_err()
    );
    workspace.documents.insert(1, removed);
    let call = ToolCall {
        id: "stopped".into(),
        name: "set_marks".into(),
        arguments: json!({"references":[reference],"marked":false}),
    };
    scope.lock().unwrap().cancellation.cancel();
    assert!(
        workspace
            .ai_commit(&scope, &call, Evidence::Json(json!({})), window, cx)
            .is_err()
    );
    assert!(workspace.documents[0].file.marked_rows.contains(1));
    assert_eq!(
        std::fs::read_to_string(document.path()).unwrap(),
        "INFO start\nERROR network timeout\nINFO retry\nERROR connection closed\n"
    );
}

#[test]
fn isolated_workspace_agent_workflow() {
    let root = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "workspace::ai::tests::log_agent_commands_share_real_workspace_state",
            "--nocapture",
        ])
        .env("VCLOGG2_AI_TEST_CHILD", "1")
        .env("VCLOGG2_DEV_DATA_DIR", root.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn sparse_directory_references_read_surrounding_source_lines() {
    let (_dir, mut doc, scope) = fixture();
    doc.open = false;
    let rows = [1usize, 3].into_iter().collect::<CompressedRows>();
    doc.document = Arc::new(doc.document.project_source_rows(&rows));
    assert!(!doc.document.has_complete_line_index());
    let cancellation = SearchCancellation::default();
    let doc = read_snapshot(&scope, doc, 0, 4, &cancellation).unwrap();
    let page = read_page_cancellable(&doc, 0, 4, &cancellation).unwrap();
    assert_eq!(page["rows"][0]["reference"]["line"], 1);
    assert_eq!(page["rows"][2]["text"], "INFO retry");
}

#[test]
fn markdown_blocks_preserve_code_without_loading_remote_images() {
    let text = "![remote](https://example.org/image) <img src='https://example.org/image'>\n```rust\nlet items: Vec<T> = vec![];\n```\n";
    let rendered = panel::safe_markdown(text);
    assert!(!rendered.contains("![remote]") && !rendered.contains("<img"));
    assert!(rendered.contains("let items: Vec<T> = vec![];"));
}
