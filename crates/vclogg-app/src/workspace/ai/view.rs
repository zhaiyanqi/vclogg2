use super::*;
use gpui_component::{input::Textarea, text::TextView};
use vclogg_ai::RunStatus;

impl AiPanel {
    fn render_message(&mut self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        if ix >= self.conversation.messages.len() {
            return v_flex().px_3().py_2().gap_2()
                .when(self.run.is_some(),|this|this.child(div().text_xs().text_color(cx.theme().muted_foreground).child(self.pending_tool.as_ref().map(|c|format!("{}: {}",crate::tr!("执行工具","Running tool"),c.name)).unwrap_or_else(||crate::tr!("正在分析…","Analyzing…").into()))))
                .when(!self.live.is_empty(),|this|this.child(TextView::new(&self.live_view).on_link_click(|url, _, _, cx| { if url.starts_with("https://") || url.starts_with("http://") { cx.open_url(url); } })))
                .when(self.conversation.messages.is_empty(),|this|this.child(div().text_sm().text_color(cx.theme().muted_foreground).child(crate::tr!("让 AI 查找异常、关联日志、添加标记或高亮关键词。先配置模型，再输入分析问题。","Ask AI to find errors, correlate logs, add marks or highlight keywords. Configure a model, then describe your investigation."))))
                .into_any_element();
        }
        let message = self.conversation.messages[ix].clone();
        let title = match &message {
            AgentMessage::User { .. } => crate::tr!("你", "You").to_owned(),
            AgentMessage::Assistant { .. } => "AI".into(),
            AgentMessage::Tool { name, result, .. } => format!(
                "{} · {}",
                name,
                if result.is_error {
                    crate::tr!("失败", "Failed")
                } else {
                    crate::tr!("完成", "Done")
                }
            ),
        };
        let is_tool = matches!(message, AgentMessage::Tool { .. });
        let expanded = self.expanded.contains(&ix);
        let copy = match &message {
            AgentMessage::User { text } | AgentMessage::Assistant { text, .. } => text.clone(),
            AgentMessage::Tool { result, .. } => {
                serde_json::to_string_pretty(&result.value).unwrap_or_default()
            }
        };
        let mut row = v_flex()
            .id(SharedString::from(format!(
                "ai-message-{}-{ix}",
                self.conversation.id
            )))
            .px_3()
            .py_2()
            .gap_1()
            .min_w_0()
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(title),
                    )
                    .child(
                        Button::new(("ai-copy", ix))
                            .xsmall()
                            .ghost()
                            .label(crate::tr!("复制", "Copy"))
                            .on_click(move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()))
                            }),
                    ),
            );
        if is_tool {
            row = row.child(
                Button::new(("ai-tool-expand", ix))
                    .small()
                    .ghost()
                    .label(if expanded {
                        crate::tr!("收起详情", "Hide details")
                    } else {
                        crate::tr!("查看详情", "Show details")
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.expanded.insert(ix) {
                            this.expanded.remove(&ix);
                        }
                        this.list.remeasure_items(ix..ix + 1);
                        cx.notify();
                    })),
            );
        }
        if !is_tool || expanded {
            row = row.child(
                TextView::new(&self.messages[ix]).on_link_click(|url, _, _, cx| {
                    if url.starts_with("https://") || url.starts_with("http://") {
                        cx.open_url(url);
                    }
                }),
            );
        }
        if let AgentMessage::Assistant { calls, .. } = &message {
            for call in calls {
                row = row.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("↳ {}", call.name)),
                );
            }
        }
        if let AgentMessage::Tool { result, .. } = &message {
            let mut refs = Vec::new();
            collect_references(&result.value, &mut refs);
            for (n, reference) in refs
                .into_iter()
                .take(if expanded { 100 } else { 3 })
                .enumerate()
            {
                let name = self
                    .scope
                    .as_ref()
                    .and_then(|scope| scope.lock().ok())
                    .and_then(|scope| {
                        scope
                            .documents
                            .get(&reference.document_id)
                            .map(|d| d.document.file_name().to_owned())
                    })
                    .unwrap_or_else(|| crate::tr!("日志", "Log").into());
                let label = format!("{name} · {} {}", crate::tr!("行", "line"), reference.line);
                row = row.child(
                    Button::new(SharedString::from(format!("ai-jump-{ix}-{n}")))
                        .small()
                        .ghost()
                        .label(label)
                        .disabled(self.busy || self.run.is_some())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.jump_reference(reference.clone(), window, cx)
                        })),
                );
            }
        }
        row.into_any_element()
    }
    fn jump_reference(
        &mut self,
        reference: LogReference,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(scope) = self.scope.clone() else {
            self.error = crate::tr!(
                "历史引用需要重新分析以确认文件内容",
                "Analyze again to refresh historical log references"
            )
            .into();
            cx.notify();
            return;
        };
        let call = ToolCall {
            id: uuid::Uuid::new_v4().to_string(),
            name: "navigate".into(),
            arguments: json!({"action":"line","reference":reference}),
        };
        // A completed run's cancellation token is not reused for a user navigation action.
        if let Ok(mut state) = scope.lock() {
            state.cancellation = SearchCancellation::default();
        }
        let work = self.workspace.update(cx, |workspace, cx| {
            workspace.ai_prepare(scope.clone(), &call, cx)
        });
        let work = match work {
            Ok(Ok(work)) => work,
            Ok(Err(e)) => {
                self.error = e.to_string();
                cx.notify();
                return;
            }
            Err(_) => return,
        };
        let workspace = self.workspace.clone();
        self.busy = true;
        cx.spawn_in(window, async move |this, cx| {
            let evidence = cx.background_spawn(async move { work() }).await;
            let result = match evidence {
                Ok(value) => workspace
                    .update_in(cx, |w, window, cx| {
                        w.ai_commit(&scope, &call, value, window, cx)
                    })
                    .and_then(|r| r),
                Err(e) => Err(e),
            };
            _ = this.update(cx, |this, cx| {
                this.busy = false;
                if let Err(e) = result {
                    this.error = e.to_string();
                }
                cx.notify();
            });
        })
        .detach();
    }
}
fn collect_references(value: &Value, refs: &mut Vec<LogReference>) {
    if refs.len() >= 100 {
        return;
    }
    if let Some(reference) = value
        .get("reference")
        .and_then(|r| serde_json::from_value::<LogReference>(r.clone()).ok())
        && !refs.contains(&reference)
    {
        refs.push(reference);
    }
    match value {
        Value::Object(values) => {
            for v in values.values() {
                collect_references(v, refs);
            }
        }
        Value::Array(values) => {
            for v in values {
                collect_references(v, refs);
            }
        }
        _ => {}
    }
}
impl Render for AiPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let disabled = self.busy || self.run.is_some();
        let mut body = v_flex()
            .size_full()
            .min_h_0()
            .min_w_0()
            .text_color(cx.theme().foreground);
        if !self.error.is_empty() {
            body = body.child(div().px_3().py_2().text_xs().child(self.error.clone()));
        }
        if self.show_settings {
            return body
                .child(self.render_settings(window, cx))
                .into_any_element();
        }
        let state = cx.entity();
        let models = cx.entity();
        let skills = cx.entity();
        let header = h_flex()
            .items_start()
            .flex_wrap()
            .gap_1()
            .p_2()
            .child(
                Button::new("ai-conversation-menu")
                    .small()
                    .ghost()
                    .label(if self.conversation.title.is_empty() {
                        crate::tr!("会话", "Conversations").to_owned()
                    } else {
                        self.conversation.title.chars().take(16).collect()
                    })
                    .disabled(disabled)
                    .dropdown_menu(move |mut menu, window, cx| {
                        for row in state.read(cx).history.clone() {
                            let id = row.id;
                            menu = menu.item(PopupMenuItem::new(row.title).on_click(
                                window.listener_for(&state, move |this, _, window, cx| {
                                    this.load_conversation(id.clone(), window, cx)
                                }),
                            ));
                        }
                        if state.read(cx).more_history {
                            menu = menu.item(
                                PopupMenuItem::new(crate::tr!(
                                    "加载更多会话",
                                    "Load more conversations"
                                ))
                                .on_click(
                                    window.listener_for(&state, |this, _, window, cx| {
                                        this.more_conversations(window, cx)
                                    }),
                                ),
                            );
                        }
                        menu = menu.separator().item(
                            PopupMenuItem::new(crate::tr!(
                                "删除当前会话",
                                "Delete current conversation"
                            ))
                            .on_click(
                                window.listener_for(&state, |this, _, window, cx| {
                                    this.delete_conversation(window, cx)
                                }),
                            ),
                        );
                        menu
                    }),
            )
            .child(
                Button::new("ai-new-conversation")
                    .small()
                    .ghost()
                    .icon(IconName::Plus)
                    .tooltip(crate::tr!("新建会话", "New conversation"))
                    .disabled(disabled)
                    .on_click(cx.listener(|this, _, _, cx| this.new_conversation(cx))),
            )
            .child(
                Button::new("ai-settings")
                    .small()
                    .ghost()
                    .icon(IconName::Settings)
                    .tooltip(crate::tr!("AI 设置…", "AI settings…"))
                    .disabled(disabled)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_settings = true;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("ai-model-menu")
                    .small()
                    .ghost()
                    .label(
                        self.settings
                            .providers
                            .iter()
                            .find(|p| Some(&p.id) == self.conversation.provider_id.as_ref())
                            .map(|p| p.name.clone())
                            .unwrap_or_else(|| crate::tr!("选择模型", "Select model").into()),
                    )
                    .disabled(disabled)
                    .dropdown_menu(move |mut menu, window, cx| {
                        for provider in models.read(cx).settings.providers.clone() {
                            let id = provider.id;
                            menu = menu.item(PopupMenuItem::new(provider.name).on_click(
                                window.listener_for(&models, move |this, _, window, cx| {
                                    this.conversation.provider_id = Some(id.clone());
                                    this.settings.active_provider = Some(id.clone());
                                    this.save_settings(window, cx);
                                }),
                            ));
                        }
                        menu
                    }),
            );
        let owner = cx.entity();
        let messages = gpui::list(self.list.clone(), move |ix, _, cx| {
            owner.update(cx, |this, cx| this.render_message(ix, cx))
        })
        .size_full();
        let footer = v_flex()
            .flex_shrink_0()
            .gap_2()
            .p_2()
            .border_t_1()
            .border_color(cx.theme().border)
            .when(!self.conversation.notice.is_empty(), |this| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(self.conversation.notice.clone()),
                )
            })
            .child(Textarea::new(&self.input).disabled(self.busy))
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        Button::new("ai-skills-menu")
                            .small()
                            .ghost()
                            .label(format!("Skills ({})", self.conversation.skill_ids.len()))
                            .disabled(disabled)
                            .dropdown_menu(move |mut menu, window, cx| {
                                let chosen = skills.read(cx).conversation.skill_ids.clone();
                                for skill in skills
                                    .read(cx)
                                    .settings
                                    .skills
                                    .iter()
                                    .filter(|s| s.enabled)
                                    .cloned()
                                    .collect::<Vec<_>>()
                                {
                                    let id = skill.id;
                                    menu =
                                        menu.item(
                                            PopupMenuItem::new(format!(
                                                "{} {}",
                                                if chosen.contains(&id) { "✓" } else { "○" },
                                                skill.name
                                            ))
                                            .on_click(window.listener_for(
                                                &skills,
                                                move |this, _, window, cx| {
                                                    if this.conversation.skill_ids.contains(&id) {
                                                        this.conversation
                                                            .skill_ids
                                                            .retain(|s| s != &id);
                                                    } else {
                                                        this.conversation
                                                            .skill_ids
                                                            .push(id.clone());
                                                    }
                                                    this.save_settings(window, cx);
                                                    cx.notify();
                                                },
                                            )),
                                        );
                                }
                                menu
                            }),
                    )
                    .child(div().flex_1())
                    .when(
                        self.conversation.status == RunStatus::LimitReached,
                        |this| {
                            this.child(
                                Button::new("ai-continue")
                                    .small()
                                    .label(crate::tr!("继续", "Continue"))
                                    .disabled(disabled)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.send(true, window, cx)
                                    })),
                            )
                        },
                    )
                    .child(if self.run.is_some() {
                        Button::new("ai-stop")
                            .small()
                            .label(crate::tr!("停止", "Stop"))
                            .on_click(cx.listener(|this, _, _, cx| this.stop(cx)))
                    } else {
                        Button::new("ai-send")
                            .small()
                            .primary()
                            .label(crate::tr!("发送", "Send"))
                            .disabled(self.busy)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.send(false, window, cx)),
                            )
                    }),
            );
        body.child(header)
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .child(messages)
                    .vertical_scrollbar(&self.list),
            )
            .child(footer)
            .into_any_element()
    }
}
