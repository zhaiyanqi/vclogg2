//! User-configured MCP connections, isolated from the log-file capability bridge.
mod transport;
use crate::{Cancellation, ToolCall, ToolResult};
use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use transport::Transport;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpServer {
    id: String,
    name: String,
    #[serde(default)]
    enabled: bool,
    connection: McpConnection,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum McpConnection {
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: BTreeMap<String, String>,
    },
    Http {
        url: String,
        #[serde(default)]
        headers: BTreeMap<String, String>,
    },
}
impl McpServer {
    pub fn new(id: String, name: String, connection: McpConnection) -> Result<Self> {
        let server = Self {
            id,
            name,
            enabled: false,
            connection,
        };
        server.validate()?;
        Ok(server)
    }
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn enabled(&self) -> bool {
        self.enabled
    }
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }
    pub fn connection(&self) -> &McpConnection {
        &self.connection
    }
    pub fn validate(&self) -> Result<()> {
        if self.id.is_empty()
            || self.id.len() > 64
            || self.name.trim().is_empty()
            || self.name.chars().count() > 120
        {
            bail!("MCP requires an ID and a name of 1–120 characters");
        }
        if serde_json::to_vec(&self.connection)?.len() > 32 * 1024 {
            bail!("MCP configuration exceeds 32 KiB");
        }
        match &self.connection {
            McpConnection::Stdio { command, args, env } => {
                if command.trim().is_empty()
                    || command.contains('\0')
                    || args.iter().any(|s| s.contains('\0'))
                    || env
                        .iter()
                        .any(|(k, v)| k.is_empty() || k.contains(['=', '\0']) || v.contains('\0'))
                {
                    bail!("Invalid MCP command, arguments or environment");
                }
            }
            McpConnection::Http { url, headers } => {
                let url = url::Url::parse(url).context("Invalid MCP URL")?;
                if !matches!(url.scheme(), "http" | "https")
                    || url.host_str().is_none()
                    || !url.username().is_empty()
                    || url.password().is_some()
                    || url.fragment().is_some()
                {
                    bail!(
                        "Use an HTTP/HTTPS MCP endpoint without embedded credentials or fragment"
                    );
                }
                for (name, value) in headers {
                    reqwest::header::HeaderName::from_bytes(name.as_bytes())
                        .context("Invalid MCP header name")?;
                    reqwest::header::HeaderValue::from_str(value)
                        .context("Invalid MCP header value")?;
                    if [
                        "host",
                        "content-length",
                        "content-type",
                        "accept",
                        "mcp-session-id",
                        "mcp-protocol-version",
                    ]
                    .iter()
                    .any(|reserved| name.eq_ignore_ascii_case(reserved))
                    {
                        bail!("MCP header is reserved by the transport");
                    }
                }
            }
        }
        Ok(())
    }
    fn redact(&self, text: &str) -> String {
        let mut text = text.to_owned();
        let values = match &self.connection {
            McpConnection::Stdio { env, .. } => env.values(),
            McpConnection::Http { headers, .. } => headers.values(),
        };
        for secret in values.filter(|v| !v.is_empty()) {
            text = text.replace(secret, "[redacted]");
        }
        if let McpConnection::Http { headers, .. } = &self.connection {
            for (name, value) in headers {
                if name.eq_ignore_ascii_case("authorization")
                    && let Some((_, secret)) = value.split_once(' ')
                    && !secret.is_empty()
                {
                    text = text.replace(secret, "[redacted]");
                }
            }
        }
        text
    }
}

struct Session {
    transport: Transport,
    next_id: u64,
    tools: Vec<Value>,
}
impl Session {
    async fn connect(server: &McpServer) -> Result<Self> {
        server.validate()?;
        let mut session = Self {
            transport: Transport::open(server.connection()).await?,
            next_id: 0,
            tools: Vec::new(),
        };
        let initialized = session.request("initialize", json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"VCLogg2","version":env!("CARGO_PKG_VERSION")}})).await?;
        let version = initialized["protocolVersion"]
            .as_str()
            .context("MCP server omitted protocol version")?;
        if !["2025-06-18", "2025-03-26", "2024-11-05"].contains(&version) {
            bail!("Unsupported MCP protocol version");
        }
        session.transport.set_protocol(version);
        session
            .transport
            .exchange(
                json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
                None,
            )
            .await?;
        if initialized["capabilities"].get("tools").is_some() {
            let mut cursor = None;
            for page in 0..8 {
                let params = cursor
                    .as_ref()
                    .map_or(json!({}), |cursor: &String| json!({"cursor":cursor}));
                let result = session.request("tools/list", params).await?;
                for tool in result["tools"]
                    .as_array()
                    .context("Invalid MCP tool list")?
                {
                    let name = tool["name"]
                        .as_str()
                        .context("MCP tool is missing a name")?;
                    if name.is_empty()
                        || name.len() > 256
                        || !tool["inputSchema"].is_object()
                        || session
                            .tools
                            .iter()
                            .any(|previous| previous["name"] == tool["name"])
                    {
                        bail!("Invalid or duplicate MCP tool definition");
                    }
                    session.tools.push(tool.clone());
                    if session.tools.len() > 256 {
                        bail!("MCP server exceeds 256 tools");
                    }
                }
                cursor = result["nextCursor"].as_str().map(str::to_owned);
                if cursor.is_none() {
                    break;
                }
                if page == 7 {
                    bail!("MCP tool listing exceeds 8 pages");
                }
            }
        }
        Ok(session)
    }
    async fn request(&mut self, method: &str, params: Value) -> Result<Value> {
        self.next_id += 1;
        let response = self
            .transport
            .exchange(
                json!({"jsonrpc":"2.0","id":self.next_id,"method":method,"params":params}),
                Some(self.next_id),
            )
            .await?;
        if response.get("error").is_some() {
            bail!("MCP request failed: {}", response["error"]);
        }
        response
            .get("result")
            .cloned()
            .context("MCP response omitted result")
    }
}

/// Each analysis owns its sessions. A timeout invalidates a session; calls are never retried.
#[derive(Default)]
pub(crate) struct McpSessions {
    servers: Vec<McpServer>,
    sessions: BTreeMap<String, Session>,
}
impl McpSessions {
    /// Uses only the schema discovered in this session. Discovery cannot authorize a call.
    pub(crate) fn decision(&self, call: &ToolCall) -> Result<crate::ToolDecision> {
        if call.name != "call_mcp_tool" {
            return Ok(crate::ToolDecision::Automatic);
        }
        let id = call.arguments["server_id"]
            .as_str()
            .context("Missing server_id")?;
        let session = self
            .sessions
            .get(id)
            .context("List MCP tools before calling")?;
        let name = call.arguments["tool_name"]
            .as_str()
            .context("Missing tool_name")?;
        let tool = session
            .tools
            .iter()
            .find(|t| t["name"] == name)
            .context("Unknown MCP tool")?;
        let arguments: Value = serde_json::from_str(
            call.arguments["arguments_json"]
                .as_str()
                .context("Missing arguments_json")?,
        )?;
        validate_arguments(&tool["inputSchema"], &arguments)?;
        Ok(match tool_decision(tool) {
            crate::ToolDecision::Confirm(reason) => {
                let server = self
                    .servers
                    .iter()
                    .find(|server| server.id() == id)
                    .context("MCP server unavailable")?;
                crate::ToolDecision::Confirm(format!(
                    "{reason}\nServer / 服务：{} ({id})\nTool / 工具：{name}",
                    server.name()
                ))
            }
            decision => decision,
        })
    }
    pub(crate) fn new(servers: Vec<McpServer>) -> Self {
        Self {
            servers: servers.into_iter().filter(McpServer::enabled).collect(),
            sessions: BTreeMap::new(),
        }
    }
    pub(crate) async fn execute(
        &mut self,
        call: &ToolCall,
        cancellation: &Cancellation,
    ) -> ToolResult {
        self.execute_with_timeout(call, cancellation, std::time::Duration::from_secs(60))
            .await
    }

    async fn execute_with_timeout(
        &mut self,
        call: &ToolCall,
        cancellation: &Cancellation,
        timeout: std::time::Duration,
    ) -> ToolResult {
        if call.name == "list_mcp_servers" {
            return ToolResult::ok(
                json!({"servers":self.servers.iter().map(|s| json!({"id":s.id(),"name":s.name()})).collect::<Vec<_>>() }),
            );
        }
        let id = call.arguments["server_id"].as_str().unwrap_or_default();
        let Some(server) = self.servers.iter().find(|s| s.id() == id).cloned() else {
            return ToolResult::error("MCP server is not enabled for this run");
        };
        let operation = async {
            if !self.sessions.contains_key(id) {
                self.sessions
                    .insert(id.into(), Session::connect(&server).await?);
            }
            let session = self
                .sessions
                .get_mut(id)
                .context("MCP session unavailable")?;
            if call.name == "list_mcp_tools" {
                let offset = call.arguments["offset"].as_u64().unwrap_or(0) as usize;
                let mut page = Vec::new();
                for tool in session.tools.iter().skip(offset).take(20) {
                    let mut tool = tool.clone();
                    tool["execution_policy"] = json!(match tool_decision(&tool) {
                        crate::ToolDecision::Automatic => "automatic",
                        _ => "confirmation_required",
                    });
                    tool["risk"] = json!({
                        "read_only": tool["annotations"]["readOnlyHint"] == true && tool["annotations"]["destructiveHint"] != true,
                        "destructive": tool["annotations"]["destructiveHint"] == true,
                        "external": true,
                    });
                    page.push(tool);
                    if serde_json::to_vec(&page)?.len() > 48 * 1024 {
                        page.pop();
                        if page.is_empty() {
                            bail!("MCP tool schema exceeds the 48 KiB page limit");
                        }
                        break;
                    }
                }
                let next = offset.saturating_add(page.len());
                return Ok(
                    json!({"tools":page,"next_offset":(next < session.tools.len()).then_some(next)}),
                );
            }
            let name = call.arguments["tool_name"].as_str().unwrap_or_default();
            let tool = session
                .tools
                .iter()
                .find(|tool| tool["name"] == name)
                .context("Unknown MCP tool; list tools before calling")?;
            let arguments: Value =
                serde_json::from_str(call.arguments["arguments_json"].as_str().unwrap_or("{}"))?;
            if !arguments.is_object() {
                bail!("MCP arguments must be a JSON object");
            }
            validate_arguments(&tool["inputSchema"], &arguments)?;
            session
                .request("tools/call", json!({"name":name,"arguments":arguments}))
                .await
        };
        let result = tokio::select! {
            _ = cancellation.cancelled() => Err(anyhow::anyhow!("MCP call stopped; its remote outcome may be unknown")),
            result = tokio::time::timeout(timeout, operation) => result.unwrap_or_else(|_| Err(anyhow::anyhow!("MCP request timed out; its remote outcome may be unknown"))),
        };
        match result {
            Ok(value) => {
                let is_error = value["isError"].as_bool().unwrap_or(false);
                let value = redact_value(value, &server);
                if serde_json::to_vec(&value)
                    .map_or(true, |bytes| bytes.len() > crate::RESULT_BYTES - 128)
                {
                    return ToolResult::error(
                        "MCP result exceeds 64 KiB; the operation may have completed. Request a smaller result, do not blindly repeat mutations.",
                    );
                }
                ToolResult { value, is_error }
            }
            Err(error) => {
                self.sessions.remove(id);
                ToolResult::error(server.redact(&error.to_string()))
            }
        }
    }
}

fn tool_decision(tool: &Value) -> crate::ToolDecision {
    if tool["annotations"]["readOnlyHint"] == true && tool["annotations"]["destructiveHint"] != true
    {
        crate::ToolDecision::Automatic
    } else {
        crate::ToolDecision::Confirm(
            "MCP tool may change external state or has unknown effects".into(),
        )
    }
}

fn validate_arguments(schema: &Value, arguments: &Value) -> Result<()> {
    if !schema.is_object() || !arguments.is_object() {
        bail!("MCP schema and arguments must be objects");
    }
    // Schema resolution must never fetch files or URLs supplied by a server.
    fn local_refs(value: &Value) -> bool {
        match value {
            Value::Object(map) => map.iter().all(|(key, value)| {
                (!matches!(key.as_str(), "$ref" | "$dynamicRef" | "$recursiveRef")
                    || value.as_str().is_some_and(|r| r.starts_with('#')))
                    && local_refs(value)
            }),
            Value::Array(values) => values.iter().all(local_refs),
            _ => true,
        }
    }
    if !local_refs(schema) {
        bail!("External MCP schema references are unsupported");
    }
    let validator = jsonschema::validator_for(schema)
        .map_err(|_| anyhow::anyhow!("Invalid MCP inputSchema"))?;
    validator
        .validate(arguments)
        .map_err(|_| anyhow::anyhow!("MCP arguments do not match inputSchema"))
}

fn redact_value(mut value: Value, server: &McpServer) -> Value {
    match &mut value {
        Value::String(text) => *text = server.redact(text),
        Value::Array(values) => {
            for value in values {
                *value = redact_value(value.take(), server);
            }
        }
        Value::Object(values) => {
            for value in values.values_mut() {
                *value = redact_value(value.take(), server);
            }
        }
        _ => {}
    }
    value
}
/// Connect and discover tools without invoking any server tool.
pub async fn probe_mcp(server: McpServer, cancellation: Cancellation) -> Result<Vec<String>> {
    crate::runner::runtime().spawn(async move {
        let result = tokio::select! {
            _ = cancellation.cancelled() => Err(anyhow::anyhow!("MCP connection test stopped")),
            result = tokio::time::timeout(std::time::Duration::from_secs(30), Session::connect(&server)) => result.unwrap_or_else(|_| Err(anyhow::anyhow!("MCP connection test timed out"))),
        };
        result.map(|session| session.tools.iter().filter_map(|t| t["name"].as_str().map(|name| server.redact(name))).collect()).map_err(|error| anyhow::anyhow!(server.redact(&error.to_string())))
    }).await?
}

#[cfg(test)]
mod capability_tests {
    use super::*;

    #[cfg(unix)]
    #[tokio::test]
    async fn timed_out_calls_invalidate_discovery_without_replay() {
        let connection = McpConnection::Stdio {
            command: "/bin/sh".into(),
            args: vec!["-c".into(), "while IFS= read -r request; do :; done".into()],
            env: BTreeMap::new(),
        };
        let mut server = McpServer::new("test".into(), "Test server".into(), connection).unwrap();
        server.set_enabled(true);
        let transport = Transport::open(server.connection()).await.unwrap();
        let mut sessions = McpSessions::new(vec![server]);
        sessions.sessions.insert("test".into(), Session {
            transport,
            next_id: 0,
            tools: vec![json!({"name":"read","annotations":{"readOnlyHint":true},"inputSchema":{"type":"object","additionalProperties":false}})],
        });
        let call = ToolCall {
            id: "call".into(),
            name: "call_mcp_tool".into(),
            arguments: json!({"server_id":"test","tool_name":"read","arguments_json":"{}"}),
        };
        assert_eq!(
            sessions.decision(&call).unwrap(),
            crate::ToolDecision::Automatic
        );
        let result = sessions
            .execute_with_timeout(
                &call,
                &Cancellation::default(),
                std::time::Duration::from_millis(20),
            )
            .await;
        assert!(result.is_error && result.value.to_string().contains("outcome may be unknown"));
        assert!(sessions.sessions.is_empty());
        assert!(sessions.decision(&call).is_err());
    }

    #[test]
    fn unknown_and_mutating_tools_need_confirmation() {
        for tool in [
            json!({}),
            json!({"annotations":{"readOnlyHint":false}}),
            json!({"annotations":{"readOnlyHint":true,"destructiveHint":true}}),
        ] {
            assert!(matches!(
                tool_decision(&tool),
                crate::ToolDecision::Confirm(_)
            ));
        }
        assert_eq!(
            tool_decision(&json!({"annotations":{"readOnlyHint":true}})),
            crate::ToolDecision::Automatic
        );
    }

    #[test]
    fn schema_validation_rejects_invalid_inputs_and_external_references() {
        let schema = json!({"type":"object","properties":{"path":{"type":"string","minLength":1}},"required":["path"],"additionalProperties":false});
        assert!(validate_arguments(&schema, &json!({"path":"a"})).is_ok());
        for value in [
            json!({}),
            json!({"path":1}),
            json!({"path":""}),
            json!({"path":"a","extra":true}),
        ] {
            assert!(validate_arguments(&schema, &value).is_err());
        }
        assert!(validate_arguments(&json!({"type":"unknown"}), &json!({})).is_err());
        assert!(
            validate_arguments(&json!({"$ref":"file:///tmp/schema.json"}), &json!({})).is_err()
        );
        assert!(
            validate_arguments(
                &json!({"$defs":{"value":{"type":"object"}},"$ref":"#/$defs/value"}),
                &json!({})
            )
            .is_ok()
        );
    }

    #[test]
    fn calls_require_a_discovered_schema() {
        let sessions = McpSessions::default();
        let call = ToolCall {
            id: "call".into(),
            name: "call_mcp_tool".into(),
            arguments: json!({"server_id":"server","tool_name":"write","arguments_json":"{}"}),
        };
        assert!(sessions.decision(&call).is_err());
    }
}
