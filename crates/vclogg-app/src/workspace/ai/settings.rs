use super::*;
use gpui_component::input::{Input, InputState};
use vclogg_ai::{Protocol, ProviderConfig};

pub(super) struct ConfigEditor {
    config: ProviderConfig,
    name: Entity<InputState>,
    url: Entity<InputState>,
    key: Entity<InputState>,
    model: Entity<InputState>,
    limit: Entity<InputState>,
}
impl AiPanel {
    pub(super) fn edit_provider(
        &mut self,
        config: ProviderConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = |value: &str, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| InputState::new(window, cx).default_value(value.to_owned()))
        };
        self.editor = Some(ConfigEditor {
            name: input(&config.name, window, cx),
            url: input(&config.base_url, window, cx),
            key: cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(config.api_key.clone())
                    .masked(true)
            }),
            model: input(&config.model, window, cx),
            limit: input(&config.max_output_tokens.to_string(), window, cx),
            config,
        });
        cx.notify();
    }
    pub(super) fn save_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.save_settings_with_prompt(None, window, cx);
    }
    pub(super) fn save_settings_with_prompt(
        &mut self,
        prompt: Option<(vclogg_ai::Prompt, String)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let saved_prompt = prompt.is_some();
        if cx.global::<super::configuration::SharedAiSettings>().saving {
            self.error = crate::tr!(
                "另一窗口正在保存设置，请稍后重试",
                "Another window is saving settings; try again shortly"
            )
            .into();
            cx.notify();
            return;
        }
        let Some(path) = self.settings_path.clone() else {
            return;
        };
        let Some(store) = self.store.clone() else {
            self.error = crate::tr!("设置存储不可用", "Settings storage unavailable").into();
            cx.notify();
            return;
        };
        let settings = self.settings.clone();
        let published = settings.clone();
        cx.update_global::<super::configuration::SharedAiSettings, _>(|shared, _| {
            shared.saving = true
        });
        let record = if self.revision > 0 {
            self.record().ok()
        } else {
            None
        };
        self.busy = true;
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    if let Some((prompt, text)) = prompt {
                        vclogg_ai::save_prompt(&path, &prompt, &text)?;
                    }
                    store.save_ai_settings(&path, &settings)?;
                    Ok::<_, anyhow::Error>(
                        record
                            .as_ref()
                            .map(|record| store.save_ai_conversation(record))
                            .transpose(),
                    )
                })
                .await;
            // Finish publication even if the originating window has closed.
            gpui::AsyncApp::update_global::<super::configuration::SharedAiSettings, _>(
                cx,
                |shared, _| {
                    shared.saving = false;
                    if result.is_ok() {
                        shared.settings = Some(published);
                    }
                },
            );
            _ = this.update(cx, |this, cx| {
                this.busy = false;
                if result.is_ok() && saved_prompt {
                    this.prompt_editor = None;
                    this.error.clear();
                }
                match result {
                    Ok(Ok(Some(revision))) => {
                        this.revision = revision;
                        this.update_history_entry();
                    }
                    Ok(Ok(None)) => {}
                    Ok(Err(error)) | Err(error) => this.error = error.to_string(),
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub(super) fn save_provider(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings_busy(cx) {
            return;
        }
        let Some(editor) = &self.editor else {
            return;
        };
        let mut config = editor.config.clone();
        config.name = editor.name.read(cx).value().trim().into();
        config.base_url = editor.url.read(cx).value().trim().into();
        config.api_key = editor.key.read(cx).value().trim().into();
        config.model = editor.model.read(cx).value().trim().into();
        config.max_output_tokens = editor.limit.read(cx).value().parse().unwrap_or(0);
        if config.name.is_empty() {
            self.error = crate::tr!("请输入配置名称", "Enter a configuration name").into();
            cx.notify();
            return;
        }
        if let Err(e) = config.endpoint() {
            self.error = e.to_string();
            cx.notify();
            return;
        }
        self.settings.providers.retain(|p| p.id != config.id);
        self.conversation.provider_id = Some(config.id.clone());
        self.settings.active_provider = Some(config.id.clone());
        self.settings.providers.push(config);
        self.editor = None;
        self.error.clear();
        self.save_settings(window, cx);
    }
    fn test_provider(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings_busy(cx) {
            return;
        }
        let Some(config) = self
            .settings
            .providers
            .iter()
            .find(|p| Some(&p.id) == self.conversation.provider_id.as_ref())
            .cloned()
        else {
            return;
        };
        let run = vclogg_ai::start_run(
            config,
            vec![AgentMessage::User {
                text: "Reply OK.".into(),
            }],
            Vec::new(),
            "Connection test; no log content or tools are supplied.".into(),
            true,
        );
        let events = run.events.clone();
        self.run = Some(run);
        self.error.clear();
        cx.spawn_in(window, async move |this, cx| {
            while let Ok(event) = events.recv().await {
                if let vclogg_ai::AgentEvent::Finished(status, error) = event {
                    _ = this.update(cx, |this, cx| {
                        this.run = None;
                        this.error = if status == vclogg_ai::RunStatus::Complete {
                            crate::tr!(
                                "连接测试通过，尚未测试日志工具",
                                "Connection passed; log tools have not been tested"
                            )
                            .into()
                        } else {
                            error
                        };
                        cx.notify();
                    });
                    break;
                }
            }
        })
        .detach();
        cx.notify();
    }
    pub(super) fn render_model_settings(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let disabled = self.settings_busy(cx);
        let mut content = v_flex().gap_3().p_3();
        if self.run.is_some() {
            content = content.child(
                Button::new("ai-settings-stop")
                    .small()
                    .text_label(crate::tr!("停止连接测试", "Stop connection test"))
                    .on_click(cx.listener(|this, _, _, cx| this.stop(cx))),
            );
        }
        if let Some(editor) = &self.editor {
            let anthropic = editor.config.protocol == Protocol::Anthropic;
            content = content.child(
                h_flex()
                    .gap_1()
                    .child(
                        Button::new("ai-protocol-openai")
                            .small()
                            .text_label("OpenAI")
                            .selected(!anthropic)
                            .disabled(disabled)
                            .on_click(cx.listener(|this, _, window, cx| {
                                if let Some(editor) = &mut this.editor {
                                    editor.config.protocol = Protocol::OpenAi;
                                    editor.url.update(cx, |i, cx| {
                                        i.set_value("https://api.openai.com/v1", window, cx)
                                    });
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("ai-protocol-anthropic")
                            .small()
                            .text_label("Anthropic")
                            .selected(anthropic)
                            .disabled(disabled)
                            .on_click(cx.listener(|this, _, window, cx| {
                                if let Some(editor) = &mut this.editor {
                                    editor.config.protocol = Protocol::Anthropic;
                                    editor.url.update(cx, |i, cx| {
                                        i.set_value("https://api.anthropic.com/v1", window, cx)
                                    });
                                }
                                cx.notify();
                            })),
                    ),
            );
            content = content.child(
                Button::new("ai-provider-cancel")
                    .small()
                    .ghost()
                    .text_label(crate::tr!("取消编辑", "Cancel editing"))
                    .disabled(disabled)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.editor = None;
                        cx.notify();
                    })),
            );
            for (label, input) in [
                (crate::tr!("名称", "Name"), &editor.name),
                ("Base URL", &editor.url),
                ("API Key", &editor.key),
                (crate::tr!("模型名", "Model"), &editor.model),
                (
                    crate::tr!("最大输出 tokens", "Output token limit"),
                    &editor.limit,
                ),
            ] {
                content = content.child(
                    v_flex()
                        .gap_1()
                        .child(div().text_xs().child(label))
                        .child(Input::new(input).small().disabled(disabled)),
                );
            }
            content=content.child(div().text_xs().text_color(cx.theme().muted_foreground).child(crate::tr!("密钥以明文保存在本机应用配置中。日志片段将发送到所选模型服务。","Keys are stored as plaintext in local app configuration. Log excerpts are sent to the selected model service."))).child(Button::new("ai-provider-save").small().primary().text_label(crate::tr!("保存","Save")).disabled(disabled).on_click(cx.listener(|this,_,window,cx|this.save_provider(window,cx))));
        } else {
            content = content.child(
                Button::new("ai-provider-add")
                    .small()
                    .text_label(crate::tr!("添加模型…", "Add model…"))
                    .disabled(disabled)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.edit_provider(ProviderConfig::default(), window, cx)
                    })),
            );
            for provider in self.settings.providers.clone() {
                let edit = provider.clone();
                let id = provider.id.clone();
                let selected = self.conversation.provider_id.as_ref() == Some(&provider.id);
                content = content.child(
                    h_flex()
                        .gap_1()
                        .child(
                            Button::new(SharedString::from(format!("ai-provider-{}", provider.id)))
                                .small()
                                .ghost()
                                .flex_1()
                                .min_w_0()
                                .text_label(provider.name.clone())
                                .selected(selected)
                                .disabled(disabled)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.conversation.provider_id = Some(id.clone());
                                    this.settings.active_provider = Some(id.clone());
                                    this.save_settings(window, cx);
                                })),
                        )
                        .child(
                            Button::new(SharedString::from(format!("ai-edit-{}", provider.id)))
                                .small()
                                .ghost()
                                .text_label(crate::tr!("编辑…", "Edit…"))
                                .disabled(disabled)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.edit_provider(edit.clone(), window, cx)
                                })),
                        ),
                );
            }
            content = content.child(
                h_flex()
                    .gap_1()
                    .child(
                        Button::new("ai-test-connection")
                            .small()
                            .text_label(crate::tr!("测试连接", "Test connection"))
                            .disabled(disabled || self.conversation.provider_id.is_none())
                            .on_click(
                                cx.listener(|this, _, window, cx| this.test_provider(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("ai-remove-provider")
                            .small()
                            .ghost()
                            .text_label(crate::tr!("移除模型", "Remove model"))
                            .disabled(disabled || self.conversation.provider_id.is_none())
                            .on_click(cx.listener(|this, _, window, cx| {
                                let id = this.conversation.provider_id.take();
                                this.settings
                                    .providers
                                    .retain(|p| Some(&p.id) != id.as_ref());
                                this.settings.active_provider =
                                    this.settings.providers.first().map(|p| p.id.clone());
                                this.conversation.provider_id =
                                    this.settings.active_provider.clone();
                                this.save_settings(window, cx);
                            })),
                    ),
            );
        }
        div()
            .id("ai-model-settings-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .child(content)
            .into_any_element()
    }
}
