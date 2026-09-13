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
        let Some(path) = self.settings_path.clone() else {
            return;
        };
        let Some(store) = self.store.clone() else {
            self.error = crate::tr!("设置存储不可用", "Settings storage unavailable").into();
            cx.notify();
            return;
        };
        let settings = self.settings.clone();
        let record = if self.revision > 0 {
            self.record().ok()
        } else {
            None
        };
        self.busy = true;
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    store.save_ai_settings(&path, &settings)?;
                    record
                        .as_ref()
                        .map(|record| store.save_ai_conversation(record))
                        .transpose()
                })
                .await;
            _ = this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(Some(revision)) => {
                        this.revision = revision;
                        this.update_history_entry();
                    }
                    Ok(None) => {}
                    Err(error) => this.error = error.to_string(),
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn save_provider(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
        if self.run.is_some() || self.busy {
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
                            crate::tr!("连接测试通过", "Connection test passed").into()
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
    fn import_skill(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.busy = true;
        cx.spawn_in(window, async move |this, cx| {
            let directory = rfd::AsyncFileDialog::new().pick_folder().await;
            let result = if let Some(directory) = directory {
                let path = directory.path().to_path_buf();
                Some(
                    cx.background_spawn(async move { vclogg_ai::import_skill(&path) })
                        .await,
                )
            } else {
                None
            };
            _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                if let Some(result) = result {
                    match result {
                        Ok(skill) => {
                            if !this
                                .settings
                                .skills
                                .iter()
                                .any(|s| s.directory == skill.directory)
                            {
                                this.conversation.skill_ids.push(skill.id.clone());
                                this.settings.skills.push(skill);
                            }
                            this.save_settings(window, cx);
                        }
                        Err(e) => this.error = e.to_string(),
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn refresh_skill(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut skill) = self.settings.skills.iter().find(|s| s.id == id).cloned() else {
            return;
        };
        self.busy = true;
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let enabled = skill.enabled;
                    skill.enabled = true;
                    vclogg_ai::refresh_skill(&mut skill)?;
                    skill.enabled = enabled;
                    Ok::<_, anyhow::Error>(skill)
                })
                .await;
            _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(skill) => {
                        if let Some(old) = this.settings.skills.iter_mut().find(|s| s.id == id) {
                            *old = skill;
                        }
                        this.save_settings(window, cx);
                    }
                    Err(e) => this.error = e.to_string(),
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub(super) fn render_settings(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let disabled = self.busy || self.run.is_some();
        let mut content = v_flex().gap_3().p_3().child(
            h_flex()
                .gap_2()
                .child(
                    Button::new("ai-settings-back")
                        .small()
                        .ghost()
                        .label(crate::tr!("返回", "Back"))
                        .disabled(disabled)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.show_settings = false;
                            this.editor = None;
                            cx.notify();
                        })),
                )
                .child(
                    div()
                        .text_sm()
                        .child(crate::tr!("模型与 Skills", "Models and skills")),
                ),
        );
        if self.run.is_some() {
            content = content.child(
                Button::new("ai-settings-stop")
                    .small()
                    .label(crate::tr!("停止连接测试", "Stop connection test"))
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
                            .label("OpenAI")
                            .selected(!anthropic)
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
                            .label("Anthropic")
                            .selected(anthropic)
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
                        .child(Input::new(input).small()),
                );
            }
            content=content.child(div().text_xs().text_color(cx.theme().muted_foreground).child(crate::tr!("密钥以明文保存在本机应用配置中。日志片段将发送到所选模型服务。","Keys are stored as plaintext in local app configuration. Log excerpts are sent to the selected model service."))).child(Button::new("ai-provider-save").small().primary().label(crate::tr!("保存","Save")).disabled(disabled).on_click(cx.listener(|this,_,window,cx|this.save_provider(window,cx))));
        } else {
            content = content.child(
                Button::new("ai-provider-add")
                    .small()
                    .label(crate::tr!("添加模型…", "Add model…"))
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
                                .label(provider.name.clone())
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
                                .label(crate::tr!("编辑…", "Edit…"))
                                .disabled(disabled)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.edit_provider(edit.clone(), window, cx)
                                })),
                        ),
                );
            }
            content = content
                .child(
                    h_flex()
                        .gap_1()
                        .child(
                            Button::new("ai-test-connection")
                                .small()
                                .label(crate::tr!("测试连接", "Test connection"))
                                .disabled(disabled || self.conversation.provider_id.is_none())
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.test_provider(window, cx)
                                })),
                        )
                        .child(
                            Button::new("ai-remove-provider")
                                .small()
                                .ghost()
                                .label(crate::tr!("移除模型", "Remove model"))
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
                )
                .child(
                    div()
                        .border_t_1()
                        .border_color(cx.theme().border)
                        .pt_3()
                        .text_sm()
                        .child("Skills"),
                )
                .child(
                    Button::new("ai-import-skill")
                        .small()
                        .label(crate::tr!("导入 Skill 目录…", "Import skill folder…"))
                        .disabled(disabled)
                        .on_click(cx.listener(|this, _, window, cx| this.import_skill(window, cx))),
                );
            for skill in self.settings.skills.clone() {
                let id = skill.id.clone();
                let refresh = id.clone();
                let remove = id.clone();
                let view = skill.directory.join("SKILL.md");
                content = content.child(
                    v_flex()
                        .gap_1()
                        .child(
                            Button::new(SharedString::from(format!("ai-skill-enable-{id}")))
                                .small()
                                .ghost()
                                .label(format!(
                                    "{} {}",
                                    if skill.enabled { "✓" } else { "○" },
                                    skill.name
                                ))
                                .disabled(disabled)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    if let Some(skill) =
                                        this.settings.skills.iter_mut().find(|s| s.id == id)
                                    {
                                        skill.enabled = !skill.enabled;
                                    }
                                    this.save_settings(window, cx);
                                })),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(skill.description),
                        )
                        .child(
                            h_flex()
                                .gap_1()
                                .child(
                                    Button::new(SharedString::from(format!(
                                        "ai-skill-view-{refresh}"
                                    )))
                                    .small()
                                    .ghost()
                                    .label(crate::tr!("查看", "View"))
                                    .on_click(move |_, _, cx| cx.open_with_system(&view)),
                                )
                                .child(
                                    Button::new(SharedString::from(format!(
                                        "ai-skill-refresh-{refresh}"
                                    )))
                                    .small()
                                    .ghost()
                                    .label(crate::tr!("刷新", "Refresh"))
                                    .disabled(disabled)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.refresh_skill(refresh.clone(), window, cx)
                                    })),
                                )
                                .child(
                                    Button::new(SharedString::from(format!(
                                        "ai-skill-remove-{remove}"
                                    )))
                                    .small()
                                    .ghost()
                                    .label(crate::tr!("移除", "Remove"))
                                    .disabled(disabled)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.settings.skills.retain(|s| s.id != remove);
                                        this.conversation.skill_ids.retain(|s| s != &remove);
                                        this.save_settings(window, cx);
                                    })),
                                ),
                        ),
                );
            }
        }
        let _ = window;
        div()
            .id("ai-settings-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .child(content)
            .into_any_element()
    }
}
