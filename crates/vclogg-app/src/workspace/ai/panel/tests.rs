use super::*;
use std::{
    io::{Read, Write},
    net::TcpListener,
    time::{Duration, Instant},
};

#[test]
fn isolated_panel_send_workflow() {
    for mode in [
        "complete",
        "transcript",
        "workspace_transcript",
        "log_analysis",
        "cancel",
        "preparing_cancel",
        "error",
        "non_sse",
        "missing_directory",
        "directory_is_file",
    ] {
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

pub(super) fn pump_until(
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
    if matches!(mode.as_str(), "transcript" | "workspace_transcript") {
        let fixture = tempfile::tempdir().unwrap();
        if mode == "workspace_transcript" {
            let path = fixture.path().join("selection.log");
            std::fs::write(&path, "INFO ready\nERROR network timeout\n").unwrap();
            let document = Arc::new(LogDocument::open(&path).unwrap());
            cx.update_window(window.into(), |_, window, cx| {
                owner.as_ref().unwrap().update(cx, |w, cx| {
                    super::super::tests::install_test_document(w, document, window, cx);
                });
            })
            .unwrap();
        }
        exercise_transcript(cx, &panel, window);
        verify_chat_geometry(cx, &panel, window, mode == "workspace_transcript");
        return;
    }
    if mode == "log_analysis" {
        super::analysis_tests::exercise(cx, &panel, owner.as_ref().unwrap(), window);
        return;
    }
    let directory_unavailable = matches!(mode.as_str(), "missing_directory" | "directory_is_file");
    let directory_fixture = tempfile::tempdir().unwrap();
    let selected_directory = directory_fixture.path().join("selected");
    if directory_unavailable {
        if mode == "directory_is_file" {
            std::fs::write(&selected_directory, "not a directory").unwrap();
        }
        cx.update(|cx| {
            owner.as_ref().unwrap().update(cx, |w, _| {
                w.global_search.directory_options.directory = Some(selected_directory.clone());
            })
        });
    }
    if directory_unavailable {
        let log_path = directory_fixture.path().join("open.log");
        std::fs::write(&log_path, "INFO ready\nERROR timeout\n").unwrap();
        let document = Arc::new(LogDocument::open(log_path).unwrap());
        cx.update_window(window.into(), |_, window, cx| {
            owner.as_ref().unwrap().update(cx, |w, cx| {
                super::super::tests::install_test_document(w, document, window, cx);
            });
        })
        .unwrap();
    }
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
            // Accepted sockets may inherit the nonblocking listener mode.
            socket.set_nonblocking(false).unwrap();
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
                if directory_unavailable {
                    let result: Value = serde_json::from_str(
                        body["messages"].as_array().unwrap().last().unwrap()["content"]
                            .as_str()
                            .unwrap(),
                    )
                    .unwrap();
                    assert_eq!(result["files"].as_array().unwrap().len(), 1);
                    assert_eq!(result["files"][0]["name"], "open.log");
                }
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
    let streamed_view = panel.read_with(cx, |p, cx| {
        assert_eq!(p.scroller.read(cx).item_count(), 2);
        assert!(p.live_row);
        p.live_view.entity_id()
    });
    cx.background_executor
        .advance_clock(Duration::from_millis(50));
    cx.run_until_parked();
    cx.update(|cx| {
        panel.update(cx, |p, cx| {
            p.live_reasoning_view.update(cx, |v, cx| v.select_all(cx));
            assert!(
                p.live_reasoning_view
                    .read(cx)
                    .selected_text()
                    .contains("开始分析")
            );
            assert!(!p.progress.is_empty());
        })
    });
    if cancelling {
        cx.update(|cx| panel.update(cx, |p, cx| p.stop(cx)));
        pump_until(cx, &panel, |p| !p.busy && p.run.is_none());
        release.send(()).unwrap();
        server.join().unwrap();
        cx.run_until_parked();
        panel.read_with(cx, |p, cx| {
            assert!(!p.live_row);
            assert!(p.reasoning_views[1].is_some());
            assert!(!p.thinking_expanded.contains(&1));
            assert_eq!(p.scroller.read(cx).item_count(), 2);
            assert_eq!(p.messages[1].entity_id(), streamed_view);
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
    panel.read_with(cx, |p, cx| {
        assert!(!p.live_row);
        assert!(p.reasoning_views[1].is_some());
        assert!(!p.thinking_expanded.contains(&1));
        assert_eq!(p.scroller.read(cx).item_count(), 2);
        assert_eq!(p.messages[1].entity_id(), streamed_view);
        assert_eq!(p.conversation.status.clone(), RunStatus::Complete, "{}", p.error);
        assert_eq!(p.conversation.messages.len(), 4);
        assert!(matches!(&p.conversation.messages[3], AgentMessage::Assistant { text, .. } if text == "分析完成"));
        assert_eq!(p.messages.len(), 4);
        let row = p.store.as_ref().unwrap().load_ai_conversation(&p.conversation.id).unwrap().unwrap();
        let restored: Conversation = serde_json::from_str(&row.payload).unwrap();
        assert!(matches!(&restored.messages[1], AgentMessage::Assistant { reasoning, .. } if reasoning == "开始分析"));
    });
    if directory_unavailable {
        panel.read_with(cx, |p, _| {
            assert!(
                p.scope
                    .as_ref()
                    .unwrap()
                    .lock()
                    .unwrap()
                    .directory
                    .directory
                    .is_none()
            );
            assert!(!p.conversation.notice.is_empty());
            assert!(p.error.is_empty());
        });
        let scope = panel.read_with(cx, |p, _| p.scope.clone().unwrap());
        let work = owner.as_ref().unwrap().read_with(cx, |w, cx| {
            w.ai_prepare(
                scope,
                &ToolCall {
                    id: "directory-search".into(),
                    name: "search_logs".into(),
                    arguments: json!({"scope":"directory", "query":"ERROR"}),
                },
                cx,
            )
            .unwrap()
        });
        assert!(
            work().is_err(),
            "Unavailable directory must remain inaccessible"
        );
        owner.as_ref().unwrap().read_with(cx, |w, _| {
            assert_eq!(
                w.global_search.directory_options.directory.as_ref(),
                Some(&selected_directory)
            );
        });
    }
    server.join().unwrap();
    drop(owner);
}

fn exercise_transcript(
    cx: &mut gpui::TestAppContext,
    panel: &Entity<AiPanel>,
    window: gpui::WindowHandle<Root>,
) {
    cx.update_window(window.into(), |_, window, cx| {
        panel.update(cx, |p, cx| {
            assert_eq!(p.scroller.read(cx).item_count(), 0);
            p.input
                .update(cx, |input, cx| input.set_value("问题", window, cx));
            p.focus(window, cx);
        });
    })
    .unwrap();
    cx.simulate_keystrokes(window.into(), "shift-enter");
    panel.read_with(cx, |p, cx| {
        assert!(p.input.read(cx).value().contains('\n'));
        assert!(p.conversation.messages.is_empty());
        assert!(p.error.is_empty());
    });
    cx.update(|cx| {
        panel.update(cx, |p, cx| {
            for _ in 0..24 {
                p.push_message(
                    AgentMessage::User {
                        text: "已保存的历史日志分析内容。\n\n".repeat(8),
                    },
                    cx,
                );
            }
            assert!(p.scroller.read(cx).is_following_tail());
            p.scroller.update(cx, |s, cx| {
                assert!(s.scroll_to_item(3, cx));
            });
            p.recover_messages(cx);
            assert!(!p.scroller.read(cx).is_following_tail());
            p.receive_event(AgentEvent::Thinking("检查日志".into()), cx);
        })
    });
    cx.run_until_parked();
    let streamed_view = panel.read_with(cx, |p, _| p.live_view.entity_id());
    for chunk in [
        "中文流式",
        "回复\n\n",
        "| 项目 | 结果 |\n| --- | --- |\n| 错误 | 2 |",
    ] {
        cx.update(|cx| {
            panel.update(cx, |p, cx| {
                p.receive_event(AgentEvent::Text(chunk.into()), cx)
            })
        });
        cx.background_executor
            .advance_clock(Duration::from_millis(50));
        cx.run_until_parked();
        panel.read_with(cx, |p, cx| {
            assert_eq!(p.scroller.read(cx).item_count(), 25);
            assert_eq!(p.live_view.entity_id(), streamed_view);
            assert!(
                p.thinking_expanded.contains(&24),
                "thinking stays expanded while the reply streams"
            );
            assert!(!p.scroller.read(cx).is_following_tail());
        });
    }
    cx.update(|cx| {
        panel.update(cx, |p, cx| {
            p.receive_event(
                AgentEvent::Assistant(AgentMessage::Assistant {
                    text: p.live.clone(),
                    reasoning: p.reasoning.clone(),
                    thinking: Vec::new(),
                    calls: Vec::new(),
                }),
                cx,
            );
            assert_eq!(p.messages[24].entity_id(), streamed_view);
            assert_eq!(p.scroller.read(cx).item_count(), 25);
            assert!(!p.live_row);
            p.receive_event(AgentEvent::Text(String::new()), cx);
            assert!(!p.live_row);
            assert!(p.live_task.is_none());
            p.receive_event(
                AgentEvent::ToolFinished(AgentMessage::Tool {
                    call_id: "read".into(),
                    name: "read_logs".into(),
                    result: ToolResult::error("File closed"),
                }),
                cx,
            );
            assert_eq!(p.scroller.read(cx).item_count(), 25);
            assert!(!p.scroller.read(cx).is_following_tail());
            p.scroller.update(cx, |state, cx| state.scroll_to_end(cx));
            assert!(p.scroller.read(cx).is_following_tail());
            p.receive_event(AgentEvent::Text("下一段".into()), cx);
            p.receive_event(
                AgentEvent::Finished(RunStatus::Interrupted, String::new()),
                cx,
            );
            assert_eq!(p.scroller.read(cx).item_count(), 25);
            assert!(!p.live_row);
            p.busy = false;
            p.new_conversation(cx);
            assert_eq!(p.scroller.read(cx).item_count(), 0);
            assert!(p.scroller.read(cx).is_following_tail());
            assert!(p.messages.is_empty());
        })
    });
}

fn verify_chat_geometry(
    cx: &mut gpui::TestAppContext,
    panel: &Entity<AiPanel>,
    window: gpui::WindowHandle<Root>,
    embedded: bool,
) {
    cx.update(|cx| {
        panel.update(cx, |p, cx| {
            p.push_message(
                AgentMessage::User {
                    text: {
                        let reference = LogReference {
                            document_id: 1,
                            version: "v1".into(),
                            line: 2,
                        };
                        let data =
                            json!({"reference":reference,"log_data":"ERROR network timeout"});
                        format!(
                            "Analyze these logs\n\n[app.log:2]({})\n```json\n{data}\n```",
                            reference.url()
                        )
                    },
                },
                cx,
            );
            p.push_message(
                AgentMessage::Assistant {
                    text: "发现两条相关日志".into(),
                    reasoning: "Check source lines.\n\n".repeat(4),
                    thinking: Vec::new(),
                    calls: Vec::new(),
                },
                cx,
            );
            // Completed replies collapse thinking through the normal finish event.
            p.receive_event(AgentEvent::Finished(RunStatus::Complete, String::new()), cx);
            p.busy = false;
        })
    });
    cx.run_until_parked();
    if !embedded {
        cx.update_window(window.into(), |_, window, cx| {
            window.replace_root(cx, |window, cx| Root::new(panel.clone(), window, cx));
        })
        .unwrap();
    }
    let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
    for mode in [
        gpui_component::ThemeMode::Dark,
        gpui_component::ThemeMode::Light,
    ] {
        visual.update(|_, cx| crate::ui_theme::apply_product_theme(mode, cx));
        let widths = if embedded {
            [1200., 1400.]
        } else {
            [320., 560.]
        };
        for width in widths {
            visual.simulate_resize(gpui::size(
                px(width),
                px(if embedded { 900. } else { 600. }),
            ));
            cx.run_until_parked();
            let sent = visual.debug_bounds("ai-user-bubble").expect("user bubble");
            let reply = visual.debug_bounds("ai-reply").expect("assistant reply");
            let references = visual
                .debug_bounds("ai-user-references")
                .expect("references above the bubble");
            assert!(references.bottom() < sent.top());
            assert!(sent.left() > reply.left(), "sent={sent:?}, reply={reply:?}");
            assert!((sent.right() - reply.right()).abs() <= px(1.));
            assert!(
                reply.size.width <= px(width),
                "width={width}, reply={reply:?}"
            );
        }
    }
    let sent = visual.debug_bounds("ai-user-bubble").unwrap();
    let from = gpui::point(sent.left() + px(17.), sent.top() + px(22.));
    let to = gpui::point(sent.right() - px(17.), sent.top() + px(22.));
    visual.simulate_mouse_down(from, MouseButton::Left, gpui::Modifiers::default());
    visual.update(|window, cx| {
        _ = window.draw(cx);
    });
    let mut lengths = Vec::new();
    for fraction in [0.4, 1.0, 0.2, 1.0] {
        let position = gpui::point(from.x + (to.x - from.x) * fraction, to.y);
        visual.simulate_mouse_move(position, MouseButton::Left, gpui::Modifiers::default());
        visual.update(|window, cx| {
            _ = window.draw(cx);
        });
        visual.update(|window, cx| {
            let blue = ui_theme::palette(cx).chat_selection_background;
            assert!(
                window.painted_quads().iter().any(|quad| {
                    quad.background == blue.into()
                        && quad.bounds.intersects(&sent.scale(window.scale_factor()))
                }),
                "the selected message paints the blue background"
            );
        });
        lengths.push(panel.read_with(cx, |p, cx| p.messages[0].read(cx).selected_text().len()));
    }
    assert!(
        lengths[0] > 0 && lengths[1] > lengths[0],
        "drag expands selection: {lengths:?}"
    );
    assert!(
        lengths[2] < lengths[1] && lengths[3] == lengths[1],
        "reverse drag shrinks selection: {lengths:?}"
    );
    visual.simulate_mouse_up(to, MouseButton::Left, gpui::Modifiers::default());
    cx.run_until_parked();
    let selected = panel.read_with(cx, |p, cx| p.messages[0].read(cx).selected_text());

    assert!(
        !selected.is_empty(),
        "user text supports pointer drag selection: {sent:?}, {from:?}, {to:?}"
    );
    assert!(!selected.contains("log_data"));
    visual.simulate_mouse_down(to, MouseButton::Right, gpui::Modifiers::default());
    visual.simulate_mouse_up(to, MouseButton::Right, gpui::Modifiers::default());
    cx.run_until_parked();
    visual.update(|window, cx| {
        _ = window.draw(cx);
    });
    visual.simulate_keystrokes("down enter");
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some(selected.clone())
    );
    visual.simulate_mouse_down(to, MouseButton::Right, gpui::Modifiers::default());
    visual.simulate_mouse_up(to, MouseButton::Right, gpui::Modifiers::default());
    cx.run_until_parked();
    visual.update(|window, cx| {
        _ = window.draw(cx);
    });
    visual.simulate_keystrokes("down down enter");
    panel.read_with(cx, |p, cx| {
        assert!(p.input.read(cx).value().ends_with(&selected))
    });
    let collapsed = visual
        .debug_bounds("ai-thinking-region")
        .unwrap()
        .size
        .height;
    let toggle = visual.debug_bounds("ai-thinking-toggle").unwrap();
    visual.simulate_click(
        gpui::point(toggle.left() + px(20.), toggle.top() + px(12.)),
        gpui::Modifiers::default(),
    );
    cx.run_until_parked();
    assert!(
        visual
            .debug_bounds("ai-thinking-region")
            .unwrap()
            .size
            .height
            > collapsed
    );
    let thought = visual.debug_bounds("ai-reasoning-text").unwrap();
    let reply = visual.debug_bounds("ai-reply").unwrap();
    assert!(
        (thought.left() - reply.left()).abs() <= px(1.),
        "thoughts align with the reply"
    );
    assert!(
        (thought.right() - reply.right()).abs() <= px(1.),
        "thoughts use the full reply width"
    );
    let from = gpui::point(thought.left() + px(2.), thought.top() + px(10.));
    let to = gpui::point(thought.left() + px(120.), thought.top() + px(10.));
    visual.simulate_mouse_down(from, MouseButton::Left, gpui::Modifiers::default());
    visual.update(|window, cx| {
        _ = window.draw(cx);
    });
    let across_paragraphs = gpui::point(to.x, from.y + px(65.));
    visual.simulate_mouse_move(
        across_paragraphs,
        MouseButton::Left,
        gpui::Modifiers::default(),
    );
    visual.update(|window, cx| {
        _ = window.draw(cx);
    });
    let expanded_selection = panel.read_with(cx, |p, cx| {
        p.reasoning_views[1]
            .as_ref()
            .unwrap()
            .read(cx)
            .selected_text()
    });
    assert!(
        expanded_selection.contains('\n'),
        "drag selects across paragraphs: {expanded_selection:?}"
    );
    visual.simulate_mouse_move(to, MouseButton::Left, gpui::Modifiers::default());
    visual.update(|window, cx| {
        _ = window.draw(cx);
    });
    visual.simulate_mouse_up(to, MouseButton::Left, gpui::Modifiers::default());
    let selected = panel.read_with(cx, |p, cx| {
        p.reasoning_views[1]
            .as_ref()
            .unwrap()
            .read(cx)
            .selected_text()
    });
    assert!(
        !selected.is_empty(),
        "expanded thoughts support drag selection: {thought:?}"
    );
    visual.simulate_mouse_down(to, MouseButton::Right, gpui::Modifiers::default());
    visual.simulate_mouse_up(to, MouseButton::Right, gpui::Modifiers::default());
    visual.update(|window, cx| {
        _ = window.draw(cx);
    });
    visual.simulate_keystrokes("down enter");
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some(selected)
    );
    assert!(panel.read_with(cx, |p, _| p.message_menu.is_none()));
    if embedded {
        let hit = |window: &Window, cx: &App| {
            let workspace = panel.read(cx).workspace.upgrade().unwrap();
            workspace
                .read(cx)
                .sidebar
                .read(cx)
                .contains_ai_transcript(from, window, cx)
        };
        visual.update(|window, cx| {
            assert!(hit(window, cx));
            panel.update(cx, |p, _| p.show_settings = true);
            assert!(
                !hit(window, cx),
                "settings must not reuse transcript bounds"
            );
            panel.update(cx, |p, _| p.show_settings = false);
        });
        visual.simulate_resize(gpui::size(px(400.), px(900.)));
        visual.update(|window, cx| {
            assert!(
                !hit(window, cx),
                "an automatically hidden sidebar must not reuse transcript bounds"
            );
        });
    }
}
