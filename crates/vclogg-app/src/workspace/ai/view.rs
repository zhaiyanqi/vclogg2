use super::*;
use gpui_component::{input::Textarea, text::TextView};
use gpui_message_scroller::MessageScroller;
use vclogg_ai::RunStatus;

impl AiPanel {
    fn render_message(&mut self, row_ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let start = self.transcript_rows[row_ix];
        let end = self
            .transcript_rows
            .get(row_ix + 1)
            .copied()
            .unwrap_or(self.messages.len());
        let row_id = SharedString::from(format!("ai-message-{}-{start}", self.conversation.id));
        if matches!(
            self.conversation.messages.get(start),
            Some(AgentMessage::User { .. })
        ) {
            let mut row = v_flex().id(row_id).w_full().min_w_0().items_end().gap_2();
            if let Some(attachments) = self.message_attachments.get(&start) {
                let mut cards = v_flex()
                    .debug_selector(|| "ai-user-references".into())
                    .w_full()
                    .min_w_0()
                    .items_end()
                    .gap_2();
                for (ix, attachment) in attachments.iter().enumerate() {
                    let reference = attachment.content.reference.clone();
                    cards =
                        cards.child(
                            v_flex()
                                .w_full()
                                .max_w(gpui::relative(0.9))
                                .min_w_0()
                                .p_2()
                                .gap_1()
                                .border_1()
                                .border_color(cx.theme().border)
                                .rounded(cx.theme().radius_lg)
                                .child(
                                    Button::new(SharedString::from(format!(
                                        "ai-attachment-{start}-{ix}"
                                    )))
                                    .small()
                                    .ghost()
                                    .icon(IconName::File)
                                    .text_label(attachment.content.label.clone())
                                    .disabled(self.busy || self.ui_busy)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.jump_reference(reference.clone(), window, cx)
                                    })),
                                )
                                .child(self.markdown_view(&attachment.view, cx)),
                        );
                }
                row = row.child(cards);
            }
            return row
                .child(
                    div()
                        .debug_selector(|| "ai-user-bubble".into())
                        .max_w(gpui::relative(0.9))
                        .min_w_0()
                        .px_4()
                        .py_3()
                        .rounded(cx.theme().radius_lg)
                        .bg(cx.theme().primary.opacity(0.12))
                        .child(self.markdown_view(&self.messages[start], cx)),
                )
                .into_any_element();
        }
        let live = self.live_row && row_ix + 1 == self.transcript_rows.len();
        let answer = if live {
            None
        } else {
            end.checked_sub(1).filter(|ix| matches!(&self.conversation.messages[*ix], AgentMessage::Assistant { calls, text, .. } if calls.is_empty() && !text.is_empty()))
        };
        let has_process = (start..end)
            .any(|ix| Some(ix) != answer || self.reasoning_views[ix].is_some())
            || (live && !self.reasoning.is_empty())
            || (row_ix + 1 == self.transcript_rows.len() && self.pending_tool.is_some());
        let mut row = v_flex()
            .id(row_id)
            .debug_selector(|| "ai-reply".into())
            .w_full()
            .min_w_0()
            .gap_3();
        if has_process {
            row = row.child(self.render_process(start, end, answer, live, cx));
        }
        if let Some(ix) = answer {
            row = row.child(self.markdown_view(&self.messages[ix], cx));
        }
        if live && !self.live.is_empty() {
            row = row.child(self.markdown_view(&self.live_view, cx));
        }
        row.into_any_element()
    }

    fn render_process(
        &self,
        start: usize,
        end: usize,
        answer: Option<usize>,
        live: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let expanded = self.thinking_expanded.contains(&start);
        let mut process = v_flex()
            .debug_selector(|| "ai-thinking-region".into())
            .items_start()
            .text_left()
            .w_full()
            .min_w_0()
            .gap_3()
            .child(
                h_flex()
                    .debug_selector(|| "ai-thinking-toggle".into())
                    .w_full()
                    .pb_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        Button::new(("ai-thinking", start))
                            .px_0()
                            .justify_start()
                            .small()
                            .ghost()
                            .text_label(if live && self.live.is_empty() {
                                crate::tr!("正在分析", "Analyzing")
                            } else {
                                crate::tr!("思考过程", "Thought process")
                            })
                            .child(
                                Icon::new(if expanded {
                                    IconName::ChevronDown
                                } else {
                                    IconName::ChevronRight
                                })
                                .size_4(),
                            )
                            .text_color(cx.theme().muted_foreground)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if !this.thinking_expanded.insert(start) {
                                    this.thinking_expanded.remove(&start);
                                }
                                this.remeasure_message(start, cx);
                                cx.notify();
                            })),
                    ),
            );
        if expanded {
            for ix in start..end {
                if let Some(view) = &self.reasoning_views[ix] {
                    process = process.child(
                        div()
                            .debug_selector(|| "ai-reasoning-text".into())
                            .w_full()
                            .min_w_0()
                            .child(self.markdown_view(view, cx)),
                    );
                }
                match &self.conversation.messages[ix] {
                    AgentMessage::Assistant { text, .. }
                        if Some(ix) != answer && !text.is_empty() =>
                    {
                        process = process.child(self.markdown_view(&self.messages[ix], cx));
                    }
                    AgentMessage::Tool { name, result, .. } => {
                        let details = self.expanded.contains(&ix);
                        let label = format!(
                            "{} · {}",
                            tool_label(name),
                            if result.is_error {
                                crate::tr!("失败", "Failed")
                            } else {
                                crate::tr!("完成", "Done")
                            }
                        );
                        process = process.child(
                            Button::new(("ai-tool-expand", ix))
                                .px_0()
                                .justify_start()
                                .small()
                                .ghost()
                                .icon(if details {
                                    IconName::ChevronDown
                                } else {
                                    IconName::ChevronRight
                                })
                                .text_label(label)
                                .text_color(cx.theme().muted_foreground)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if !this.expanded.insert(ix) {
                                        this.expanded.remove(&ix);
                                    }
                                    this.remeasure_message(ix, cx);
                                    cx.notify();
                                })),
                        );
                        if details {
                            process = process.child(self.markdown_view(&self.messages[ix], cx));
                            let mut references = Vec::new();
                            collect_references(&result.value, &mut references);
                            for (n, reference) in references.into_iter().enumerate() {
                                process = process.child(
                                    Button::new(SharedString::from(format!("ai-jump-{ix}-{n}")))
                                        .small()
                                        .ghost()
                                        .text_label(format!(
                                            "{} {}",
                                            crate::tr!("日志行", "Log line"),
                                            reference.line
                                        ))
                                        .disabled(self.busy || self.ui_busy)
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.jump_reference(reference.clone(), window, cx)
                                        })),
                                );
                            }
                        }
                    }
                    _ => {}
                }
            }
            if live && !self.reasoning.is_empty() {
                process = process.child(self.markdown_view(&self.live_reasoning_view, cx));
            }
            if let Some(call) = self
                .pending_tool
                .as_ref()
                .filter(|_| end == self.messages.len())
            {
                process = process.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("{}…", tool_label(&call.name))),
                );
            }
        }
        process.into_any_element()
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
    ) -> AnyElement {
        let owner = cx.weak_entity();
        let selected_view = view.clone();
        let colors = ui_theme::palette(cx);
        let style = gpui_component::text::TextViewStyle::default().selection_colors(
            colors.chat_selection_background,
            colors.chat_selection_foreground,
        );
        div().id(SharedString::from(format!("ai-text-{:?}", view.entity_id())))
            .min_w_0().w_full()
            .child(TextView::new(view).style(style).selectable(true).on_link_click(move |url, event, window, cx| {
                if !matches!(event, gpui::ClickEvent::Mouse(event) if event.up.button != MouseButton::Left) {
                    _ = owner.update(cx, |this, cx| this.open_link(url, window, cx));
                }
            }))
            .on_mouse_down(MouseButton::Right, cx.listener(move |this, event: &gpui::MouseDownEvent, window, cx| {
                this.open_message_menu(&selected_view, event.position, window, cx);
                cx.stop_propagation();
            })).into_any_element()
    }

    fn open_message_menu(
        &mut self,
        view: &Entity<gpui_component::text::TextViewState>,
        position: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let local_selection = view.read(cx).selected_text();
        let selection = gpui_base::TextSelection::selected_text(window, cx);
        let text = if local_selection.is_empty() || selection.is_empty() {
            local_selection
        } else {
            selection
        };
        let focus = window
            .focused(cx)
            .unwrap_or_else(|| self.input.focus_handle(cx));
        let panel = cx.entity();
        let documents = self
            .scope
            .as_ref()
            .and_then(|scope| scope.lock().ok())
            .map(|scope| {
                scope
                    .documents
                    .values()
                    .filter(|d| d.open)
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let busy = self.busy || self.ui_busy;
        let menu = PopupMenu::build(window, cx, move |menu, window, _cx| {
            let mut menu = menu
                .action_context(focus)
                .item(
                    PopupMenuItem::new(crate::tr!("复制", "Copy"))
                        .disabled(text.is_empty())
                        .on_click({
                            let text = text.clone();
                            move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(text.clone()))
                            }
                        }),
                )
                .item(
                    PopupMenuItem::new(crate::tr!("添加到对话框", "Add to composer"))
                        .disabled(text.trim().is_empty())
                        .on_click(window.listener_for(&panel, {
                            let text = text.clone();
                            move |this, _, window, cx| this.add_text_to_composer(&text, window, cx)
                        })),
                );
            for doc in documents {
                let text = text.clone();
                let label = format!(
                    "{} · {}",
                    crate::tr!("追加到搜索框", "Append to search box"),
                    doc.document.file_name()
                );
                menu = menu.item(PopupMenuItem::new(label)
                    .disabled(text.trim().is_empty() || busy)
                    .on_click(window.listener_for(&panel, move |this, _, window, cx| {
                        this.run_ui_tool(ToolCall { id: uuid::Uuid::new_v4().to_string(), name: "append_search".into(), arguments: json!({"document_id":doc.id,"version":doc.version,"text":text}) }, window, cx);
                    })));
            }
            menu
        });
        self.message_menu_subscription =
            Some(cx.subscribe(&menu, |this, _, _: &gpui::DismissEvent, cx| {
                this.message_menu = None;
                this.message_menu_subscription = None;
                cx.notify();
            }));
        menu.focus_handle(cx).focus(window, cx);
        self.message_menu = Some((menu, position));
        cx.notify();
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
        let disabled = self.settings_busy(cx) || self.ui_busy;
        let body = v_flex()
            .size_full()
            .min_h_0()
            .min_w_0()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground);
        let models = cx.entity();
        let header = self.render_conversation_tabs(window, cx);
        let owner = cx.entity();
        let messages =
            MessageScroller::new("ai-transcript", self.scroller.clone(), move |ix, _, cx| {
                owner.update(cx, |this, cx| this.render_message(ix, cx))
            })
            .with_list_style(gpui::StyleRefinement::default().p_3())
            .with_row_style(gpui::StyleRefinement::default().pb_4())
            .with_jump_button_label(crate::tr!("回到最新消息", "Jump to latest"))
            .with_jump_button_renderer(|button| {
                button.small().text_label(crate::tr!("最新消息", "Latest"))
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
                    .bg(cx.theme().background)
                    .border_1()
                    .border_color(cx.theme().border)
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
                            .bordered(false)
                            .aria_label(crate::tr!("分析问题", "Analysis question"))
                            .disabled(self.busy),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .flex_wrap()
                            .child(
                                Button::new("ai-add-context")
                                    .small()
                                    .ghost()
                                    .icon(IconName::Plus)
                                    .tooltip(crate::tr!("附加所选日志", "Attach selected logs"))
                                    .disabled(disabled || self.attachments_loading)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        let targets = this
                                            .workspace
                                            .read_with(cx, |workspace, cx| {
                                                workspace.ai_attachment_targets(
                                                    workspace.active_log_region,
                                                    cx,
                                                )
                                            })
                                            .unwrap_or_default();
                                        if targets.is_empty() {
                                            this.error = crate::tr!(
                                                "先在日志区域选择要附加的行",
                                                "Select log rows to attach first"
                                            )
                                            .into();
                                            cx.notify();
                                        } else {
                                            this.attach_logs(targets, window, cx);
                                        }
                                    })),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(crate::tr!("日志访问", "Log access")),
                            )
                            .child(div().flex_1().min_w_0())
                            .child(
                                Button::new("ai-model-menu")
                                    .small()
                                    .ghost()
                                    .text_label(
                                        self.settings
                                            .providers
                                            .iter()
                                            .find(|p| {
                                                Some(&p.id)
                                                    == self.conversation.provider_id.as_ref()
                                            })
                                            .map(|p| p.model.clone())
                                            .unwrap_or_else(|| {
                                                crate::tr!("选择模型", "Select model").into()
                                            }),
                                    )
                                    .disabled(disabled)
                                    .dropdown_menu(move |mut menu, window, cx| {
                                        for provider in models.read(cx).settings.providers.clone() {
                                            let id = provider.id;
                                            menu = menu.item(
                                                PopupMenuItem::new(provider.name).on_click(
                                                    window.listener_for(
                                                        &models,
                                                        move |this, _, window, cx| {
                                                            this.conversation.provider_id =
                                                                Some(id.clone());
                                                            this.settings.active_provider =
                                                                Some(id.clone());
                                                            this.save_settings(window, cx);
                                                        },
                                                    ),
                                                ),
                                            );
                                        }
                                        menu
                                    }),
                            )
                            .when(
                                self.conversation.status == RunStatus::LimitReached,
                                |this| {
                                    this.child(
                                        Button::new("ai-continue")
                                            .small()
                                            .text_label(crate::tr!("继续", "Continue"))
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
                                        .text_label(crate::tr!("停止", "Stop"))
                                        .on_click(cx.listener(|this, _, _, cx| this.stop(cx)))
                                } else {
                                    Button::new("ai-send")
                                        .small()
                                        .primary()
                                        .icon(IconName::ArrowRight)
                                        .rounded(cx.theme().radius_full())
                                        .tooltip(crate::tr!("发送（Enter）", "Send (Enter)"))
                                        .disabled(
                                            disabled
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
        let transcript_bounds = self.transcript_bounds.clone();
        body.child(header)
            .when(!self.error.is_empty(), |this| {
                this.child(div().px_3().py_2().text_xs().child(self.error.clone()))
            })
            .child(
                div()
                    .on_prepaint(move |bounds, _, _| transcript_bounds.set(Some(bounds)))
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
            .when_some(self.message_menu.clone(), |this, (menu, position)| {
                this.child(gpui::deferred(gpui::anchored().position(position)
                    .snap_to_window_with_margin(gpui::rems(0.5).to_pixels(window.rem_size()))
                    .child(menu)).with_priority(gpui_base::POPUP_PRIORITY))
            })
            .into_any_element()
    }
}

fn tool_label(name: &str) -> &str {
    match name {
        "get_context" => crate::tr!("获取当前日志", "Read current context"),
        "list_logs" => crate::tr!("列出日志文件", "List log files"),
        "read_logs" => crate::tr!("读取日志", "Read logs"),
        "locate_files" => crate::tr!("查找日志文件", "Find log files"),
        "open_file" => crate::tr!("打开文件", "Open file"),
        "close_file" => crate::tr!("关闭文件", "Close file"),
        "switch_file" => crate::tr!("切换文件", "Switch file"),
        "reveal_file" => crate::tr!("定位文件", "Reveal file"),
        "read_log_context" => crate::tr!("读取引用上下文", "Read log context"),
        "summarize_search" => crate::tr!("汇总搜索结果", "Summarize search results"),
        "search_logs" | "show_search" => crate::tr!("搜索日志", "Search logs"),
        "search_results" | "control_search" => crate::tr!("读取搜索结果", "Inspect search results"),
        "append_search" => crate::tr!("追加搜索文字", "Append search text"),
        "set_marks" => crate::tr!("标记日志", "Mark logs"),
        "highlight_keyword" => crate::tr!("高亮关键词", "Highlight keywords"),
        "text_mark" => crate::tr!("文字标记", "Annotate logs"),
        "navigate" => crate::tr!("定位日志", "Navigate to logs"),
        _ => name,
    }
}
