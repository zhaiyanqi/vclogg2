use super::tests::pump_until;
use super::*;
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    time::{Duration, Instant},
};

pub(super) fn exercise(
    cx: &mut gpui::TestAppContext,
    panel: &Entity<AiPanel>,
    workspace: &Entity<Workspace>,
    window: gpui::WindowHandle<Root>,
) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("attached.log");
    std::fs::write(
        &path,
        "INFO ready\nERROR network timeout\nINFO retry\nERROR disconnected\n",
    )
    .unwrap();
    let first = Arc::new(LogDocument::open(&path).unwrap());
    let other_path = root.path().join("other.log");
    std::fs::write(&other_path, "INFO unrelated\n").unwrap();
    let other = Arc::new(LogDocument::open(&other_path).unwrap());
    let (targets, id, label) = cx
        .update_window(window.into(), |_, window, cx| {
            workspace.update(cx, |w, cx| {
                super::super::tests::install_test_document(w, first.clone(), window, cx);
                super::super::tests::install_test_document(w, other, window, cx);
                w.activate_tab(0, window, cx);
                let id = w.documents[0].id;
                w.documents[0].log_table.update(cx, |table, cx| {
                    table.delegate().settle_table_selection(1);
                    table.set_active_log_row(1, cx);
                });
                let targets = w.ai_attachment_targets(LogRegion::Body, cx);
                assert_eq!(targets.len(), 1);
                assert_eq!(targets[0].source_row, 1);
                // Result index 1 refers to source row 3, including in wrapped views.
                w.documents[0].result_table.update(cx, |table, cx| {
                    table
                        .delegate_mut()
                        .set_row_projection([1usize, 3].into_iter().collect());
                    table.delegate().settle_table_selection(1);
                    table.set_active_log_row(1, cx);
                });
                let result_targets = w.ai_attachment_targets(LogRegion::CurrentResults, cx);
                assert_eq!(result_targets[0].source_row, 3);
                w.activate_tab(1, window, cx);
                (targets, id, w.color_labels[0].id.clone())
            })
        })
        .unwrap();
    // The captured target survives a tab change before the menu action runs.
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |w, cx| w.add_logs_to_ai(targets.clone(), window, cx))
    })
    .unwrap();
    pump_until(cx, panel, |p| !p.attachments_loading);
    panel.read_with(cx, |p, _| {
        assert_eq!(p.draft_logs.len(), 1, "{}", p.error);
        assert_eq!(p.draft_logs[0].document.id, id);
        assert_eq!(p.draft_logs[0].preview, "ERROR network timeout");
    });
    cx.update_window(window.into(), |_, window, cx| {
        panel.update(cx, |p, cx| p.attach_logs(targets, window, cx))
    })
    .unwrap();
    pump_until(cx, panel, |p| !p.attachments_loading);
    assert_eq!(panel.read_with(cx, |p, _| p.draft_logs.len()), 1);

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let config = vclogg_ai::ProviderConfig {
        base_url: format!("http://{}/v1", listener.local_addr().unwrap()),
        model: "analysis-mock".into(),
        ..Default::default()
    };
    let server = std::thread::spawn(move || {
        let (mut socket, first_request) = request(&listener);
        let user = first_request["messages"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()["content"]
            .as_str()
            .unwrap();
        assert!(user.contains("ERROR network timeout"));
        let attached: Value = serde_json::from_str(
            user.split("```json\n")
                .nth(1)
                .unwrap()
                .split("\n```")
                .next()
                .unwrap(),
        )
        .unwrap();
        let reference = &attached["reference"];
        assert_eq!(reference["document_id"], id);
        let calls = [
            ("read_logs", json!({"document_id":id,"version":reference["version"],"start_line":1,"limit":4})),
            ("search_logs", json!({"scope":"open","query":"ERROR"})),
            ("set_marks", json!({"references":[reference],"marked":true})),
            ("highlight_keyword", json!({"document_id":id,"version":reference["version"],"action":"set","keyword":"ERROR","color_label_id":label})),
            ("text_mark", json!({"reference":reference,"action":"add","text":"网络故障"})),
            ("append_search", json!({"document_id":id,"version":reference["version"],"text":"timeout"})),
            ("navigate", json!({"action":"line","reference":reference})),
        ].into_iter().enumerate().map(|(ix, (name, args))| json!({"index":ix,"id":format!("call-{ix}"),"type":"function","function":{"name":name,"arguments":args.to_string()}})).collect::<Vec<_>>();
        socket
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n",
            )
            .unwrap();
        write!(
            socket,
            "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
            json!({"choices":[{"delta":{"reasoning_content":"关联网络异常"}}]}),
            json!({"choices":[{"delta":{"tool_calls":calls},"finish_reason":"tool_calls"}]})
        )
        .unwrap();
        drop(socket);
        let (mut socket, next) = request(&listener);
        let tools = next["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| m["role"] == "tool")
            .collect::<Vec<_>>();
        assert_eq!(tools.len(), 7);
        for tool in tools {
            let result: Value = serde_json::from_str(tool["content"].as_str().unwrap()).unwrap();
            assert!(result.get("error").is_none(), "{result}");
        }
        let reference: LogReference = serde_json::from_value(reference.clone()).unwrap();
        let citation = reference.url();
        let summary =
            format!("分析完成：发现 [attached.log:2]({citation}) 网络异常，已高亮并添加文字标记。");
        socket
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n",
            )
            .unwrap();
        write!(
            socket,
            "data: {}\n\ndata: [DONE]\n\n",
            json!({"choices":[{"delta":{"content":summary},"finish_reason":"stop"}]})
        )
        .unwrap();
        citation
    });
    cx.update_window(window.into(), |_, window, cx| {
        panel.update(cx, |p, cx| {
            p.settings.providers = vec![config.clone()];
            p.conversation.provider_id = Some(config.id);
            p.input.update(cx, |input, cx| {
                input.set_value("分析附加日志并标记异常", window, cx)
            });
            p.send(false, window, cx);
        })
    })
    .unwrap();
    pump_until(cx, panel, |p| !p.busy && p.run.is_none());
    let citation = server.join().unwrap();
    panel.read_with(cx, |p, _| {
        assert_eq!(p.conversation.status, RunStatus::Complete, "{}", p.error);
        assert!(p.draft_logs.is_empty());
        assert!(p.thinking_expanded.is_empty());
        assert!(
            p.conversation
                .messages
                .iter()
                .all(|m| !matches!(m, AgentMessage::Tool { result, .. } if result.is_error))
        );
    });
    workspace.read_with(cx, |w, cx| {
        assert!(w.documents[0].file.marked_rows.contains(1));
        assert!(
            w.documents[0]
                .file
                .keyword_color_rules
                .iter()
                .any(|r| r.keyword == "ERROR")
        );
        assert!(
            w.documents[0]
                .file
                .row_tags
                .row(1)
                .any(|(_, mark)| mark.label == "网络故障")
        );
        assert_eq!(w.query.read(cx).value(), "timeout");
        assert_eq!(w.selected_source_row, Some(1));
        assert!(w.documents[1].file.marked_rows.is_empty());
    });
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |w, cx| w.activate_tab(1, window, cx));
        // Starting another turn must not invalidate an unchanged earlier citation.
        let fresh = workspace.read(cx).ai_scope();
        panel.update(cx, |p, cx| {
            p.reference_scopes.push(p.scope.take().unwrap());
            p.scope = Some(fresh);
            p.open_link(&citation, window, cx);
        });
    })
    .unwrap();
    pump_until(cx, panel, |p| !p.ui_busy);
    workspace.read_with(cx, |w, _| {
        assert_eq!(w.active_document().unwrap().id, id);
        assert_eq!(w.selected_source_row, Some(1));
    });
    // A changed source is rejected before the citation can move selection.
    std::fs::write(&path, "changed\n").unwrap();
    cx.update_window(window.into(), |_, window, cx| {
        workspace.update(cx, |w, cx| w.activate_tab(1, window, cx));
        panel.update(cx, |p, cx| p.open_link(&citation, window, cx));
    })
    .unwrap();
    pump_until(cx, panel, |p| !p.ui_busy);
    assert!(!panel.read_with(cx, |p, _| p.error.clone()).is_empty());
    workspace.read_with(cx, |w, _| assert_ne!(w.active_document().unwrap().id, id));

    // A manually selected unopened result grants only that source, even without a directory.
    let unopened_path = root.path().join("unopened.log");
    std::fs::write(&unopened_path, "ERROR selected directory result\n").unwrap();
    let unopened = DocumentSnapshot {
        id: next_directory_id(),
        version: "attached-unopened".into(),
        document: Arc::new(LogDocument::open(&unopened_path).unwrap()),
        open: false,
    };
    cx.update_window(window.into(), |_, window, cx| {
        panel.update(cx, |p, cx| {
            p.attach_logs(
                vec![super::super::attachments::DraftLog {
                    document: unopened.clone(),
                    source_row: 0,
                    preview: String::new(),
                }],
                window,
                cx,
            )
        });
    })
    .unwrap();
    pump_until(cx, panel, |p| !p.attachments_loading);
    let scope = workspace.read_with(cx, |w, _| w.ai_scope());
    let attached = panel.read_with(cx, |p, _| p.message_with_attachments("", &scope).unwrap());
    assert!(attached.contains("ERROR selected directory result"));
    assert!(scope.lock().unwrap().directory.directory.is_none());
    for (name, arguments) in [
        ("list_logs", json!({})),
        ("get_context", json!({})),
        (
            "read_logs",
            json!({"document_id":unopened.id,"version":unopened.version,"start_line":1}),
        ),
    ] {
        let work = workspace.read_with(cx, |w, cx| {
            w.ai_prepare(
                scope.clone(),
                &ToolCall {
                    id: name.into(),
                    name: name.into(),
                    arguments,
                },
                cx,
            )
            .unwrap()
        });
        assert!(
            work().is_ok(),
            "{name} must retain the explicit attachment grant"
        );
    }
    assert!(scope.lock().unwrap().document(u64::MAX, None).is_err());
}

fn request(listener: &TcpListener) -> (TcpStream, Value) {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut socket = loop {
        match listener.accept() {
            Ok((socket, _)) => break socket,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(Instant::now() < deadline, "request timed out");
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("{error}"),
        }
    };
    // Accepted sockets may inherit the nonblocking listener mode.
    socket.set_nonblocking(false).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut bytes = Vec::new();
    loop {
        let mut chunk = [0; 4096];
        let n = socket.read(&mut chunk).unwrap();
        assert!(n > 0);
        bytes.extend_from_slice(&chunk[..n]);
        if let Some(end) = bytes.windows(4).position(|chunk| chunk == b"\r\n\r\n") {
            let length: usize = String::from_utf8_lossy(&bytes[..end])
                .lines()
                .find_map(|line| {
                    line.to_lowercase()
                        .strip_prefix("content-length:")
                        .map(|s| s.trim().parse().unwrap())
                })
                .unwrap();
            if bytes.len() >= end + 4 + length {
                return (
                    socket,
                    serde_json::from_slice(&bytes[end + 4..end + 4 + length]).unwrap(),
                );
            }
        }
    }
}
