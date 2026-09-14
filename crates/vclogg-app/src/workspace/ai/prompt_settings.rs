use super::*;
use gpui_component::{
    input::{Input, InputState, Textarea, TextareaState},
    switch::Switch,
};
use vclogg_ai::Prompt;

pub(super) struct PromptEditor {
    prompt: Prompt,
    name: Entity<InputState>,
    text: Entity<TextareaState>,
}
impl AiPanel {
    fn install_prompt_editor(
        &mut self,
        prompt: Prompt,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.prompt_editor = Some(PromptEditor {
            name: cx.new(|cx| InputState::new(window, cx).default_value(prompt.name.clone())),
            text: cx.new(|cx| {
                TextareaState::new(window, cx)
                    .auto_grow(12, 20)
                    .default_value(text)
            }),
            prompt,
        });
        cx.notify();
    }
    fn edit_prompt(
        &mut self,
        prompt: Prompt,
        import: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.settings_busy(cx) {
            return;
        }
        let Some(path) = self.settings_path.clone() else {
            return;
        };
        self.busy = true;
        let generation = self.settings_generation;
        cx.spawn_in(window, async move |this, cx| {
            let selected = if import {
                rfd::AsyncFileDialog::new()
                    .add_filter("Markdown", &["md", "txt"])
                    .pick_file()
                    .await
                    .map(|file| file.path().to_path_buf())
            } else {
                None
            };
            let copy = prompt.clone();
            let result = if import && selected.is_none() {
                None
            } else {
                Some(
                    cx.background_spawn(async move {
                        if let Some(file) = selected {
                            vclogg_ai::read_prompt_text(&file)
                        } else {
                            vclogg_ai::read_prompt(&path, &copy)
                        }
                    })
                    .await,
                )
            };
            _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                if this.show_settings
                    && this.settings_generation == generation
                    && let Some(result) = result
                {
                    match result {
                        Ok(text) => this.install_prompt_editor(prompt, text, window, cx),
                        Err(error) => this.error = error.to_string(),
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub(super) fn save_prompt_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings_busy(cx) {
            return;
        }
        let Some(editor) = &self.prompt_editor else {
            return;
        };
        let mut prompt = editor.prompt.clone();
        prompt.name = editor.name.read(cx).value().trim().to_owned();
        if prompt.name.is_empty() {
            self.error = crate::tr!("请输入提示词名称", "Enter a prompt name").into();
            cx.notify();
            return;
        }
        let text = editor.text.read(cx).value().to_string();
        if let Some(old) = self.settings.prompts.iter_mut().find(|p| p.id == prompt.id) {
            *old = prompt.clone();
        } else {
            self.settings.prompts.push(prompt.clone());
        }
        self.save_settings_with_prompt(Some((prompt, text)), window, cx);
    }
    pub(super) fn render_prompt_settings(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let disabled = self.settings_busy(cx);
        let mut content = v_flex().p_3().gap_3();
        if let Some(editor) = &self.prompt_editor {
            content = content
                .child(div().text_sm().child(crate::tr!("名称", "Name")))
                .child(Input::new(&editor.name).disabled(disabled))
                .child(
                    div()
                        .text_sm()
                        .child(crate::tr!("提示词内容", "Prompt text")),
                )
                .child(Textarea::new(&editor.text).disabled(disabled))
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("ai-save-prompt")
                                .small()
                                .primary()
                                .text_label(crate::tr!("保存", "Save"))
                                .disabled(disabled)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.save_prompt_editor(window, cx)
                                })),
                        )
                        .child(
                            Button::new("ai-cancel-prompt")
                                .small()
                                .ghost()
                                .text_label(crate::tr!("取消", "Cancel"))
                                .disabled(disabled)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.prompt_editor = None;
                                    cx.notify();
                                })),
                        ),
                );
        } else {
            content = content.child(h_flex().gap_2()
                .child(Button::new("ai-add-prompt").small().text_label(crate::tr!("新增提示词…", "New prompt…")).disabled(disabled)
                    .on_click(cx.listener(|this, _, window, cx| {
                        let id = uuid::Uuid::new_v4().to_string();
                        this.install_prompt_editor(Prompt { file: format!("{id}.md"), id, name: String::new(), enabled: true }, String::new(), window, cx);
                    })))
                .child(Button::new("ai-import-rules").small().text_label(crate::tr!("添加 RULES…", "Import RULES…")).disabled(disabled)
                    .on_click(cx.listener(|this, _, window, cx| {
                        let prompt = this.settings.prompts.iter().find(|prompt| prompt.id == "rules").cloned().unwrap_or_else(|| vclogg_ai::default_prompts().remove(1));
                        this.edit_prompt(prompt, true, window, cx);
                    }))))
                .child(div().text_sm().text_color(cx.theme().muted_foreground).child(crate::tr!(
                    "启用的提示词按列表顺序用于后续分析。AIAgent 为默认行为，RULES 可补充项目规则；编辑后保存生效。",
                    "Enabled prompts apply in list order to subsequent runs. AIAgent defines default behavior; RULES adds project instructions. Save edits to apply them."
                )));
            if let Some(path) = self
                .settings_path
                .as_ref()
                .and_then(|p| p.parent())
                .map(|p| p.join("prompts"))
            {
                content = content.child(
                    Button::new("ai-open-prompts")
                        .small()
                        .ghost()
                        .text_label(crate::tr!("打开提示词目录", "Open prompt folder"))
                        .on_click(move |_, _, cx| cx.open_with_system(&path)),
                );
            }
            for prompt in self.settings.prompts.clone() {
                let id = prompt.id.clone();
                let edit = prompt.clone();
                content = content.child(
                    h_flex()
                        .gap_2()
                        .child(
                            Switch::new(SharedString::from(format!("ai-prompt-enable-{id}")))
                                .label(prompt.name)
                                .checked(prompt.enabled)
                                .disabled(disabled)
                                .on_click(cx.listener(move |this, enabled, window, cx| {
                                    if let Some(prompt) =
                                        this.settings.prompts.iter_mut().find(|p| p.id == id)
                                    {
                                        prompt.enabled = *enabled;
                                    }
                                    this.save_settings(window, cx);
                                })),
                        )
                        .child(div().flex_1())
                        .child(
                            Button::new(SharedString::from(format!(
                                "ai-prompt-edit-{}",
                                prompt.id
                            )))
                            .small()
                            .ghost()
                            .text_label(crate::tr!("编辑…", "Edit…"))
                            .disabled(disabled)
                            .on_click(cx.listener(
                                move |this, _, window, cx| {
                                    this.edit_prompt(edit.clone(), false, window, cx)
                                },
                            )),
                        ),
                );
            }
        }
        div()
            .id("ai-prompts-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .child(content)
            .into_any_element()
    }
}
