use super::*;
use gpui_component::{
    input::{Input, InputState, Textarea, TextareaState},
    switch::Switch,
};
use vclogg_data::AiMemoryRecord;

#[derive(Default)]
pub(super) struct SharedAiMemory;
impl gpui::Global for SharedAiMemory {}

pub(super) struct MemoryEditor {
    record: AiMemoryRecord,
    title: Entity<InputState>,
    content: Entity<TextareaState>,
}

/// Runs exclusively on the background executor, through the data-layer repository.
pub(super) fn execute_memory_tool(
    store: &StateStore,
    call: &ToolCall,
    cancellation: &vclogg_ai::Cancellation,
) -> Result<Value> {
    if cancellation.is_cancelled() {
        bail!("Memory operation stopped");
    }
    match call.name.as_str() {
        "search_memory" => {
            let query = call.arguments["query"]
                .as_str()
                .unwrap_or_default()
                .to_lowercase();
            let offset = call.arguments["offset"].as_u64().unwrap_or(0) as usize;
            let records = store
                .ai_memories()?
                .into_iter()
                .filter(|record| {
                    record.title().to_lowercase().contains(&query)
                        || record.content().to_lowercase().contains(&query)
                })
                .collect::<Vec<_>>();
            let page = records.iter().skip(offset).take(3).collect::<Vec<_>>();
            let next = offset.saturating_add(page.len());
            Ok(
                json!({"memories":page,"next_offset":(next < records.len()).then_some(next),"total":records.len()}),
            )
        }
        "save_memory" => {
            let id = call.arguments["id"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            let record = AiMemoryRecord::new(
                id,
                call.arguments["title"].as_str().unwrap_or_default().into(),
                call.arguments["content"]
                    .as_str()
                    .unwrap_or_default()
                    .into(),
                call.arguments["revision"].as_u64().unwrap_or(0),
            );
            let saved = store.save_ai_memory(&record)?;
            Ok(json!({"saved":saved}))
        }
        "delete_memory" => {
            store.delete_ai_memory(
                call.arguments["id"].as_str().unwrap_or_default(),
                call.arguments["revision"].as_u64().unwrap_or(0),
            )?;
            Ok(json!({"deleted":true}))
        }
        _ => bail!("Unknown memory operation"),
    }
}
impl AiPanel {
    pub(super) fn load_memories(&mut self, cx: &mut Context<Self>) {
        let Some(store) = self.store.clone() else {
            return;
        };
        self.memory_loading = true;
        self.memory_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { store.ai_memories() })
                .await;
            _ = this.update(cx, |this, cx| {
                this.memory_loading = false;
                match result {
                    Ok(records) => this.memories = records,
                    Err(error) => this.error = error.to_string(),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
    fn edit_memory(&mut self, record: AiMemoryRecord, window: &mut Window, cx: &mut Context<Self>) {
        let title =
            cx.new(|cx| InputState::new(window, cx).default_value(record.title().to_owned()));
        let content = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(8, 16)
                .default_value(record.content().to_owned())
        });
        self.memory_editor = Some(MemoryEditor {
            record,
            title,
            content,
        });
        self.error.clear();
        cx.notify();
    }
    pub(super) fn save_memory_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = &self.memory_editor else {
            return;
        };
        let record = AiMemoryRecord::new(
            editor.record.id().into(),
            editor.title.read(cx).value().to_string(),
            editor.content.read(cx).value().to_string(),
            editor.record.revision(),
        );
        self.mutate_memory(record, false, window, cx);
    }
    fn mutate_memory(
        &mut self,
        record: AiMemoryRecord,
        delete: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.settings_busy(cx) || self.memory_loading {
            return;
        }
        let Some(store) = self.store.clone() else {
            return;
        };
        self.busy = true;
        let generation = self.settings_generation;
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    if delete {
                        store.delete_ai_memory(record.id(), record.revision())
                    } else {
                        store.save_ai_memory(&record).map(|_| ())
                    }
                })
                .await;
            if result.is_ok() {
                gpui::AsyncApp::update_global::<SharedAiMemory, _>(cx, |_, _| {});
            }
            _ = this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(()) => {
                        if this.settings_generation == generation {
                            this.memory_editor = None;
                        }
                        this.error.clear();
                    }
                    Err(error) => this.error = error.to_string(),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub(super) fn render_memory_settings(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let disabled = self.settings_busy(cx) || self.memory_loading;
        let mut content = v_flex().p_3().gap_3().min_w_0()
            .child(Switch::new("ai-memory-enabled").label(crate::tr!("在对话中使用记忆", "Use memory in conversations"))
                .checked(self.settings.memory_enabled).disabled(disabled)
                .on_click(cx.listener(|this, enabled, window, cx| { this.settings.memory_enabled = *enabled; this.save_settings(window, cx); })))
            .child(Switch::new("ai-memory-auto-save").label(crate::tr!("自动保存有用的记忆", "Automatically save useful memories"))
                .checked(self.settings.memory_auto_save).disabled(disabled || !self.settings.memory_enabled)
                .on_click(cx.listener(|this, enabled, window, cx| { this.settings.memory_auto_save = *enabled; this.save_settings(window, cx); })))
            .child(div().text_sm().text_color(cx.theme().muted_foreground).child(crate::tr!(
                "记忆保存在本机，跨会话与窗口使用；相关内容会发送给当前模型。默认仅在你明确要求“记住”时保存，开启自动保存后由 AI 提炼。关闭使用不会删除已有记忆。",
                "Memories stay on this device and are shared across conversations and windows. Relevant content is sent to the current model. By default, saving requires an explicit request; automatic saving lets AI extract useful facts. Disabling memory keeps existing entries."
            )));
        if let Some(editor) = &self.memory_editor {
            content = content
                .child(div().text_sm().child(crate::tr!("标题", "Title")))
                .child(Input::new(&editor.title).disabled(disabled))
                .child(
                    div()
                        .text_sm()
                        .child(crate::tr!("记忆内容", "Memory content")),
                )
                .child(Textarea::new(&editor.content).disabled(disabled))
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("ai-memory-save")
                                .small()
                                .primary()
                                .text_label(crate::tr!("保存", "Save"))
                                .disabled(disabled)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.save_memory_editor(window, cx)
                                })),
                        )
                        .child(
                            Button::new("ai-memory-cancel")
                                .small()
                                .ghost()
                                .text_label(crate::tr!("取消", "Cancel"))
                                .disabled(disabled)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.memory_editor = None;
                                    cx.notify();
                                })),
                        ),
                );
        } else {
            content = content.child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("ai-memory-add")
                            .small()
                            .text_label(crate::tr!("新增记忆…", "New memory…"))
                            .disabled(disabled)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.edit_memory(
                                    AiMemoryRecord::new(
                                        uuid::Uuid::new_v4().to_string(),
                                        String::new(),
                                        String::new(),
                                        0,
                                    ),
                                    window,
                                    cx,
                                )
                            })),
                    )
                    .child(
                        Button::new("ai-memory-refresh")
                            .small()
                            .ghost()
                            .text_label(crate::tr!("刷新", "Refresh"))
                            .disabled(disabled)
                            .on_click(cx.listener(|this, _, _, cx| this.load_memories(cx))),
                    ),
            );
            if self.memory_loading {
                content = content.child(
                    div()
                        .text_sm()
                        .child(crate::tr!("正在读取记忆…", "Loading memories…")),
                );
            } else if self.memories.is_empty() {
                content = content.child(div().text_sm().text_color(cx.theme().muted_foreground).child(crate::tr!("暂无记忆。可以新增，或在对话中说“记住”。", "No memories yet. Add one here or ask the assistant to remember something.")));
            }
            for record in self.memories.clone() {
                let mut preview = record.content().chars().take(240).collect::<String>();
                if record.content().chars().nth(240).is_some() {
                    preview.push('…');
                }
                let edit = record.clone();
                let delete = record.clone();
                content = content.child(
                    v_flex()
                        .gap_1()
                        .py_2()
                        .border_b_1()
                        .border_color(cx.theme().border)
                        .child(
                            h_flex()
                                .gap_2()
                                .min_w_0()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .text_sm()
                                        .child(record.title().to_owned()),
                                )
                                .child(
                                    Button::new(SharedString::from(format!(
                                        "ai-memory-edit-{}",
                                        record.id()
                                    )))
                                    .small()
                                    .ghost()
                                    .text_label(crate::tr!("编辑…", "Edit…"))
                                    .disabled(disabled)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.edit_memory(edit.clone(), window, cx)
                                    })),
                                )
                                .child(
                                    Button::new(SharedString::from(format!(
                                        "ai-memory-delete-{}",
                                        record.id()
                                    )))
                                    .small()
                                    .ghost()
                                    .text_label(crate::tr!("删除", "Delete"))
                                    .disabled(disabled)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.mutate_memory(delete.clone(), true, window, cx)
                                    })),
                                ),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(preview),
                        ),
                );
            }
        }
        div()
            .id("ai-memory-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .child(content)
            .into_any_element()
    }
}
