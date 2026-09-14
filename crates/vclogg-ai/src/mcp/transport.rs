use super::McpConnection;
use anyhow::{Context as _, Result, bail};
use futures_util::StreamExt as _;
use serde_json::{Value, json};
use std::{process::Stdio, time::Duration};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

const MESSAGE_LIMIT: usize = 4 * 1024 * 1024;
pub(super) enum Transport {
    Stdio {
        _child: tokio::process::Child,
        input: tokio::process::ChildStdin,
        output: BufReader<tokio::process::ChildStdout>,
    },
    Http {
        client: reqwest::Client,
        url: String,
        headers: reqwest::header::HeaderMap,
        session: Option<String>,
        protocol: Option<String>,
    },
}
impl Transport {
    pub(super) async fn open(connection: &McpConnection) -> Result<Self> {
        match connection {
            McpConnection::Stdio { command, args, env } => {
                // Spawn the configured executable directly, with no implicit shell.
                // Never inherit a project's working directory or its stdin/stdout.
                let mut command = tokio::process::Command::new(command);
                command
                    .args(args)
                    .envs(env)
                    .current_dir(std::env::temp_dir())
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .kill_on_drop(true);
                #[cfg(windows)]
                command.creation_flags(0x08000000); // CREATE_NO_WINDOW
                let mut child = command.spawn().context(
                    "Could not start MCP server; check the executable path and environment",
                )?;
                let input = child.stdin.take().context("MCP stdin unavailable")?;
                let output = BufReader::new(child.stdout.take().context("MCP stdout unavailable")?);
                Ok(Self::Stdio {
                    _child: child,
                    input,
                    output,
                })
            }
            McpConnection::Http { url, headers } => {
                let mut map = reqwest::header::HeaderMap::new();
                for (name, value) in headers {
                    map.insert(
                        reqwest::header::HeaderName::from_bytes(name.as_bytes())?,
                        reqwest::header::HeaderValue::from_str(value)?,
                    );
                }
                Ok(Self::Http {
                    client: reqwest::Client::builder()
                        .redirect(reqwest::redirect::Policy::none())
                        .connect_timeout(Duration::from_secs(10))
                        .timeout(Duration::from_secs(55))
                        .build()?,
                    url: url.clone(),
                    headers: map,
                    session: None,
                    protocol: None,
                })
            }
        }
    }
    pub(super) fn set_protocol(&mut self, version: &str) {
        if let Self::Http { protocol, .. } = self {
            *protocol = Some(version.into());
        }
    }
    pub(super) async fn exchange(&mut self, message: Value, id: Option<u64>) -> Result<Value> {
        match self {
            Self::Stdio { input, output, .. } => {
                let mut bytes = serde_json::to_vec(&message)?;
                bytes.push(b'\n');
                input.write_all(&bytes).await?;
                input.flush().await?;
                let Some(id) = id else {
                    return Ok(Value::Null);
                };
                for _ in 0..1024 {
                    let line = read_line(output).await?;
                    let response: Value = serde_json::from_slice(&line)
                        .context("MCP stdout must contain newline-delimited JSON-RPC only")?;
                    if response["jsonrpc"] != "2.0" {
                        bail!("Invalid MCP JSON-RPC version");
                    }
                    if response.get("method").is_some() {
                        if let Some(request_id) = response.get("id") {
                            // No sampling, elicitation or roots capabilities were advertised.
                            let reply = if response["method"] == "ping" {
                                json!({"jsonrpc":"2.0","id":request_id,"result":{}})
                            } else {
                                json!({"jsonrpc":"2.0","id":request_id,"error":{"code":-32601,"message":"Client capability not supported"}})
                            };
                            input.write_all(format!("{reply}\n").as_bytes()).await?;
                            input.flush().await?;
                        }
                        continue;
                    }
                    if response["id"].as_u64() == Some(id) {
                        return Ok(response);
                    }
                    bail!("MCP response identity mismatch");
                }
                bail!("Too many MCP notifications without a response");
            }
            Self::Http {
                client,
                url,
                headers,
                session,
                protocol,
            } => {
                let mut request = client
                    .post(url.as_str())
                    .headers(headers.clone())
                    .header("Accept", "application/json, text/event-stream")
                    .json(&message);
                if let Some(session) = session.as_ref() {
                    request = request.header("Mcp-Session-Id", session);
                }
                if let Some(protocol) = protocol.as_ref() {
                    request = request.header("MCP-Protocol-Version", protocol);
                }
                let response = request
                    .send()
                    .await
                    .map_err(|_| anyhow::anyhow!("Could not reach MCP endpoint"))?;
                if !response.status().is_success() {
                    bail!("MCP endpoint returned HTTP {}", response.status().as_u16());
                }
                if let Some(value) = response.headers().get("mcp-session-id") {
                    let value = value.to_str().context("Invalid MCP session ID")?;
                    if value.len() > 1024 {
                        bail!("MCP session ID too long");
                    }
                    *session = Some(value.into());
                }
                let Some(id) = id else {
                    return Ok(Value::Null);
                };
                let sse = response
                    .headers()
                    .get(reqwest::header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    .is_some_and(|v| v.starts_with("text/event-stream"));
                let mut stream = response.bytes_stream();
                let mut buffer = Vec::new();
                let mut total = 0usize;
                while let Some(chunk) = stream.next().await {
                    let chunk = chunk
                        .context("MCP response stream interrupted; execution outcome unknown")?;
                    total = total.saturating_add(chunk.len());
                    if total > MESSAGE_LIMIT {
                        bail!("MCP response exceeds 4 MiB; execution outcome may be complete");
                    }
                    buffer.extend_from_slice(&chunk);
                    if sse {
                        // Parse complete events from bytes so split UTF-8 characters survive.
                        while let Some((end, delimiter)) = event_end(&buffer) {
                            let event = std::str::from_utf8(&buffer[..end])
                                .context("Invalid MCP SSE UTF-8")?;
                            let data = event
                                .lines()
                                .filter_map(|line| {
                                    line.strip_prefix("data:")
                                        .map(|s| s.strip_prefix(' ').unwrap_or(s))
                                })
                                .collect::<Vec<_>>()
                                .join("\n");
                            if !data.is_empty() {
                                let response: Value =
                                    serde_json::from_str(&data).context("Invalid MCP SSE JSON")?;
                                if response["jsonrpc"] != "2.0" {
                                    bail!("Invalid MCP JSON-RPC version");
                                }
                                if response.get("method").is_none()
                                    && response["id"].as_u64() == Some(id)
                                {
                                    return Ok(response);
                                }
                                if response.get("method").is_some() && response.get("id").is_some()
                                {
                                    bail!("MCP server requested an unsupported client capability");
                                }
                            }
                            buffer.drain(..end + delimiter);
                        }
                    }
                }
                if sse {
                    bail!("MCP stream ended before a matching response; execution outcome unknown");
                }
                let response: Value =
                    serde_json::from_slice(&buffer).context("Invalid MCP JSON response")?;
                if response["jsonrpc"] != "2.0" || response["id"].as_u64() != Some(id) {
                    bail!("MCP response identity mismatch");
                }
                Ok(response)
            }
        }
    }
}
fn event_end(buffer: &[u8]) -> Option<(usize, usize)> {
    let lf = buffer.windows(2).position(|w| w == b"\n\n").map(|p| (p, 2));
    let crlf = buffer
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|p| (p, 4));
    match (lf, crlf) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}
async fn read_line(output: &mut BufReader<tokio::process::ChildStdout>) -> Result<Vec<u8>> {
    let mut line = Vec::new();
    loop {
        let bytes = output.fill_buf().await?;
        if bytes.is_empty() {
            bail!("MCP process closed stdout; check the server command");
        }
        let count = bytes
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(bytes.len(), |ix| ix + 1);
        if line.len() + count > MESSAGE_LIMIT {
            bail!("MCP message exceeds 4 MiB");
        }
        let done = bytes[count - 1] == b'\n';
        line.extend_from_slice(&bytes[..count]);
        output.consume(count);
        if done {
            return Ok(line);
        }
    }
}

impl Drop for Transport {
    fn drop(&mut self) {
        // Streamable HTTP sessions may hold server resources beyond a POST.
        // Terminate them best-effort without delaying UI cancellation or shutdown.
        if let Self::Http {
            client,
            url,
            headers,
            session: Some(session),
            protocol,
        } = self
        {
            let mut request = client
                .delete(url.as_str())
                .headers(headers.clone())
                .header("Mcp-Session-Id", session.as_str())
                .timeout(Duration::from_secs(2));
            if let Some(protocol) = protocol {
                request = request.header("MCP-Protocol-Version", protocol.as_str());
            }
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(async move {
                    let _ = request.send().await;
                });
            }
        }
    }
}
