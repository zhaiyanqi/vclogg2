use super::analysis_tests::request;
use super::tests::pump_until;
use super::*;
use std::{io::Write, net::TcpListener, time::Duration};

pub(super) fn exercise(
    cx: &mut gpui_kit::TestAppContext,
    first: &Entity<ConversationSession>,
    workspace: &Entity<Workspace>,
    window: gpui_kit::WindowHandle<Root>,
    cancel: bool,
) {
    let host = workspace.read_with(cx, |w, cx| w.sidebar.read(cx).ai_test_host());
    let listeners = [
        TcpListener::bind("127.0.0.1:0").unwrap(),
        TcpListener::bind("127.0.0.1:0").unwrap(),
    ];
    let configs = listeners
        .iter()
        .enumerate()
        .map(|(ix, listener)| {
            listener.set_nonblocking(true).unwrap();
            vclogg_ai::ProviderConfig {
                model: format!("model-{ix}"),
                base_url: format!("http://{}/v1", listener.local_addr().unwrap()),
                ..Default::default()
            }
        })
        .collect::<Vec<_>>();
    let mut releases = Vec::new();
    let mut servers = Vec::new();
    for (ix, listener) in listeners.into_iter().enumerate() {
        let (release, wait) = std::sync::mpsc::channel();
        releases.push(release);
        servers.push(std::thread::spawn(move || {
            for round in 0..if ix == 1 { 2 } else { 1 } {
                let (mut socket, body) = request(&listener);
                assert_eq!(body["model"], format!("model-{ix}"), "queued model changed");
                let messages = body["messages"].to_string();
                assert!(!messages.contains(if ix == 0 { "question-B" } else { "question-A" }), "cross-session transcript");
                socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n").unwrap();
                write!(socket, "data: {}\n\n", json!({"choices":[{"delta":{"content":format!("stream-{ix}")}}]})).unwrap();
                if round == 0 { wait.recv_timeout(Duration::from_secs(10)).unwrap(); }
                let _ = write!(socket, "data: {}\n\n", json!({"choices":[{"delta":{"content":format!("done-{ix}")},"finish_reason":"stop"}]}));
            }
        }));
    }
    cx.update_window(window.into(), |_, window, cx| {
        first.update(cx, |s, cx| {
            s.settings.providers = configs.clone();
            s.settings.active_provider = Some(configs[0].id.clone());
            s.conversation.provider_id = Some(configs[0].id.clone());
            s.input
                .update(cx, |input, cx| input.set_value("question-A", window, cx));
            s.send(false, window, cx);
        });
    })
    .unwrap();
    pump_until(cx, first, |s| s.live == "stream-0");
    let mut second = None;
    cx.update_window(window.into(), |_, window, cx| {
        host.update(cx, |host, cx| {
            host.new_conversation_tab(window, cx);
            second = Some(host.active.clone());
        });
        second.as_ref().unwrap().update(cx, |s, cx| {
            s.conversation.provider_id = Some(configs[1].id.clone());
            s.input
                .update(cx, |input, cx| input.set_value("question-B", window, cx));
            s.send(false, window, cx);
        });
    })
    .unwrap();
    let second = second.unwrap();
    pump_until(cx, &second, |s| s.live == "stream-1");
    cx.update_window(window.into(), |_, window, cx| {
        let focus = host.read(cx).conversation_tab_focus.clone();
        focus.focus(window, cx);
    })
    .unwrap();
    cx.simulate_keystrokes(window.into(), "left");
    assert_eq!(host.read_with(cx, |host, _| host.active.clone()), *first);
    let first_id = first.read_with(cx, |s, _| s.conversation.id.clone());
    let second_id = second.read_with(cx, |s, _| s.conversation.id.clone());
    cx.update_window(window.into(), |_, window, cx| {
        first.update(cx, |s, cx| {
            assert!(!s.settings_busy(cx));
            s.select_model(configs[1].id.clone(), cx);
            assert_eq!(s.running_model.as_deref(), Some("model-0"));
            s.save_settings(window, cx);
        });
        second.update(cx, |s, cx| {
            s.input
                .update(cx, |input, cx| input.set_value("follow-up-B", window, cx));
            s.queue_current_prompt(false, window, cx);
            s.select_model(configs[0].id.clone(), cx);
            assert_eq!(s.queued_prompts[0].preferences.model(), "model-1");
        });
        host.update(cx, |host, cx| {
            host.load_conversation(first_id.clone(), window, cx);
            assert_eq!(host.active, *first);
            host.close_conversation_tab(&second_id, window, cx);
            assert!(!host.open_conversations.contains(&second_id));
            assert!(host.is_running(&second_id, cx));
            host.refresh_conversation_history(false, window, cx);
        });
    })
    .unwrap();
    pump_until(cx, first, |s| !s.settings_work);
    assert!(first.read_with(cx, |s, _| s.run.is_some()));
    releases[1].send(()).unwrap();
    pump_until(cx, &second, |s| {
        !s.is_running() && s.queued_prompts.is_empty()
    });
    assert!(second.read_with(cx, |s, _| s.unread));
    cx.update_window(window.into(), |_, window, cx| {
        host.update(cx, |host, cx| {
            host.load_conversation(second_id.clone(), window, cx);
            assert_eq!(host.active, second);
            assert!(!host.active.read(cx).unread);
        });
        if cancel {
            first.update(cx, |s, cx| s.stop(cx));
        }
    })
    .unwrap();
    if cancel {
        pump_until(cx, first, |s| !s.is_running());
    }
    releases[0].send(()).unwrap();
    pump_until(cx, first, |s| !s.is_running());
    first.read_with(cx, |s, _| {
        assert_eq!(
            s.conversation.status,
            if cancel {
                RunStatus::Interrupted
            } else {
                RunStatus::Complete
            }
        );
        assert!(s.error.is_empty() || cancel, "{}", s.error);
    });
    second.read_with(cx, |s, _| {
        assert_eq!(s.conversation.status, RunStatus::Complete);
        assert_eq!(s.conversation.provider_id.as_ref(), Some(&configs[0].id));
        assert!(s.error.is_empty(), "{}", s.error);
    });
    for session in [first, &second] {
        session.read_with(cx, |s, _| {
            let saved = s
                .store
                .as_ref()
                .unwrap()
                .load_ai_conversation(&s.conversation.id)
                .unwrap()
                .unwrap();
            let saved: Conversation = serde_json::from_str(&saved.payload).unwrap();
            assert_eq!(saved.id, s.conversation.id);
            assert_eq!(saved.messages.len(), s.conversation.messages.len());
            assert_eq!(saved.status, s.conversation.status);
            assert_eq!(saved.provider_id, s.conversation.provider_id);
            assert_eq!(saved.run_models, s.conversation.run_models);
        });
    }
    let original_count = first.read_with(cx, |s, _| s.conversation.messages.len());
    cx.update_window(window.into(), |_, _, cx| {
        first.update(cx, |_, cx| cx.emit(SessionEvent::Fork(1)));
    })
    .unwrap();
    let branch = host.read_with(cx, |host, _| host.active.clone());
    pump_until(cx, &branch, |s| !s.is_running());
    branch.read_with(cx, |s, _| {
        assert_ne!(s.conversation.id, first_id);
        assert_eq!(s.conversation.messages.len(), 1);
        assert!(s.revision > 0);
    });
    assert_eq!(
        first.read_with(cx, |s, _| s.conversation.messages.len()),
        original_count
    );
    for server in servers {
        server.join().unwrap();
    }
}

pub(super) fn questions(
    cx: &mut gpui_kit::TestAppContext,
    first: &Entity<ConversationSession>,
    workspace: &Entity<Workspace>,
    window: gpui_kit::WindowHandle<Root>,
) {
    let host = workspace.read_with(cx, |w, cx| w.sidebar.read(cx).ai_test_host());
    let mut sessions = vec![first.clone()];
    let mut servers = Vec::new();
    for ix in 0..2 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let config = vclogg_ai::ProviderConfig {
            base_url: format!("http://{}/v1", listener.local_addr().unwrap()),
            model: format!("question-{ix}"),
            ..Default::default()
        };
        servers.push(std::thread::spawn(move || {
            let (mut socket, _) = request(&listener);
            let arguments = json!({"question":format!("Question {ix}"),"options":[{"id":"yes","label":"Yes"},{"id":"no","label":"No"}],"allow_free_text":true}).to_string();
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\ndata: {}\n\n", json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"same-call-id","function":{"name":"ask_user","arguments":arguments}}]},"finish_reason":"tool_calls"}]})).unwrap();
            drop(socket);
            let (mut socket, body) = request(&listener);
            let reply: Value = serde_json::from_str(body["messages"].as_array().unwrap().last().unwrap()["content"].as_str().unwrap()).unwrap();
            assert_eq!(reply["answer"], format!("answer-{ix}"));
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\ndata: {}\n\n", json!({"choices":[{"delta":{"content":"answered"},"finish_reason":"stop"}]})).unwrap();
        }));
        cx.update_window(window.into(), |_, window, cx| {
            if ix == 1 {
                host.update(cx, |host, cx| {
                    host.new_conversation_tab(window, cx);
                    sessions.push(host.active.clone());
                });
            }
            sessions[ix].update(cx, |s, cx| {
                s.settings.providers = vec![config.clone()];
                s.conversation.provider_id = Some(config.id.clone());
                s.input.update(cx, |input, cx| {
                    input.set_value("ask a question", window, cx)
                });
                s.send(false, window, cx);
            });
        })
        .unwrap();
        pump_until(cx, &sessions[ix], |s| s.pending_question.is_some());
    }
    assert_eq!(
        host.read_with(cx, |host, _| host.active.clone()),
        sessions[1]
    );
    for ix in [1, 0] {
        cx.update_window(window.into(), |_, window, cx| {
            sessions[ix].update(cx, |s, cx| {
                s.answer_question(None, format!("answer-{ix}"), window, cx)
            });
        })
        .unwrap();
        pump_until(cx, &sessions[ix], |s| !s.is_running());
        if ix == 1 {
            assert!(first.read_with(cx, |s, _| s.pending_question.is_some()));
        }
    }
    for server in servers {
        server.join().unwrap();
    }
}

pub(super) fn slots(
    cx: &mut gpui_kit::TestAppContext,
    first: &Entity<ConversationSession>,
    workspace: &Entity<Workspace>,
    window: gpui_kit::WindowHandle<Root>,
) {
    let host = workspace.read_with(cx, |w, cx| w.sidebar.read(cx).ai_test_host());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let config = vclogg_ai::ProviderConfig {
        base_url: format!("http://{}/v1", listener.local_addr().unwrap()),
        model: "slots".into(),
        ..Default::default()
    };
    let (release, wait) = std::sync::mpsc::channel();
    let server = std::thread::spawn(move || {
        let mut sockets = Vec::new();
        for _ in 0..3 {
            let (mut socket, _) = request(&listener);
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\ndata: {}\n\n", json!({"choices":[{"delta":{"content":"started"}}]})).unwrap();
            sockets.push(socket);
        }
        wait.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(
            matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock),
            "a fourth model request escaped the concurrency limit"
        );
        for mut socket in sockets {
            write!(
                socket,
                "data: {}\n\n",
                json!({"choices":[{"delta":{"content":"done"},"finish_reason":"stop"}]})
            )
            .unwrap();
        }
    });
    let mut sessions = vec![first.clone()];
    for ix in 0..4 {
        cx.update_window(window.into(), |_, window, cx| {
            if ix > 0 {
                host.update(cx, |host, cx| {
                    host.new_conversation_tab(window, cx);
                    sessions.push(host.active.clone());
                });
            }
            sessions[ix].update(cx, |s, cx| {
                s.settings.providers = vec![config.clone()];
                s.conversation.provider_id = Some(config.id.clone());
                s.input.update(cx, |input, cx| {
                    input.set_value(format!("question-{ix}"), window, cx)
                });
                s.send(false, window, cx);
            });
        })
        .unwrap();
        pump_until(cx, &sessions[ix], |s| {
            if ix < 3 {
                s.live == "started"
            } else {
                s.waiting_for_slot
            }
        });
    }
    cx.update_window(window.into(), |_, _, cx| {
        sessions[3].update(cx, |s, cx| s.stop(cx))
    })
    .unwrap();
    pump_until(cx, &sessions[3], |s| !s.is_running());
    assert_eq!(
        sessions[3].read_with(cx, |s, _| s.conversation.status.clone()),
        RunStatus::Interrupted
    );
    release.send(()).unwrap();
    for session in &sessions[..3] {
        pump_until(cx, session, |s| !s.is_running());
    }
    server.join().unwrap();
}
