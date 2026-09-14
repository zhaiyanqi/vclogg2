use super::*;
use std::{
    io::{Read, Write},
    net::TcpListener,
    time::{Duration, Instant},
};

#[test]
fn isolated_panel_send_workflow() {
    for mode in ["complete", "cancel", "preparing_cancel", "error", "non_sse"] {
        let root = tempfile::tempdir().unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "workspace::ai::panel::tests::panel_sends_streams_and_runs_tools",
                "--nocapture",
            ])
            .env("VCLOGG2_AI_TEST_CHILD", "1")
            .env("VCLOGG2_AI_TEST_MODE", mode)
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
}

fn pump_until(
    cx: &mut gpui::TestAppContext,
    panel: &Entity<AiPanel>,
    ready: impl Fn(&AiPanel) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        cx.run_until_parked();
        if panel.read_with(cx, |p, _| ready(p)) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "panel timed out: {:?}",
            panel.read_with(cx, |p, _| (
                p.busy,
                p.run.is_some(),
                p.conversation.status.clone(),
                p.error.clone(),
                p.live.clone()
            ))
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[gpui::test]
fn panel_sends_streams_and_runs_tools(cx: &mut gpui::TestAppContext) {
    if std::env::var_os("VCLOGG2_AI_TEST_CHILD").is_none() {
        return;
    }
    // Exercise the real Tokio transport against a local TCP service.
    cx.background_executor.allow_parking();
    cx.background_executor.forbid_parking();
    cx.update(|cx| {
        gpui_component::init(cx);
        Workspace::init_window_registry(cx);
        crate::notifications::init(cx);
        crate::app_icon::init(cx);
    });
    let mut owner = None;
    let mut panel = None;
    let window = cx.add_window(|window, cx| {
        let workspace = cx.new(|cx| Workspace::new(false, Vec::new(), window, cx));
        owner = Some(workspace.clone());
        Root::new(workspace, window, cx)
    });
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        let sidebar = owner.as_ref().unwrap().read(cx).sidebar.clone();
        panel = Some(sidebar.update(cx, |s, cx| s.ai_test_panel(window, cx)));
    })
    .unwrap();
    let panel = panel.unwrap();
    pump_until(cx, &panel, |p| !p.busy);
    let mode = std::env::var("VCLOGG2_AI_TEST_MODE").unwrap();
    let preparing_cancel = mode == "preparing_cancel";
    let cancelling = mode == "cancel";
    let failed = matches!(mode.as_str(), "error" | "non_sse");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let config = vclogg_ai::ProviderConfig {
        base_url: format!("http://{}/v1", listener.local_addr().unwrap()),
        model: "mock".into(),
        ..Default::default()
    };
    let (release, wait) = std::sync::mpsc::channel();
    let server = std::thread::spawn(move || {
        for round in 0..if preparing_cancel {
            0
        } else if cancelling || failed {
            1
        } else {
            2
        } {
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut socket = loop {
                match listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline);
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(e) => panic!("{e}"),
                }
            };
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut bytes = Vec::new();
            let body = loop {
                let mut chunk = [0; 4096];
                let n = socket.read(&mut chunk).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&chunk[..n]);
                if let Some(end) = bytes.windows(4).position(|x| x == b"\r\n\r\n") {
                    let length: usize = String::from_utf8_lossy(&bytes[..end])
                        .lines()
                        .find_map(|l| {
                            l.to_lowercase()
                                .strip_prefix("content-length:")
                                .map(|n| n.trim().parse().unwrap())
                        })
                        .unwrap();
                    if bytes.len() >= end + 4 + length {
                        break serde_json::from_slice::<Value>(&bytes[end + 4..end + 4 + length])
                            .unwrap();
                    }
                }
            };
            assert!(body["tools"].as_array().unwrap().len() > 1);
            if round == 1 {
                assert_eq!(body["messages"][2]["reasoning_content"], "开始分析");
                assert_eq!(
                    body["messages"].as_array().unwrap().last().unwrap()["role"],
                    "tool"
                );
            }
            if failed {
                let (status, body) = if mode == "error" {
                    (400, r#"{"error":{"message":"Tool schema not supported"}}"#)
                } else {
                    (200, r#"{"choices":[]}"#)
                };
                write!(socket, "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
                continue;
            }
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n").unwrap();
            if round == 0 {
                write!(
                    socket,
                    "data: {}\n\n",
                    json!({"choices":[{"delta":{"reasoning_content":"开始分析"}}]})
                )
                .unwrap();
                wait.recv_timeout(Duration::from_secs(8)).unwrap();
                let _ = write!(
                    socket,
                    "data: {}\n\n",
                    json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"one","function":{"name":"list_logs","arguments":"{}"}}]},"finish_reason":"tool_calls"}]})
                );
            } else {
                write!(
                    socket,
                    "data: {}\n\n",
                    json!({"choices":[{"delta":{"content":"分析完成"},"finish_reason":"stop"}]})
                )
                .unwrap();
            }
        }
    });
    cx.update_window(window.into(), |_, window, cx| {
        panel.update(cx, |p, cx| {
            p.settings.providers = vec![config.clone()];
            p.conversation.provider_id = Some(config.id);
            p.input.update(cx, |input, cx| {
                input.set_value("分析日志", window, cx);
            });
        })
    })
    .unwrap();
    cx.update_window(window.into(), |_, window, cx| {
        panel.update(cx, |p, cx| p.focus(window, cx))
    })
    .unwrap();
    if preparing_cancel {
        cx.update_window(window.into(), |_, window, cx| {
            panel.update(cx, |p, cx| {
                p.send(false, window, cx);
                assert!(p.busy && p.run.is_none());
                p.stop(cx);
            })
        })
        .unwrap();
        pump_until(cx, &panel, |p| !p.busy && p.run.is_none());
        assert_eq!(
            panel.read_with(cx, |p, _| p.conversation.status.clone()),
            RunStatus::Interrupted
        );
        server.join().unwrap();
        return;
    }
    cx.simulate_keystrokes(window.into(), "enter");
    if failed {
        pump_until(cx, &panel, |p| {
            p.conversation.status == RunStatus::Failed && !p.busy
        });
        panel.read_with(cx, |p, _| {
            assert!(!p.error.is_empty());
            assert_eq!(p.conversation.notice, p.error);
            assert_eq!(p.conversation.messages.len(), 1);
        });
        server.join().unwrap();
        return;
    }
    pump_until(cx, &panel, |p| p.reasoning == "开始分析");
    cx.background_executor
        .advance_clock(Duration::from_millis(50));
    cx.run_until_parked();
    cx.update(|cx| {
        panel.update(cx, |p, cx| {
            p.live_view.update(cx, |v, cx| v.select_all(cx));
            assert!(p.live_view.read(cx).selected_text().contains("开始分析"));
            assert!(!p.progress.is_empty());
        })
    });
    if cancelling {
        cx.update(|cx| panel.update(cx, |p, cx| p.stop(cx)));
        pump_until(cx, &panel, |p| !p.busy && p.run.is_none());
        release.send(()).unwrap();
        server.join().unwrap();
        cx.run_until_parked();
        panel.read_with(cx, |p, _| {
            assert_eq!(p.conversation.status, RunStatus::Interrupted);
            assert_eq!(p.conversation.messages.len(), 2);
            assert!(
                p.conversation
                    .messages
                    .iter()
                    .all(|m| !matches!(m, AgentMessage::Tool { .. }))
            );
        });
        return;
    }
    assert!(panel.read_with(cx, |p, _| p.run.is_some()));
    release.send(()).unwrap();
    pump_until(cx, &panel, |p| !p.busy && p.run.is_none());
    panel.read_with(cx, |p, _| {
        assert_eq!(p.conversation.status.clone(), RunStatus::Complete, "{}", p.error);
        assert_eq!(p.conversation.messages.len(), 4);
        assert!(matches!(&p.conversation.messages[3], AgentMessage::Assistant { text, .. } if text == "分析完成"));
        assert_eq!(p.messages.len(), 4);
        let row = p.store.as_ref().unwrap().load_ai_conversation(&p.conversation.id).unwrap().unwrap();
        let restored: Conversation = serde_json::from_str(&row.payload).unwrap();
        assert!(matches!(&restored.messages[1], AgentMessage::Assistant { reasoning, .. } if reasoning == "开始分析"));
    });
    server.join().unwrap();
    drop(owner);
}
