use super::*;
use gpui_component::{
    input::{Input, InputState, Textarea, TextareaState},
    switch::Switch,
};
use vclogg_ai::{McpConnection, McpServer};

pub(super) struct McpEditor {
    id: String,
    enabled: bool,
    original: Option<McpServer>,
    name: Entity<InputState>,
    connection: Entity<TextareaState>,
}
impl AiPanel {
    fn edit_mcp(&mut self, server: Option<McpServer>, window: &mut Window, cx: &mut Context<Self>) {
        let connection = server
            .as_ref()
            .and_then(|s| serde_json::to_string_pretty(s.connection()).ok())
            .unwrap_or_else(|| "{\n  \"command\": \"\",\n  \"args\": [],\n  \"env\": {}\n}".into());
        let name = server.as_ref().map_or(String::new(), |s| s.name().into());
        self.mcp_editor = Some(McpEditor {
            id: server
                .as_ref()
                .map_or_else(|| uuid::Uuid::new_v4().to_string(), |s| s.id().into()),
            enabled: server.as_ref().is_some_and(McpServer::enabled),
            original: server.clone(),
            name: cx.new(|cx| InputState::new(window, cx).default_value(name)),
            connection: cx.new(|cx| {
                TextareaState::new(window, cx)
                    .auto_grow(10, 18)
                    .default_value(connection)
            }),
        });
        self.mcp_status.clear();
        self.error.clear();
        cx.notify();
    }
    fn mcp_editor_value(&self, cx: &App) -> Result<McpServer> {
        let editor = self.mcp_editor.as_ref().context("No MCP server selected")?;
        let text = editor.connection.read(cx).value();
        if text.len() > 32 * 1024 {
            bail!("MCP configuration exceeds 32 KiB");
        }
        // Exactly one transport; reject typos instead of silently dropping credentials.
        let value: Value = serde_json::from_str(&text).context("Invalid MCP connection JSON")?;
        let object = value
            .as_object()
            .context("MCP connection must be a JSON object")?;
        let keys = if object.contains_key("command") {
            &["command", "args", "env"][..]
        } else {
            &["url", "headers"][..]
        };
        if object.keys().any(|key| !keys.contains(&key.as_str())) {
            bail!("Use command/args/env for stdio, or url/headers for HTTP");
        }
        let connection: McpConnection = serde_json::from_value(value)
            .context("Set a command with args/env, or a URL with headers")?;
        let mut server = McpServer::new(
            editor.id.clone(),
            editor.name.read(cx).value().trim().into(),
            connection,
        )?;
        server.set_enabled(editor.enabled);
        Ok(server)
    }
    pub(super) fn save_mcp_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings_busy(cx) || self.mcp_testing {
            return;
        }
        let server = match self.mcp_editor_value(cx) {
            Ok(server) => server,
            Err(error) => {
                self.error = error.to_string();
                cx.notify();
                return;
            }
        };
        let current = self
            .settings
            .mcp_servers
            .iter()
            .find(|s| s.id() == server.id());
        let original = self
            .mcp_editor
            .as_ref()
            .and_then(|editor| editor.original.as_ref());
        if current != original {
            self.error = crate::tr!(
                "此 MCP 服务已在另一窗口修改，请取消后重新编辑",
                "This MCP server changed in another window; cancel and reopen the editor"
            )
            .into();
            cx.notify();
            return;
        }
        if let Some(existing) = self
            .settings
            .mcp_servers
            .iter_mut()
            .find(|s| s.id() == server.id())
        {
            *existing = server;
        } else {
            if self.settings.mcp_servers.len() >= 32 {
                self.error = crate::tr!(
                    "最多添加 32 个 MCP 服务",
                    "Up to 32 MCP servers are supported"
                )
                .into();
                cx.notify();
                return;
            }
            self.settings.mcp_servers.push(server);
        }
        self.save_settings(window, cx);
    }
    fn test_mcp(&mut self, server: McpServer, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings_busy(cx) || self.mcp_testing {
            return;
        }
        self.mcp_testing = true;
        self.mcp_status = crate::tr!("正在连接并读取工具…", "Connecting and listing tools…").into();
        self.error.clear();
        let cancellation = vclogg_ai::Cancellation::default();
        self.mcp_test_cancel = Some(cancellation.clone());
        let generation = self.settings_generation;
        self.mcp_test_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = vclogg_ai::probe_mcp(server, cancellation).await;
            _ = this.update(cx, |this, cx| {
                this.mcp_testing = false;
                this.mcp_test_cancel = None;
                if this.settings_generation == generation {
                    match result {
                        Ok(names) => {
                            this.mcp_status = format!(
                                "{} ({})\n{}",
                                crate::tr!("连接成功，可用工具", "Connected, available tools"),
                                names.len(),
                                names.join(", ")
                            )
                        }
                        Err(error) => {
                            this.mcp_status.clear();
                            this.error = error.to_string();
                        }
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
    pub(super) fn render_mcp_settings(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let disabled = self.settings_busy(cx) || self.mcp_testing;
        let mut content = v_flex().p_3().gap_3().min_w_0()
            .child(div().text_sm().text_color(cx.theme().muted_foreground).child(crate::tr!(
                "连接本地 stdio 或 Streamable HTTP 服务。新增服务默认关闭；启用后，AI 可按你的请求调用其工具。连接测试会启动本地服务或访问配置的地址，仅发现工具。",
                "Connect a local stdio or Streamable HTTP server. New servers start disabled; enabling one lets AI call its tools for your requests. Testing starts the local service or contacts its endpoint to discover tools."
            )));
        if let Some(editor) = &self.mcp_editor {
            content = content
                .child(div().text_sm().child(crate::tr!("服务名称", "Server name")))
                .child(Input::new(&editor.name).disabled(disabled))
                .child(div().text_sm().child(crate::tr!("连接配置（JSON）", "Connection configuration (JSON)")))
                .child(Textarea::new(&editor.connection).disabled(disabled))
                .child(div().text_sm().text_color(cx.theme().muted_foreground).child(crate::tr!(
                    "stdio 使用 command、args、env；建议 command 填可执行文件绝对路径。HTTP 使用 url、headers，例如 {\"url\":\"http://localhost:3000/mcp\",\"headers\":{}}。配置保存在本机 AI 设置中。",
                    "For stdio use command, args and env; prefer an absolute executable path. For HTTP use url and headers, e.g. {\"url\":\"http://localhost:3000/mcp\",\"headers\":{}}. Configuration is stored in local AI settings."
                )))
                .child(h_flex().gap_2().flex_wrap()
                    .child(Button::new("ai-mcp-save").small().primary().text_label(crate::tr!("保存", "Save")).disabled(disabled).on_click(cx.listener(|this, _, window, cx| this.save_mcp_editor(window, cx))))
                    .child(Button::new("ai-mcp-test-draft").small().text_label(crate::tr!("测试连接", "Test connection")).disabled(disabled).on_click(cx.listener(|this, _, window, cx| match this.mcp_editor_value(cx) {
                        Ok(server) => this.test_mcp(server, window, cx),
                        Err(error) => { this.error = error.to_string(); cx.notify(); }
                    })))
                    .child(Button::new("ai-mcp-cancel").small().ghost().text_label(crate::tr!("取消", "Cancel")).disabled(disabled).on_click(cx.listener(|this, _, _, cx| { this.mcp_editor = None; this.mcp_status.clear(); cx.notify(); }))));
        } else {
            content = content.child(
                Button::new("ai-mcp-add")
                    .small()
                    .text_label(crate::tr!("新增服务…", "New server…"))
                    .disabled(disabled)
                    .on_click(cx.listener(|this, _, window, cx| this.edit_mcp(None, window, cx))),
            );
            if self.settings.mcp_servers.is_empty() {
                content = content.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(crate::tr!("尚未配置 MCP 服务", "No MCP servers configured")),
                );
            }
            for server in self.settings.mcp_servers.clone() {
                let id = server.id().to_owned();
                let remove = id.clone();
                let edit = server.clone();
                let test = server.clone();
                content = content.child(
                    h_flex()
                        .gap_2()
                        .flex_wrap()
                        .min_w_0()
                        .child(
                            Switch::new(SharedString::from(format!("ai-mcp-enable-{id}")))
                                .label(server.name().to_owned())
                                .checked(server.enabled())
                                .disabled(disabled)
                                .on_click(cx.listener(move |this, enabled, window, cx| {
                                    if let Some(server) =
                                        this.settings.mcp_servers.iter_mut().find(|s| s.id() == id)
                                    {
                                        server.set_enabled(*enabled);
                                    }
                                    this.save_settings(window, cx);
                                })),
                        )
                        .child(div().flex_1())
                        .child(
                            Button::new(SharedString::from(format!("ai-mcp-edit-{}", server.id())))
                                .small()
                                .ghost()
                                .text_label(crate::tr!("编辑…", "Edit…"))
                                .disabled(disabled)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.edit_mcp(Some(edit.clone()), window, cx)
                                })),
                        )
                        .child(
                            Button::new(SharedString::from(format!("ai-mcp-test-{}", server.id())))
                                .small()
                                .ghost()
                                .text_label(crate::tr!("测试连接", "Test connection"))
                                .disabled(disabled)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.test_mcp(test.clone(), window, cx)
                                })),
                        )
                        .child(
                            Button::new(SharedString::from(format!(
                                "ai-mcp-remove-{}",
                                server.id()
                            )))
                            .small()
                            .ghost()
                            .text_label(crate::tr!("移除", "Remove"))
                            .disabled(disabled)
                            .on_click(cx.listener(
                                move |this, _, window, cx| {
                                    this.settings.mcp_servers.retain(|s| s.id() != remove);
                                    this.save_settings(window, cx);
                                },
                            )),
                        ),
                );
            }
        }
        if !self.mcp_status.is_empty() {
            content = content.child(div().text_sm().child(self.mcp_status.clone()));
        }
        if self.mcp_testing {
            content = content.child(
                Button::new("ai-mcp-stop-test")
                    .small()
                    .ghost()
                    .text_label(crate::tr!("停止测试", "Stop test"))
                    .on_click(cx.listener(|this, _, _, _| {
                        if let Some(token) = &this.mcp_test_cancel {
                            token.cancel();
                        }
                    })),
            );
        }
        div()
            .id("ai-mcp-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .child(content)
            .into_any_element()
    }
}
