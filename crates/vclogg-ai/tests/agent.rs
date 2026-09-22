use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{Arc, Mutex},
    time::Duration,
};
use vclogg_ai::*;

#[tokio::test]
async fn explicit_compaction_sends_no_tools_and_returns_summary() {
    let (config, requests, server) = mock(vec![(
        200,
        openai_text("Confirmed cause; refresh log references."),
    )]);
    let messages = vec![AgentMessage::User {
        text: "Investigate timeout".into(),
    }];
    let summary = compact_conversation(
        &config,
        "Earlier finding",
        &messages,
        &Cancellation::default(),
    )
    .await
    .unwrap();
    server.join().unwrap();
    assert!(summary.contains("Confirmed cause"));
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].get("tools").is_none());
    assert!(
        requests[0]["messages"]
            .to_string()
            .contains("Earlier finding")
    );
}

#[tokio::test]
async fn agent_automatically_compacts_before_the_context_limit() {
    let (mut config, requests, server) = mock(vec![
        (
            200,
            openai_text("Earlier timeout was confirmed; references need refresh."),
        ),
        (200, openai_text("Continuing from the summary.")),
    ]);
    config.context_window_tokens = Some(500);
    let messages = vec![
        AgentMessage::User {
            text: "old log question".into(),
        },
        AgentMessage::Assistant {
            text: "old finding".into(),
            reasoning: String::new(),
            thinking: Vec::new(),
            calls: Vec::new(),
        },
        AgentMessage::User {
            text: "continue".into(),
        },
    ];
    let run = start_run(config, messages, Vec::new(), String::new(), false);
    let mut compacted = false;
    loop {
        match tokio::time::timeout(Duration::from_secs(10), run.events.recv())
            .await
            .unwrap()
            .unwrap()
        {
            AgentEvent::ContextCompacted { summary, through } => {
                compacted = summary.contains("timeout") && through == 2;
            }
            AgentEvent::Finished(status, error) => {
                assert_eq!(status, RunStatus::Complete, "{error}");
                break;
            }
            _ => {}
        }
    }
    server.join().unwrap();
    assert!(compacted);
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].get("tools").is_none());
    assert_eq!(requests[1]["messages"].as_array().unwrap().len(), 3);
    assert!(
        requests[1]["messages"][2]["content"]
            .as_str()
            .unwrap()
            .contains("continue")
    );
}

fn mock(
    responses: Vec<(u16, String)>,
) -> (
    ProviderConfig,
    Arc<Mutex<Vec<Value>>>,
    std::thread::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let config = ProviderConfig {
        base_url: format!("http://{}/v1", listener.local_addr().unwrap()),
        model: "test-model".into(),
        api_key: "local-test-key".into(),
        ..Default::default()
    };
    let requests = Arc::new(Mutex::new(Vec::new()));
    let output = requests.clone();
    let task = std::thread::spawn(move || {
        for (status, response) in responses {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut bytes = Vec::new();
            let (header_end, length) = loop {
                let mut chunk = [0; 1024];
                let count = socket.read(&mut chunk).unwrap();
                assert!(count > 0);
                bytes.extend_from_slice(&chunk[..count]);
                if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&bytes[..end]);
                    let len = header
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap())
                        })
                        .unwrap();
                    break (end + 4, len);
                }
            };
            while bytes.len() < header_end + length {
                let mut chunk = [0; 4096];
                let count = socket.read(&mut chunk).unwrap();
                assert!(count > 0);
                bytes.extend_from_slice(&chunk[..count]);
            }
            output
                .lock()
                .unwrap()
                .push(serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap());
            write!(socket,"HTTP/1.1 {status} Test\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",response.len()).unwrap();
            // Exercise arbitrary UTF-8 / JSON packet boundaries.
            for chunk in response.as_bytes().chunks(7) {
                if socket.write_all(chunk).is_err() {
                    break;
                }
            }
        }
    });
    (config, requests, task)
}
fn event(value: Value) -> String {
    format!("data: {value}\n\n")
}
fn openai_tool(id: &str, args: &str) -> String {
    openai_named_tool(id, "list_logs", args)
}
fn openai_named_tool(id: &str, name: &str, args: &str) -> String {
    event(
        json!({"choices":[{"delta":{"content":"检查日志","tool_calls":[{"index":0,"id":id,"type":"function","function":{"name":name,"arguments":args}}]},"finish_reason":"tool_calls"}]}),
    ) + "data: [DONE]\n\n"
}
fn openai_text(text: &str) -> String {
    event(json!({"choices":[{"delta":{"content":text},"finish_reason":"stop"}]}))
        + "data: [DONE]\n\n"
}

#[tokio::test]
async fn ask_user_pauses_and_resumes_the_same_run() {
    let (config, requests, server) = mock(vec![
        (
            200,
            openai_named_tool(
                "question-1",
                "ask_user",
                r#"{"question":"Which service?","options":[{"id":"api","label":"API"},{"id":"worker","label":"Worker"}],"allow_free_text":true}"#,
            ),
        ),
        (200, openai_text("Continuing with the worker service.")),
    ]);
    let run = start_run(
        config,
        vec![AgentMessage::User {
            text: "Investigate the failure".into(),
        }],
        vec![],
        String::new(),
        false,
    );
    let question = loop {
        match tokio::time::timeout(Duration::from_secs(5), run.events.recv())
            .await
            .unwrap()
            .unwrap()
        {
            AgentEvent::QuestionRequested(question) => break question,
            AgentEvent::Finished(status, error) => panic!("finished early: {status:?} {error}"),
            _ => {}
        }
    };
    assert_eq!(question.call_id, "question-1");
    assert_eq!(question.options.len(), 2);
    run.replies
        .send((
            question.call_id,
            ToolResult::ok(json!({"option_id":"worker","answer":"Worker"})),
        ))
        .await
        .unwrap();
    loop {
        if let AgentEvent::Finished(status, error) =
            tokio::time::timeout(Duration::from_secs(5), run.events.recv())
                .await
                .unwrap()
                .unwrap()
        {
            assert_eq!(status, RunStatus::Complete, "{error}");
            break;
        }
    }
    server.join().unwrap();
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[1].to_string().contains("worker"));
}

#[tokio::test]
async fn openai_runs_tools_returns_results_and_does_not_repeat_call_ids() {
    let (config, requests, server) = mock(vec![
        (
            200,
            openai_named_tool("load-vclogg", "load_tool_group", r#"{"group":"vclogg"}"#),
        ),
        (200, openai_tool("one", "{}")),
        (200, openai_tool("one", "{}")),
        (200, openai_text("日志分析完成")),
    ]);
    let run = start_run(
        config,
        vec![AgentMessage::User {
            text: "分析日志".into(),
        }],
        vec![],
        String::new(),
        false,
    );
    let mut calls = 0;
    let mut final_text = String::new();
    loop {
        match tokio::time::timeout(Duration::from_secs(5), run.events.recv())
            .await
            .unwrap()
            .unwrap()
        {
            AgentEvent::ToolStarted(call) => {
                calls += 1;
                run.replies
                    .send((call.id, ToolResult::ok(json!({"files":["app.log"]}))))
                    .await
                    .unwrap();
            }
            AgentEvent::Text(text) => final_text.push_str(&text),
            AgentEvent::Finished(status, error) => {
                assert_eq!(status, RunStatus::Complete, "{error}");
                break;
            }
            _ => {}
        }
    }
    server.join().unwrap();
    assert_eq!(calls, 1);
    assert!(final_text.contains("日志分析完成"));
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 4);
    let tool_result = requests[2]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|message| message["role"] == "tool")
        .unwrap();
    assert!(tool_result["content"].as_str().unwrap().contains("app.log"));
}

#[tokio::test]
async fn tool_groups_are_loaded_only_after_discovery() {
    let (config, requests, server) = mock(vec![
        (
            200,
            openai_named_tool("load-vclogg", "load_tool_group", r#"{"group":"vclogg"}"#),
        ),
        (200, openai_text("工具已就绪")),
    ]);
    let run = start_run(
        config,
        vec![AgentMessage::User {
            text: "读取更多证据".into(),
        }],
        vec![],
        String::new(),
        false,
    );
    loop {
        if let AgentEvent::Finished(status, error) =
            tokio::time::timeout(Duration::from_secs(5), run.events.recv())
                .await
                .unwrap()
                .unwrap()
        {
            assert_eq!(status, RunStatus::Complete, "{error}");
            break;
        }
    }
    server.join().unwrap();
    let requests = requests.lock().unwrap();
    let names = |request: &Value| {
        request["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["function"]["name"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    let first = names(&requests[0]);
    assert_eq!(first.len(), 3);
    assert!(first.iter().any(|name| name == "get_context"));
    assert!(first.iter().any(|name| name == "ask_user"));
    assert!(!first.iter().any(|name| name == "read_logs"));
    let second = names(&requests[1]);
    assert!(second.iter().any(|name| name == "read_logs"));
    assert!(second.iter().any(|name| name == "open_file"));
    assert!(second.iter().any(|name| name == "set_marks"));
}

#[tokio::test]
async fn shell_reads_absolute_file_without_workspace_or_root_discovery() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("日志 sample.log");
    std::fs::write(&path, "absolute-file-evidence\n").unwrap();
    let command = format!(
        "{} \"{}\"",
        if cfg!(windows) { "type" } else { "cat" },
        path.display()
    );
    let arguments = json!({"command":command}).to_string();
    let (config, requests, server) = mock(vec![
        (
            200,
            openai_named_tool("load-shell", "load_tool_group", r#"{"group":"shell"}"#),
        ),
        (200, openai_named_tool("read-file", "shell", &arguments)),
        (200, openai_text("已读取")),
    ]);
    let run = start_run(
        config,
        vec![AgentMessage::User {
            text: "读取当前文件".into(),
        }],
        vec![],
        format!("Current file: {}", path.display()),
        false,
    );
    loop {
        match tokio::time::timeout(Duration::from_secs(5), run.events.recv())
            .await
            .unwrap()
            .unwrap()
        {
            AgentEvent::QuestionRequested(_) | AgentEvent::ToolStarted(_) => {
                panic!("Reading an absolute path must not ask for a workspace or host action")
            }
            AgentEvent::Finished(status, error) => {
                assert_eq!(status, RunStatus::Complete, "{error}");
                break;
            }
            _ => {}
        }
    }
    server.join().unwrap();
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    assert!(requests[0]["tools"].to_string().contains("shell"));
    let result = requests[2]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|message| message["role"] == "tool" && message["tool_call_id"] == "read-file")
        .unwrap();
    assert!(
        result["content"]
            .as_str()
            .unwrap()
            .contains("absolute-file-evidence")
    );
    assert!(
        !requests[0]["messages"]
            .to_string()
            .contains("先用 list_source_workspaces 取得 root")
    );
}
#[tokio::test]
async fn anthropic_native_protocol_returns_tool_result_blocks() {
    let response=[
        event(json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"tool-a","name":"list_logs","input":{}}})),
        event(json!({"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{}"}})),
        event(json!({"type":"message_delta","delta":{"stop_reason":"tool_use"}})),event(json!({"type":"message_stop"})),
    ].concat();
    let done = event(
        json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"完成"}}),
    ) + &event(json!({"type":"message_delta","delta":{"stop_reason":"end_turn"}}))
        + &event(json!({"type":"message_stop"}));
    let (mut config, requests, server) = mock(vec![(200, response), (200, done)]);
    config.protocol = Protocol::Anthropic;
    let run = start_run(
        config,
        vec![AgentMessage::User {
            text: "分析".into(),
        }],
        vec![],
        String::new(),
        false,
    );
    loop {
        match tokio::time::timeout(Duration::from_secs(5), run.events.recv())
            .await
            .unwrap()
            .unwrap()
        {
            AgentEvent::ToolStarted(call) => run
                .replies
                .send((call.id, ToolResult::ok(json!({"files":[]}))))
                .await
                .unwrap(),
            AgentEvent::Finished(status, error) => {
                assert_eq!(status, RunStatus::Complete, "{error}");
                break;
            }
            _ => {}
        }
    }
    server.join().unwrap();
    let requests = requests.lock().unwrap();
    assert!(requests[0]["tools"][0]["input_schema"].is_object());
    assert_eq!(
        requests[1]["messages"][2]["content"][0]["type"],
        "tool_result"
    );
    assert_eq!(
        requests[1]["messages"][2]["content"][0]["tool_use_id"],
        "tool-a"
    );
}
#[tokio::test]
async fn malformed_arguments_are_returned_as_errors_without_execution() {
    let (config, requests, server) = mock(vec![
        (200, openai_tool("bad", "{incomplete")),
        (200, openai_text("请修正参数")),
    ]);
    let run = start_run(
        config,
        vec![AgentMessage::User {
            text: "test".into(),
        }],
        vec![],
        String::new(),
        false,
    );
    loop {
        match run.events.recv().await.unwrap() {
            AgentEvent::ToolStarted(_) => panic!("Invalid call executed"),
            AgentEvent::Finished(status, _) => {
                assert_eq!(status, RunStatus::Complete);
                break;
            }
            _ => {}
        }
    }
    server.join().unwrap();
    assert!(
        requests.lock().unwrap()[1]["messages"][3]["content"]
            .as_str()
            .unwrap()
            .contains("error")
    );
}
#[tokio::test]
async fn auth_rate_limit_and_disconnect_are_visible_failures() {
    for protocol in [Protocol::OpenAi, Protocol::Anthropic] {
        for (status, response) in [
            (401, String::new()),
            (429, String::new()),
            (
                200,
                event(json!({"choices":[{"delta":{"content":"partial"}}]})),
            ),
        ] {
            let (mut config, _, server) = mock(vec![(status, response)]);
            config.protocol = protocol;
            let run = start_run(
                config,
                vec![AgentMessage::User {
                    text: "test".into(),
                }],
                vec![],
                String::new(),
                false,
            );
            loop {
                if let AgentEvent::Finished(status, error) = run.events.recv().await.unwrap() {
                    assert_eq!(status, RunStatus::Failed);
                    assert!(!error.is_empty());
                    break;
                }
            }
            server.join().unwrap();
        }
    }
}
#[tokio::test]
async fn cancellation_aborts_waiting_for_tool_without_another_request() {
    let (config, requests, server) = mock(vec![
        (
            200,
            openai_named_tool("load-vclogg", "load_tool_group", r#"{"group":"vclogg"}"#),
        ),
        (200, openai_tool("cancel", "{}")),
    ]);
    let run = start_run(
        config,
        vec![AgentMessage::User {
            text: "test".into(),
        }],
        vec![],
        String::new(),
        false,
    );
    loop {
        match run.events.recv().await.unwrap() {
            AgentEvent::ToolStarted(_) => run.cancellation.cancel(),
            AgentEvent::Finished(status, _) => {
                assert_eq!(status, RunStatus::Interrupted);
                break;
            }
            _ => {}
        }
    }
    server.join().unwrap();
    assert_eq!(requests.lock().unwrap().len(), 2);
}
#[test]
fn skill_references_cannot_escape_and_refresh_preserves_identity() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("skill");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("SKILL.md"),"---\nname: log-review\ndescription: >\n  Investigate errors\n  with context\n---\nUse log tools").unwrap();
    let mut skill = import_skill(&root).unwrap();
    assert_eq!(skill.description, "Investigate errors with context");
    let id = skill.id.clone();
    refresh_skill(&mut skill).unwrap();
    assert_eq!(id, skill.id);
    std::fs::write(dir.path().join("secret"), "outside").unwrap();
    assert!(read_skill_file(&skill, "../secret").is_err());
    assert!(read_skill_file(&skill, "/etc/passwd").is_err());
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(dir.path().join("secret"), root.join("link")).unwrap();
        assert!(read_skill_file(&skill, "link").is_err());
    }
    skill.enabled = false;
    assert!(read_skill_file(&skill, "SKILL.md").is_err());
}
#[test]
fn settings_are_private_and_never_debug_print_credentials() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ai/settings.json");
    let config = ProviderConfig {
        api_key: "secret-key".into(),
        ..Default::default()
    };
    assert!(!format!("{config:?}").contains("secret-key"));
    let mut settings = AiSettings::default();
    settings.providers = vec![config];
    settings.save(&path).unwrap();
    assert_eq!(
        AiSettings::load(&path).unwrap().providers[0].api_key,
        "secret-key"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    assert!(settings.save(&path.join("invalid")).is_err());
    assert_eq!(AiSettings::load(&path).unwrap().providers.len(), 1);
}
#[test]
fn recovering_a_pending_mutation_never_replays_it() {
    let mut conversation = Conversation {
        status: RunStatus::Running,
        messages: vec![
            AgentMessage::User {
                text: "mark".into(),
            },
            AgentMessage::Assistant {
                reasoning: String::new(),
                thinking: Vec::new(),
                text: String::new(),
                calls: vec![ToolCall {
                    id: "one".into(),
                    name: "set_marks".into(),
                    arguments: json!({}),
                }],
            },
        ],
        ..Default::default()
    };
    conversation.recover();
    assert_eq!(conversation.status, RunStatus::Interrupted);
    assert!(matches!(&conversation.messages[2],AgentMessage::Tool{result,..} if result.is_error));
    let n = conversation.messages.len();
    conversation.recover();
    assert_eq!(n, conversation.messages.len());
}

#[tokio::test]
async fn both_protocols_assemble_interleaved_chinese_and_multiple_tool_arguments() {
    for protocol in [Protocol::OpenAi, Protocol::Anthropic] {
        let fragments = ["{\"scope\":\"current\",\"query\":\"", "网络", "异常\"}"];
        let (load, response, done) = if protocol == Protocol::OpenAi {
            let mut response = event(
                json!({"choices":[{"delta":{"content":"开始排查。","tool_calls":[
                    {"index":0,"id":"first","function":{"name":"list_logs","arguments":"{"}},
                    {"index":1,"id":"second","function":{"name":"search_logs","arguments":fragments[0]}}
                ]}}]}),
            );
            response += &event(
                json!({"choices":[{"delta":{"content":"查找网络异常。","tool_calls":[
                    {"index":1,"function":{"arguments":fragments[1]}},{"index":0,"function":{"arguments":"}"}}
                ]}}]}),
            );
            response += &event(
                json!({"choices":[{"delta":{"tool_calls":[{"index":1,"function":{"arguments":fragments[2]}}]},"finish_reason":"tool_calls"}]}),
            );
            (
                openai_named_tool("load-vclogg", "load_tool_group", r#"{"group":"vclogg"}"#),
                response,
                openai_text("总结完成。"),
            )
        } else {
            let load = event(
                json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"load-vclogg","name":"load_tool_group","input":{}}}),
            ) + &event(
                json!({"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"group\":\"vclogg\"}"}}),
            ) + &event(json!({"type":"content_block_stop","index":0}))
                + &event(json!({"type":"message_delta","delta":{"stop_reason":"tool_use"}}))
                + &event(json!({"type":"message_stop"}));
            let mut response = event(
                json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":"开始排查。"}}),
            );
            response += &event(json!({"type":"content_block_stop","index":0}));
            for (ix, id, name, parts) in [
                (1, "first", "list_logs", vec!["{", "}"]),
                (2, "second", "search_logs", fragments.to_vec()),
            ] {
                response += &event(
                    json!({"type":"content_block_start","index":ix,"content_block":{"type":"tool_use","id":id,"name":name,"input":{}}}),
                );
                for part in parts {
                    response += &event(
                        json!({"type":"content_block_delta","index":ix,"delta":{"type":"input_json_delta","partial_json":part}}),
                    );
                }
                response += &event(json!({"type":"content_block_stop","index":ix}));
            }
            response += &event(json!({"type":"message_delta","delta":{"stop_reason":"tool_use"}}));
            response += &event(json!({"type":"message_stop"}));
            let done = event(
                json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":"总结完成。"}}),
            ) + &event(
                json!({"type":"message_delta","delta":{"stop_reason":"end_turn"}}),
            ) + &event(json!({"type":"message_stop"}));
            (load, response, done)
        };
        let (mut config, requests, server) = mock(vec![(200, load), (200, response), (200, done)]);
        config.protocol = protocol;
        let run = start_run(
            config,
            vec![AgentMessage::User {
                text: "排查问题".into(),
            }],
            vec![],
            String::new(),
            false,
        );
        let mut calls = Vec::new();
        let mut text = String::new();
        loop {
            match tokio::time::timeout(Duration::from_secs(5), run.events.recv())
                .await
                .unwrap()
                .unwrap()
            {
                AgentEvent::Text(delta) => text.push_str(&delta),
                AgentEvent::ToolStarted(call) => {
                    calls.push(call.clone());
                    run.replies
                        .send((call.id, ToolResult::ok(json!({"rows":[]}))))
                        .await
                        .unwrap();
                }
                AgentEvent::Finished(status, error) => {
                    assert_eq!(status, RunStatus::Complete, "{error}");
                    break;
                }
                _ => {}
            }
        }
        assert_eq!(
            calls.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
            ["first", "second"]
        );
        assert_eq!(calls[1].arguments["query"], "网络异常");
        assert!(text.contains("开始排查。") && text.contains("总结完成。"));
        server.join().unwrap();
        if protocol == Protocol::Anthropic {
            let messages = requests.lock().unwrap()[2]["messages"]
                .as_array()
                .unwrap()
                .clone();
            let result = messages
                .iter()
                .rev()
                .find(|message| message["role"] == "user" && message["content"].is_array())
                .unwrap();
            assert_eq!(result["content"].as_array().unwrap().len(), 2);
        }
    }
}

#[tokio::test]
async fn more_than_twenty_tool_round_trips_continue_until_the_answer() {
    let mut responses = (0..25)
        .map(|ix| (200, openai_tool(&format!("call-{ix}"), "{}")))
        .collect::<Vec<_>>();
    responses.push((200, openai_text("Investigation complete")));
    let (config, requests, server) = mock(responses);
    let run = start_run(
        config,
        vec![AgentMessage::User {
            text: "Investigate".into(),
        }],
        vec![],
        String::new(),
        false,
    );
    loop {
        match run.events.recv().await.unwrap() {
            AgentEvent::ToolStarted(call) => run
                .replies
                .send((call.id, ToolResult::ok(json!({}))))
                .await
                .unwrap(),
            AgentEvent::Finished(status, _) => {
                assert_eq!(status, RunStatus::Complete);
                break;
            }
            _ => {}
        }
    }
    server.join().unwrap();
    assert_eq!(requests.lock().unwrap().len(), 26);
}

#[test]
fn recovery_pairs_reused_ids_with_their_own_assistant_response() {
    let assistant = AgentMessage::Assistant {
        reasoning: String::new(),
        thinking: Vec::new(),
        text: String::new(),
        calls: vec![ToolCall {
            id: "reused".into(),
            name: "set_marks".into(),
            arguments: json!({}),
        }],
    };
    let mut conversation = Conversation {
        status: RunStatus::Running,
        messages: vec![
            assistant.clone(),
            AgentMessage::Tool {
                call_id: "reused".into(),
                name: "set_marks".into(),
                result: ToolResult::ok(json!({})),
            },
            AgentMessage::User {
                text: "Again".into(),
            },
            assistant,
            AgentMessage::Assistant {
                reasoning: String::new(),
                thinking: Vec::new(),
                text: "Partial output".into(),
                calls: vec![],
            },
        ],
        ..Default::default()
    };
    conversation.recover();
    assert!(
        matches!(&conversation.messages[4], AgentMessage::Tool { result, .. } if result.is_error)
    );
    assert!(
        matches!(&conversation.messages[5], AgentMessage::Assistant { text, .. } if text == "Partial output")
    );
}

#[cfg(unix)]
#[test]
fn replacing_imported_skill_root_with_a_symlink_is_rejected() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("skill");
    let outside = directory.path().join("outside");
    std::fs::create_dir(&root).unwrap();
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(root.join("SKILL.md"), "Original skill").unwrap();
    std::fs::write(outside.join("SKILL.md"), "Outside content").unwrap();
    let skill = import_skill(&root).unwrap();
    std::fs::rename(&root, directory.path().join("old-skill")).unwrap();
    std::os::unix::fs::symlink(outside, root).unwrap();
    assert!(read_skill_file(&skill, "SKILL.md").is_err());
}

#[tokio::test]
async fn anthropic_malformed_tool_json_recovers_with_valid_wire_blocks() {
    let response = event(
        json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"bad-json","name":"list_logs","input":{}}}),
    ) + &event(
        json!({"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{bad"}}),
    ) + &event(json!({"type":"message_delta","delta":{"stop_reason":"tool_use"}}))
        + &event(json!({"type":"message_stop"}));
    let done = event(
        json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":"参数需要修正"}}),
    ) + &event(json!({"type":"message_stop"}));
    let (mut config, requests, server) = mock(vec![(200, response), (200, done)]);
    config.protocol = Protocol::Anthropic;
    let run = start_run(
        config,
        vec![AgentMessage::User {
            text: "检查".into(),
        }],
        vec![],
        String::new(),
        false,
    );
    loop {
        match run.events.recv().await.unwrap() {
            AgentEvent::ToolStarted(_) => panic!("Malformed tool was executed"),
            AgentEvent::Finished(status, error) => {
                assert_eq!(status, RunStatus::Complete, "{error}");
                break;
            }
            _ => {}
        }
    }
    server.join().unwrap();
    let requests = requests.lock().unwrap();
    assert!(requests[1]["messages"][1]["content"][0]["input"].is_object());
    assert_eq!(requests[1]["messages"][2]["content"][0]["is_error"], true);
}

#[tokio::test]
async fn reasoning_streams_and_survives_tool_round_trips() {
    for protocol in [Protocol::OpenAi, Protocol::Anthropic] {
        let first = if protocol == Protocol::OpenAi {
            event(json!({"choices":[{"delta":{"reasoning_content":"检查中文日志"}}]}))
                + &openai_tool("one", "{}")
        } else {
            [
                event(json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}})),
                event(json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"检查中文日志"}})),
                event(json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"signed-block"}})),
                event(json!({"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"one","name":"list_logs","input":{}}})),
                event(json!({"type":"message_delta","delta":{"stop_reason":"tool_use"}})),
                event(json!({"type":"message_stop"})),
            ].concat()
        };
        let last = if protocol == Protocol::OpenAi {
            openai_text("完成")
        } else {
            event(
                json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"完成"}}),
            ) + &event(json!({"type":"message_stop"}))
        };
        let (mut config, requests, server) = mock(vec![(200, first), (200, last)]);
        config.protocol = protocol;
        let run = start_run(
            config,
            vec![AgentMessage::User {
                text: "分析".into(),
            }],
            vec![],
            String::new(),
            false,
        );
        let mut thinking = String::new();
        loop {
            match tokio::time::timeout(Duration::from_secs(5), run.events.recv())
                .await
                .unwrap()
                .unwrap()
            {
                AgentEvent::Thinking(delta) => thinking.push_str(&delta),
                AgentEvent::ToolStarted(call) => {
                    assert_eq!(thinking, "检查中文日志");
                    run.replies
                        .send((call.id, ToolResult::ok(json!({"files":[]}))))
                        .await
                        .unwrap();
                }
                AgentEvent::Finished(status, error) => {
                    assert_eq!(status, RunStatus::Complete, "{error}");
                    break;
                }
                _ => {}
            }
        }
        server.join().unwrap();
        let requests = requests.lock().unwrap();
        if protocol == Protocol::OpenAi {
            assert_eq!(requests[1]["messages"][2]["reasoning_content"], thinking);
        } else {
            assert_eq!(
                requests[1]["messages"][1]["content"][0],
                json!({"type":"thinking","thinking":thinking,"signature":"signed-block"})
            );
        }
    }
}

#[tokio::test]
async fn empty_responses_fail_and_service_errors_are_redacted() {
    for (status, response, expected) in [
        (200, openai_text(""), "no answer"),
        (
            200,
            event(
                json!({"choices":[{"delta":{"reasoning_content":"正在思考"},"finish_reason":"length"}]}),
            ),
            "Output limit",
        ),
        (
            400,
            json!({"error":{"message":"Unsupported tools local-test-key"}}).to_string(),
            "Unsupported tools [redacted]",
        ),
    ] {
        let (config, _, server) = mock(vec![(status, response)]);
        let run = start_run(config, vec![], vec![], String::new(), true);
        loop {
            match tokio::time::timeout(Duration::from_secs(5), run.events.recv())
                .await
                .unwrap()
                .unwrap()
            {
                AgentEvent::Finished(status, error) => {
                    assert_eq!(status, RunStatus::Failed);
                    assert!(error.contains(expected), "{error}");
                    assert!(!error.contains("local-test-key"));
                    break;
                }
                AgentEvent::ToolStarted(_) => panic!("Incomplete response must not execute tools"),
                _ => {}
            }
        }
        server.join().unwrap();
    }
}

#[test]
fn log_citations_round_trip_without_becoming_external_access() {
    let reference = vclogg_ai::LogReference {
        document_id: 42,
        version: "快照 / ?&v=1".into(),
        line: 17,
    };
    let url = reference.url();
    assert_eq!(vclogg_ai::LogReference::from_url(&url), Some(reference));
    for invalid in [
        "https://log?document_id=42&version=v1&line=17",
        "file:///private/log",
        "vclogg://log?document_id=0&version=v1&line=1",
        "vclogg://log?document_id=1&version=v1&line=0",
        "vclogg://log?document_id=1&version=&line=1",
    ] {
        assert!(vclogg_ai::LogReference::from_url(invalid).is_none());
    }
}
