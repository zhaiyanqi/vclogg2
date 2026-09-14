use super::*;
use gpui_component::{input::Textarea, text::TextView};
use gpui_message_scroller::MessageScroller;
use vclogg_ai::RunStatus;

impl AiPanel {
    fn render_message(&mut self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        // Append-only transcript ordinals are stable within a conversation.
        // The live response uses the same ID and TextView as its final message.
        let row_id = SharedString::from(format!("ai-message-{}-{ix}", self.conversation.id));
        if ix >= self.conversation.messages.len() {
            return v_flex()
                .id(row_id)
                .w_full()
                .min_w_0()
                .gap_2()
                .when(!self.reasoning.is_empty(), |row| {
                    row.child(self.render_thinking(ix, true, cx))
                })
                .when(!self.live.is_empty(), |row| {
                    row.child(self.markdown_view(&self.live_view, cx))
                })
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
        let sent = matches!(message, AgentMessage::User { .. });
        let expanded = self.expanded.contains(&ix);
        let copy = match &message {
            AgentMessage::User { text } | AgentMessage::Assistant { text, .. } => text.clone(),
            AgentMessage::Tool { result, .. } => {
                serde_json::to_string_pretty(&result.value).unwrap_or_default()
            }
        };
        let mut row = v_flex()
            .id(row_id)
            .debug_selector(move || {
                if sent {
                    "ai-user-bubble".into()
                } else {
                    "ai-reply".into()
                }
            })
            .w_full()
            .min_w_0()
            .gap_1()
            .when(sent, |row| {
                row.p_3().rounded(cx.theme().radius_lg).bg(cx.theme().muted)
            })
            .when(is_tool, |row| {
                row.p_2()
                    .border_1()
                    .border_color(cx.theme().border)
                    .rounded(cx.theme().radius)
            })
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
                        this.scroller
                            .update(cx, |state, cx| state.remeasure_items(ix..ix + 1, cx));
                        cx.notify();
                    })),
            );
        }
        if self.reasoning_views.get(ix).is_some_and(Option::is_some) {
            row = row.child(self.render_thinking(ix, false, cx));
        }
        if !is_tool || expanded {
            row = row.child(self.markdown_view(&self.messages[ix], cx));
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
                        .disabled(self.busy || self.ui_busy)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.jump_reference(reference.clone(), window, cx)
                        })),
                );
            }
        }
        let selected_view = self.messages[ix].clone();
        let panel = cx.entity();
        let row = row.context_menu(move |mut menu, window, cx| {
            let text = selected_view.read(cx).selected_text();
            menu = menu.item(PopupMenuItem::new(crate::tr!("复制选中文字", "Copy selected text"))
                .disabled(text.is_empty()).on_click({ let text = text.clone(); move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(text.clone())) }));
            let documents = panel.read(cx).scope.as_ref().and_then(|scope| scope.lock().ok())
                .map(|scope| scope.documents.values().filter(|d| d.open).cloned().collect::<Vec<_>>()).unwrap_or_default();
            for doc in documents {
                let text = text.clone();
                let label = format!("{} · {}", crate::tr!("追加到搜索框", "Append to search box"), doc.document.file_name());
                menu = menu.item(PopupMenuItem::new(label)
                    .disabled(text.trim().is_empty() || panel.read(cx).busy || panel.read(cx).ui_busy)
                    .on_click(window.listener_for(&panel, move |this, _, window, cx| {
                        this.run_ui_tool(ToolCall { id: uuid::Uuid::new_v4().to_string(), name: "append_search".into(), arguments: json!({"document_id":doc.id,"version":doc.version,"text":text}) }, window, cx);
                    })));
            }
            menu
        });
        if sent {
            h_flex()
                .w_full()
                .min_w_0()
                .justify_end()
                .child(div().max_w(gpui::relative(0.9)).min_w_0().child(row))
                .into_any_element()
        } else {
            row.into_any_element()
        }
    }
    fn render_attachments(&self, cx: &Context<Self>) -> AnyElement {
        let mut content = v_flex()
            .id("ai-attachments")
            .gap_1()
            .max_h(gpui::rems(8.))
            .overflow_y_scroll();
        for (ix, log) in self.draft_logs.iter().enumerate() {
            let label = format!(
                "{}:{}",
                log.document.document.file_name(),
                log.source_row + 1
            );
            content = content.child(
                h_flex()
                    .min_w_0()
                    .gap_1()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_xs()
                            .text_ellipsis()
                            .child(label),
                    )
                    .child(
                        Button::new(("ai-remove-attachment", ix))
                            .xsmall()
                            .ghost()
                            .icon(IconName::Close)
                            .tooltip(crate::tr!("移除附加日志", "Remove attached log"))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if ix < this.draft_logs.len() {
                                    this.draft_logs.remove(ix);
                                }
                                cx.notify();
                            })),
                    ),
            );
        }
        content.into_any_element()
    }

    fn markdown_view(
        &self,
        view: &Entity<gpui_component::text::TextViewState>,
        cx: &Context<Self>,
    ) -> TextView {
        let owner = cx.weak_entity();
        TextView::new(view).on_link_click(move |url, event, window, cx| {
            if !matches!(event, gpui::ClickEvent::Mouse(event) if event.up.button != MouseButton::Left) {
                _ = owner.update(cx, |this, cx| this.open_link(url, window, cx));
            }
        })
    }

    fn render_thinking(&self, ix: usize, live: bool, cx: &Context<Self>) -> AnyElement {
        let expanded = if live {
            !self.live_thinking_collapsed
        } else {
            self.thinking_expanded.contains(&ix)
        };
        let view = if live {
            Some(&self.live_reasoning_view)
        } else {
            self.reasoning_views.get(ix).and_then(Option::as_ref)
        };
        v_flex()
            .debug_selector(|| "ai-thinking-region".into())
            .min_w_0()
            .items_start()
            .gap_2()
            .child(
                Button::new(("ai-thinking", ix))
                    .small()
                    .ghost()
                    .icon(if expanded {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .label(if live && self.live.is_empty() {
                        crate::tr!("思考中", "Thinking")
                    } else {
                        crate::tr!("思考过程", "Thought process")
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if live {
                            this.live_thinking_collapsed = !this.live_thinking_collapsed;
                        } else if !this.thinking_expanded.insert(ix) {
                            this.thinking_expanded.remove(&ix);
                        }
                        this.scroller
                            .update(cx, |state, cx| state.remeasure_items(ix..ix + 1, cx));
                        cx.notify();
                    })),
            )
            .when(expanded, |row| {
                row.when_some(view, |row, view| {
                    row.child(
                        div()
                            .pl_3()
                            .border_l_1()
                            .border_color(cx.theme().border)
                            .text_color(cx.theme().muted_foreground)
                            .child(self.markdown_view(view, cx)),
                    )
                })
            })
            .into_any_element()
    }

    pub(super) fn open_link(&mut self, url: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(reference) = LogReference::from_url(url) {
            if self.busy || self.ui_busy {
                return;
            }
            let parsed = url::Url::parse(url).expect("validated citation");
            let pairs = parsed.query_pairs().collect::<BTreeMap<_, _>>();
            let mut args = json!({"action":"line", "reference":reference});
            if let (Some(search), Some(index)) = (pairs.get("search_id"), pairs.get("result_index"))
                && let Ok(index) = index.parse::<usize>()
            {
                args = json!({"action":"result", "search_id":search, "result_index":index, "reference":reference});
            }
            self.run_ui_tool(
                ToolCall {
                    id: uuid::Uuid::new_v4().to_string(),
                    name: "navigate".into(),
                    arguments: args,
                },
                window,
                cx,
            );
        } else if url.starts_with("https://") || url.starts_with("http://") {
            cx.open_url(url);
        }
    }

    fn jump_reference(
        &mut self,
        reference: LogReference,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_ui_tool(
            ToolCall {
                id: uuid::Uuid::new_v4().to_string(),
                name: "navigate".into(),
                arguments: json!({"action":"line","reference":reference}),
            },
            window,
            cx,
        );
    }
    fn run_ui_tool(&mut self, call: ToolCall, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.ui_busy {
            return;
        }
        let reference: Option<LogReference> = call
            .arguments
            .get("reference")
            .and_then(|value| serde_json::from_value(value.clone()).ok());
        let scope = self
            .scope
            .iter()
            .chain(self.reference_scopes.iter().rev())
            .find(|scope| {
                scope.lock().is_ok_and(|state| {
                    if let Some(reference) = &reference {
                        state
                            .document(reference.document_id, Some(&reference.version))
                            .is_ok()
                    } else if let Some(search) = call.arguments["search_id"].as_str() {
                        state.searches.contains_key(search)
                    } else {
                        call.arguments["document_id"].as_u64().is_some_and(|id| {
                            state
                                .document(id, call.arguments["version"].as_str())
                                .is_ok()
                        })
                    }
                })
            })
            .cloned();
        let Some(scope) = scope else {
            self.error = crate::tr!(
                "历史引用需要重新分析以确认文件内容",
                "Analyze again to refresh historical log references"
            )
            .into();
            cx.notify();
            return;
        };
        // A manual action has its own cancellation token; it cannot revive a stopped run.
        let Ok(mut state) = scope.lock().map(|state| state.clone()) else {
            return;
        };
        state.cancellation = SearchCancellation::default();
        let scope = Arc::new(Mutex::new(state));
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
        self.error.clear();
        self.ui_busy = true;
        self.ui_task = Some(cx.spawn_in(window, async move |this, cx| {
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
                this.ui_busy = false;
                if let Err(e) = result {
                    this.error = e.to_string();
                }
                cx.notify();
            });
        }));
        cx.notify();
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
        let disabled = self.busy || self.run.is_some() || self.ui_busy;
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
            .p_3()
            .border_b_1()
            .border_color(cx.theme().border)
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
        let messages =
            MessageScroller::new("ai-transcript", self.scroller.clone(), move |ix, _, cx| {
                owner.update(cx, |this, cx| this.render_message(ix, cx))
            })
            .with_list_style(gpui::StyleRefinement::default().p_3())
            .with_row_style(gpui::StyleRefinement::default().pb_4())
            .with_jump_button_label(crate::tr!("回到最新消息", "Jump to latest"))
            .with_jump_button_renderer(|button| {
                button.small().label(crate::tr!("最新消息", "Latest"))
            });
        let footer = v_flex()
            .flex_shrink_0()
            .gap_2()
            .p_3()
            .when(
                !self.conversation.notice.is_empty() && self.conversation.notice != self.error,
                |this| {
                    this.child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(self.conversation.notice.clone()),
                    )
                },
            )
            .when(
                self.run.is_some() || (self.busy && self.conversation.status == RunStatus::Running),
                |this| {
                    this.child(
                        div()
                            .id("ai-progress")
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(self.progress.clone()),
                    )
                },
            )
            .child(
                v_flex()
                    .w_full()
                    .min_w_0()
                    .gap_2()
                    .p_2()
                    .rounded(cx.theme().radius_lg)
                    .bg(cx.theme().muted)
                    .when(self.attachments_loading, |this| {
                        this.child(
                            div()
                                .text_xs()
                                .child(crate::tr!("正在添加日志…", "Adding logs…")),
                        )
                    })
                    .when(!self.draft_logs.is_empty(), |this| {
                        this.child(self.render_attachments(cx))
                    })
                    .child(
                        Textarea::new(&self.input)
                            .appearance(false)
                            .aria_label(crate::tr!("分析问题", "Analysis question"))
                            .disabled(self.busy),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                Button::new("ai-skills-menu")
                                    .small()
                                    .ghost()
                                    .label(format!(
                                        "Skills ({})",
                                        self.conversation.skill_ids.len()
                                    ))
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
                                            menu = menu.item(
                                                PopupMenuItem::new(format!(
                                                    "{} {}",
                                                    if chosen.contains(&id) {
                                                        "✓"
                                                    } else {
                                                        "○"
                                                    },
                                                    skill.name
                                                ))
                                                .on_click(window.listener_for(
                                                    &skills,
                                                    move |this, _, window, cx| {
                                                        if this.conversation.skill_ids.contains(&id)
                                                        {
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
                            .child(
                                if self.run.is_some()
                                    || (self.busy && self.conversation.status == RunStatus::Running)
                                {
                                    Button::new("ai-stop")
                                        .small()
                                        .primary()
                                        .rounded(cx.theme().radius_full())
                                        .tooltip(crate::tr!("停止生成", "Stop generating"))
                                        .label(crate::tr!("停止", "Stop"))
                                        .on_click(cx.listener(|this, _, _, cx| this.stop(cx)))
                                } else {
                                    Button::new("ai-send")
                                        .small()
                                        .primary()
                                        .icon(IconName::ArrowUp)
                                        .rounded(cx.theme().radius_full())
                                        .tooltip(crate::tr!("发送（Enter）", "Send (Enter)"))
                                        .label(crate::tr!("发送", "Send"))
                                        .disabled(
                                            self.busy
                                                || self.ui_busy
                                                || self.attachments_loading
                                                || (self.input.read(cx).value().trim().is_empty()
                                                    && self.draft_logs.is_empty()),
                                        )
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.send(false, window, cx)
                                        }))
                                },
                            ),
                    ),
            );
        body.child(header)
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .when(self.conversation.messages.is_empty() && !self.live_row, |this| {
                        this.child(v_flex().p_3().gap_2()
                            .child(div().font_semibold().child(crate::tr!("分析日志", "Analyze logs")))
                            .child(div().text_sm().text_color(cx.theme().muted_foreground).child(crate::tr!(
                                "让 AI 查找异常、关联日志、添加标记或高亮关键词。先配置模型，再输入分析问题。",
                                "Ask AI to find errors, correlate logs, add marks or highlight keywords. Configure a model, then describe your investigation."
                            ))))
                    })
                    .when(!self.conversation.messages.is_empty() || self.live_row, |this| this.child(messages)),
            )
            .child(footer)
            .into_any_element()
    }
}
