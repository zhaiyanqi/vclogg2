use crate::{
    AgentEvent, AgentMessage, Cancellation, Protocol, ProviderConfig, ToolCall, tool_definitions,
};
use anyhow::{Context as _, Result, bail};
use futures_util::StreamExt as _;
use serde_json::{Value, json};
use std::{collections::BTreeMap, time::Duration};

pub(crate) fn request_body(
    config: &ProviderConfig,
    system: &str,
    messages: &[AgentMessage],
    with_tools: bool,
) -> Value {
    let definitions = tool_definitions();
    let mut wire = Vec::<Value>::new();
    if config.protocol == Protocol::OpenAi {
        wire.push(json!({"role":"system","content":system}));
    }
    for message in messages {
        match (config.protocol, message) {
            (_, AgentMessage::User { text }) => wire.push(json!({"role":"user","content":text})),
            (
                Protocol::OpenAi,
                AgentMessage::Assistant {
                    text,
                    calls,
                    reasoning,
                    ..
                },
            ) => {
                let mut value = json!({"role":"assistant","content":text});
                if !reasoning.is_empty() {
                    value["reasoning_content"] = json!(reasoning);
                }
                if !calls.is_empty() {
                    value["tool_calls"] = json!(calls.iter().map(|c| json!({"id":c.id,"type":"function","function":{"name":c.name,"arguments":c.arguments.as_str().map(str::to_owned).unwrap_or_else(|| c.arguments.to_string())}})).collect::<Vec<_>>());
                }
                wire.push(value);
            }
            (
                Protocol::OpenAi,
                AgentMessage::Tool {
                    call_id, result, ..
                },
            ) => wire.push(
                json!({"role":"tool","tool_call_id":call_id,"content":result.value.to_string()}),
            ),
            (
                Protocol::Anthropic,
                AgentMessage::Assistant {
                    text,
                    calls,
                    thinking,
                    ..
                },
            ) => {
                let mut content = thinking.clone();
                if !text.is_empty() {
                    content.push(json!({"type":"text","text":text}));
                }
                for call in calls {
                    content.push(json!({"type":"tool_use","id":call.id,"name":call.name,"input":if call.arguments.is_object() {call.arguments.clone()} else {json!({})}}));
                }
                if content.is_empty() {
                    content.push(json!({"type":"text","text":"[Interrupted response]"}));
                }
                wire.push(json!({"role":"assistant","content":content}));
            }
            (
                Protocol::Anthropic,
                AgentMessage::Tool {
                    call_id, result, ..
                },
            ) => {
                let block = json!({"type":"tool_result","tool_use_id":call_id,"is_error":result.is_error,"content":result.value.to_string()});
                if let Some(last) = wire
                    .last_mut()
                    .filter(|last| last["role"] == "user" && last["content"].is_array())
                {
                    last["content"]
                        .as_array_mut()
                        .expect("array checked")
                        .push(block);
                } else {
                    wire.push(json!({"role":"user","content":[block]}));
                }
            }
        }
    }
    let mut body = json!({"model":config.model,"messages":wire,"stream":true});
    // OpenAI-compatible services choose their own output default. Anthropic
    // requires max_tokens; use the discovered limit when available.
    if config.protocol == Protocol::Anthropic {
        body["max_tokens"] = json!(config.max_output_tokens.max(1));
    }
    // Providers may require schemas while replaying tool-use history even when
    // the final request is reserved for prose. Keep schemas and disable new calls.
    let has_tool_history = messages
        .iter()
        .any(|m| matches!(m, AgentMessage::Assistant { calls, .. } if !calls.is_empty()));
    match config.protocol {
        Protocol::OpenAi => {
            if with_tools || has_tool_history {
                body["tools"] = json!(definitions.iter().map(|d| json!({"type":"function","function":{"name":d.name,"description":d.description,"parameters":d.parameters}})).collect::<Vec<_>>());
            }
            if !with_tools && has_tool_history {
                body["tool_choice"] = json!("none");
            }
        }
        Protocol::Anthropic => {
            body["system"] = json!(system);
            if with_tools || has_tool_history {
                body["tools"] = json!(definitions.iter().map(|d| json!({"name":d.name,"description":d.description,"input_schema":d.parameters})).collect::<Vec<_>>());
            }
            if !with_tools && has_tool_history {
                body["tool_choice"] = json!({"type":"none"});
            }
        }
    }
    body
}

#[derive(Default)]
struct PartialCall {
    id: String,
    name: String,
    arguments: String,
    initial: Option<Value>,
}
#[derive(Default)]
struct Completion {
    text: String,
    reasoning: String,
    thinking: BTreeMap<usize, Value>,
    calls: BTreeMap<usize, PartialCall>,
    finished: bool,
    stop_reason: String,
}
impl Completion {
    fn event(&mut self, protocol: Protocol, payload: &str) -> Result<Option<String>> {
        if payload.trim() == "[DONE]" {
            return Ok(None);
        }
        let value: Value = serde_json::from_str(payload).context("Invalid SSE JSON")?;
        if value.get("error").is_some() || value["type"] == "error" {
            bail!("Provider reported a stream error");
        }
        let mut text = None;
        let mut reasoning = None;
        match protocol {
            Protocol::OpenAi => {
                if let Some(choice) = value["choices"].as_array().and_then(|v| v.first()) {
                    if let Some(reason) = choice["finish_reason"].as_str() {
                        self.finished = true;
                        self.stop_reason = reason.into();
                    }
                    text = choice["delta"]["content"].as_str().map(str::to_owned);
                    reasoning = choice["delta"]["reasoning_content"]
                        .as_str()
                        .or_else(|| choice["delta"]["reasoning"].as_str())
                        .map(str::to_owned);
                    for delta in choice["delta"]["tool_calls"]
                        .as_array()
                        .into_iter()
                        .flatten()
                    {
                        let ix = delta["index"].as_u64().context("Missing tool index")? as usize;
                        if ix > 127 {
                            bail!("Too many tool calls");
                        }
                        let call = self.calls.entry(ix).or_default();
                        if let Some(id) = delta["id"].as_str() {
                            call.id.push_str(id);
                        }
                        if let Some(name) = delta["function"]["name"].as_str() {
                            call.name.push_str(name);
                        }
                        if let Some(args) = delta["function"]["arguments"].as_str() {
                            call.arguments.push_str(args);
                        }
                    }
                }
            }
            Protocol::Anthropic => {
                let ix = value["index"].as_u64().unwrap_or(0) as usize;
                if ix > 127 {
                    bail!("Too many content blocks");
                }
                match value["type"].as_str() {
                    Some("content_block_start")
                        if matches!(
                            value["content_block"]["type"].as_str(),
                            Some("thinking" | "redacted_thinking")
                        ) =>
                    {
                        reasoning = value["content_block"]["thinking"]
                            .as_str()
                            .map(str::to_owned);
                        self.thinking.insert(ix, value["content_block"].clone());
                    }
                    Some("content_block_start") if value["content_block"]["type"] == "tool_use" => {
                        if ix > 127 {
                            bail!("Too many tool calls");
                        }
                        let block = &value["content_block"];
                        self.calls.insert(
                            ix,
                            PartialCall {
                                id: block["id"].as_str().unwrap_or_default().into(),
                                name: block["name"].as_str().unwrap_or_default().into(),
                                initial: Some(block["input"].clone()),
                                ..Default::default()
                            },
                        );
                    }
                    Some("content_block_start") if value["content_block"]["type"] == "text" => {
                        text = value["content_block"]["text"].as_str().map(str::to_owned)
                    }
                    Some("content_block_delta") => match value["delta"]["type"].as_str() {
                        Some("thinking_delta" | "signature_delta") => {
                            let is_thinking = value["delta"]["type"] == "thinking_delta";
                            let field = if is_thinking { "thinking" } else { "signature" };
                            let delta = value["delta"][field].as_str().unwrap_or_default();
                            let block = self
                                .thinking
                                .get_mut(&ix)
                                .context("Thinking delta has no start")?;
                            if !block[field].is_string() {
                                block[field] = json!("");
                            }
                            if let Value::String(text) = &mut block[field] {
                                text.push_str(delta);
                            }
                            if is_thinking {
                                reasoning = Some(delta.to_owned());
                            }
                        }
                        Some("text_delta") => {
                            text = value["delta"]["text"].as_str().map(str::to_owned)
                        }
                        Some("input_json_delta") => {
                            let call =
                                self.calls.get_mut(&ix).context("Tool delta has no start")?;
                            call.arguments.push_str(
                                value["delta"]["partial_json"].as_str().unwrap_or_default(),
                            );
                        }
                        _ => {}
                    },
                    Some("message_delta") => {
                        if let Some(reason) = value["delta"]["stop_reason"].as_str() {
                            self.stop_reason = reason.into();
                        }
                    }
                    Some("message_stop") => self.finished = true,
                    _ => {}
                }
            }
        }
        if let Some(reasoning) = reasoning {
            self.reasoning.push_str(&reasoning);
        }
        if let Some(text) = &text {
            self.text.push_str(text);
        }
        if self.text.len()
            + self.reasoning.len()
            + self
                .thinking
                .values()
                .map(|v| {
                    ["thinking", "signature", "data"]
                        .iter()
                        .map(|key| v[key].as_str().map_or(0, str::len))
                        .sum::<usize>()
                })
                .sum::<usize>()
            + self
                .calls
                .values()
                .map(|c| c.arguments.len() + c.id.len() + c.name.len())
                .sum::<usize>()
            > 512 * 1024
        {
            bail!("Response exceeded 512 KiB");
        }
        Ok(text)
    }
    fn finish(self) -> Result<AgentMessage> {
        if !self.finished {
            bail!("Stream disconnected before completion; partial output retained");
        }
        if matches!(self.stop_reason.as_str(), "length" | "max_tokens") {
            bail!(
                "Output limit reached. Increase the model output limit; incomplete tools were not executed"
            );
        }
        let mut ids = std::collections::BTreeSet::new();
        let calls = self
            .calls
            .into_values()
            .map(|call| {
                if call.id.is_empty() || call.name.is_empty() || !ids.insert(call.id.clone()) {
                    bail!("Invalid or duplicate tool call identity");
                }
                let arguments = if call.arguments.is_empty() {
                    call.initial.unwrap_or(json!({}))
                } else {
                    serde_json::from_str(&call.arguments).unwrap_or(Value::String(call.arguments))
                };
                Ok(ToolCall {
                    id: call.id,
                    name: call.name,
                    arguments,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        if self.text.trim().is_empty() && calls.is_empty() {
            bail!(
                "Model returned no answer or tool calls. Check model compatibility and output limit"
            );
        }
        Ok(AgentMessage::Assistant {
            text: self.text,
            reasoning: self.reasoning,
            thinking: self.thinking.into_values().collect(),
            calls,
        })
    }
}

/// SSE byte decoder. UTF-8 is decoded only after a complete event, not per network packet.
#[derive(Default)]
pub(crate) struct SseDecoder {
    buffer: Vec<u8>,
    data: Vec<String>,
}
impl SseDecoder {
    pub(crate) fn push(&mut self, bytes: &[u8]) -> Result<Vec<String>> {
        self.buffer.extend_from_slice(bytes);
        if self.buffer.len() > 1024 * 1024 {
            bail!("SSE frame too large");
        }
        let mut events = Vec::new();
        let mut consumed = 0;
        while let Some(n) = self.buffer[consumed..].iter().position(|b| *b == b'\n') {
            let end = consumed + n;
            let line = std::str::from_utf8(&self.buffer[consumed..end])
                .context("Invalid stream UTF-8")?
                .trim_end_matches('\r');
            if line.is_empty() {
                if !self.data.is_empty() {
                    events.push(self.data.join("\n"));
                    self.data.clear();
                }
            } else if let Some(data) = line.strip_prefix("data:") {
                self.data
                    .push(data.strip_prefix(' ').unwrap_or(data).to_owned());
            }
            consumed = end + 1;
        }
        self.buffer.drain(..consumed);
        if self.data.iter().map(String::len).sum::<usize>() > 1024 * 1024 {
            bail!("SSE event too large");
        }
        Ok(events)
    }
}

#[derive(Default)]
struct StreamingRedactor {
    pending: String,
}
impl StreamingRedactor {
    fn push(&mut self, text: &str, key: &str) -> String {
        self.pending.push_str(text);
        if key.is_empty() {
            return std::mem::take(&mut self.pending);
        }
        self.pending = self.pending.replace(key, "[redacted]");
        let withheld = key
            .char_indices()
            .skip(1)
            .map(|(n, _)| n)
            .filter(|n| self.pending.ends_with(&key[..*n]))
            .max()
            .unwrap_or(0);
        let split = self.pending.len() - withheld;
        let tail = self.pending.split_off(split);
        std::mem::replace(&mut self.pending, tail)
    }
    fn finish(&mut self) -> String {
        std::mem::take(&mut self.pending)
    }
}

pub async fn stream_completion(
    config: &ProviderConfig,
    system: &str,
    messages: &[AgentMessage],
    with_tools: bool,
    cancellation: &Cancellation,
    events: &async_channel::Sender<AgentEvent>,
) -> Result<AgentMessage> {
    stream_with_timeouts(
        config,
        system,
        messages,
        with_tools,
        cancellation,
        events,
        Duration::from_secs(60),
        Duration::from_secs(600),
    )
    .await
}
#[allow(clippy::too_many_arguments)]
async fn stream_with_timeouts(
    config: &ProviderConfig,
    system: &str,
    messages: &[AgentMessage],
    with_tools: bool,
    cancellation: &Cancellation,
    events: &async_channel::Sender<AgentEvent>,
    idle_timeout: Duration,
    request_timeout: Duration,
) -> Result<AgentMessage> {
    let work = async {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        let mut request = client
            .post(config.endpoint()?)
            .header("accept", "text/event-stream")
            .json(&request_body(config, system, messages, with_tools));
        match config.protocol {
            Protocol::OpenAi => {
                if !config.api_key.is_empty() {
                    request = request.bearer_auth(&config.api_key);
                }
            }
            Protocol::Anthropic => {
                let mut key = reqwest::header::HeaderValue::from_str(&config.api_key)
                    .map_err(|_| anyhow::anyhow!("Invalid API key header"))?;
                key.set_sensitive(true);
                request = request
                    .header("x-api-key", key)
                    .header("anthropic-version", "2023-06-01");
            }
        }
        // Show only bounded, structured error messages; never expose request bodies or HTML error pages.
        let mut response = tokio::time::timeout(idle_timeout, request.send())
            .await
            .context("Model response headers timed out")?
            .map_err(|_| anyhow::anyhow!("Could not connect to the model service"))?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let mut body = Vec::new();
            while let Some(chunk) = tokio::time::timeout(idle_timeout, response.chunk())
                .await
                .context("Service error response timed out")?
                .context("Could not read service error")?
            {
                if body.len() + chunk.len() > 8192 {
                    break;
                }
                body.extend_from_slice(&chunk);
            }
            let detail = serde_json::from_slice::<Value>(&body)
                .ok()
                .and_then(|v| {
                    v["error"]["message"]
                        .as_str()
                        .or_else(|| v["message"].as_str())
                        .map(str::to_owned)
                })
                .unwrap_or_default();
            let detail = config
                .redact(&detail)
                .chars()
                .take(1024)
                .collect::<String>();
            bail!("Model service returned HTTP {status}: {detail}");
        }
        if !response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| {
                v.split(';')
                    .next()
                    .is_some_and(|v| v.trim().eq_ignore_ascii_case("text/event-stream"))
            })
        {
            bail!("Model service did not return an SSE stream. Check the protocol and base URL");
        }
        events.send(AgentEvent::ResponseStarted).await?;
        let mut stream = response.bytes_stream();
        let mut decoder = SseDecoder::default();
        let mut completion = Completion::default();
        let mut redactor = StreamingRedactor::default();
        let mut thinking_redactor = StreamingRedactor::default();
        let mut preparing_tools = false;
        while let Some(bytes) = tokio::time::timeout(idle_timeout, stream.next())
            .await
            .context("Model stream timed out")?
        {
            let bytes = bytes.map_err(|_| anyhow::anyhow!("Model stream disconnected"))?;
            for payload in decoder.push(&bytes)? {
                let reasoning_start = completion.reasoning.len();
                let text = completion.event(config.protocol, &payload)?;
                let reasoning = thinking_redactor
                    .push(&completion.reasoning[reasoning_start..], &config.api_key);
                if !reasoning.is_empty() {
                    events.send(AgentEvent::Thinking(reasoning)).await?;
                }
                if !preparing_tools && !completion.calls.is_empty() {
                    preparing_tools = true;
                    events.send(AgentEvent::PreparingTools).await?;
                }
                if let Some(text) = text {
                    let text = redactor.push(&text, &config.api_key);
                    if !text.is_empty() {
                        events.send(AgentEvent::Text(text)).await?;
                    }
                }
            }
            if completion.finished {
                break;
            }
        }
        let result = completion.finish();
        let tail = redactor.finish();
        if !tail.is_empty() {
            events.send(AgentEvent::Text(tail)).await?;
        }
        let tail = thinking_redactor.finish();
        if !tail.is_empty() {
            events.send(AgentEvent::Thinking(tail)).await?;
        }
        result
    };
    tokio::select! {
        _ = cancellation.cancelled() => bail!("Analysis stopped"),
        result = tokio::time::timeout(request_timeout, work) => result.context("Model request timed out")?,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn output_limit_is_provider_managed_where_supported() {
        let mut config = ProviderConfig::default();
        let body = request_body(&config, "", &[], false);
        assert!(body.get("max_completion_tokens").is_none());
        assert!(body.get("max_tokens").is_none());
        config.base_url = "https://compatible.example/v1".into();
        assert!(
            request_body(&config, "", &[], false)
                .get("max_tokens")
                .is_none()
        );
        config.protocol = Protocol::Anthropic;
        assert_eq!(request_body(&config, "", &[], false)["max_tokens"], 4096);
    }
    #[tokio::test]
    async fn stalled_http_streams_timeout_and_cancel_for_both_protocols() {
        use std::io::{Read as _, Write as _};
        for protocol in [Protocol::OpenAi, Protocol::Anthropic] {
            for cancel in [false, true] {
                let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
                let config = ProviderConfig {
                    protocol,
                    model: "test".into(),
                    base_url: format!("http://{}", listener.local_addr().unwrap()),
                    ..Default::default()
                };
                let (ready, received) = async_channel::bounded(1);
                let (release, held) = std::sync::mpsc::channel();
                let server = std::thread::spawn(move || {
                    let (mut socket, _) = listener.accept().unwrap();
                    socket
                        .set_read_timeout(Some(Duration::from_secs(3)))
                        .unwrap();
                    let mut request = [0; 8192];
                    assert!(socket.read(&mut request).unwrap() > 0);
                    socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n").unwrap();
                    ready.send_blocking(()).unwrap();
                    let _ = held.recv_timeout(Duration::from_secs(3));
                });
                let token = Cancellation::default();
                let (events, _receiver) = async_channel::unbounded();
                let request = stream_with_timeouts(
                    &config,
                    "test",
                    &[],
                    false,
                    &token,
                    &events,
                    Duration::from_millis(80),
                    Duration::from_secs(2),
                );
                let control = async {
                    received.recv().await.unwrap();
                    if cancel {
                        token.cancel();
                    }
                };
                let (result, ()) = tokio::join!(request, control);
                let error = result.unwrap_err().to_string();
                assert!(
                    error.contains(if cancel { "stopped" } else { "timed out" }),
                    "{error}"
                );
                release.send(()).unwrap();
                server.join().unwrap();
            }
        }
    }
    #[test]
    fn credentials_split_across_deltas_are_never_emitted() {
        let mut redactor = StreamingRedactor::default();
        let mut result = String::new();
        for text in ["prefix sk-", "sec", "ret suffix"] {
            let delta = redactor.push(text, "sk-secret");
            assert!(!delta.contains("sk-"));
            result.push_str(&delta);
        }
        result.push_str(&redactor.finish());
        assert_eq!(result, "prefix [redacted] suffix");
    }
    #[test]
    fn byte_split_chinese_and_multiline_sse() {
        let mut decoder = SseDecoder::default();
        let bytes = "event: message\r\ndata: 中文\r\ndata: 日志\r\n\r\n".as_bytes();
        let mut output = Vec::new();
        for byte in bytes {
            output.extend(decoder.push(&[*byte]).unwrap());
        }
        assert_eq!(output, vec!["中文\n日志"]);
    }
    #[test]
    fn tool_arguments_are_only_executed_after_complete_response() {
        let mut completion = Completion::default();
        completion.event(Protocol::OpenAi, r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"list_logs","arguments":"{"}}]}}]}"#).unwrap();
        assert!(completion.finish().is_err());
    }
    #[test]
    fn anthropic_accumulates_tool_and_text_blocks() {
        let mut completion = Completion::default();
        for event in [
            json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":"分析"}}),
            json!({"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"a","name":"list_logs","input":{}}}),
            json!({"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{}"}}),
            json!({"type":"message_delta","delta":{"stop_reason":"tool_use"}}),
            json!({"type":"message_stop"}),
        ] {
            completion
                .event(Protocol::Anthropic, &event.to_string())
                .unwrap();
        }
        let AgentMessage::Assistant { text, calls, .. } = completion.finish().unwrap() else {
            panic!()
        };
        assert_eq!(text, "分析");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].arguments, json!({}));
    }
}
