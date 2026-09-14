use super::*;
use gpui_component::switch::Switch;

impl AiPanel {
    fn scan_skills(&mut self, system: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings_busy(cx) {
            return;
        }
        self.busy = true;
        let existing = self
            .settings
            .skill_directories
            .iter()
            .map(|r| r.directory.clone())
            .collect::<Vec<_>>();
        cx.spawn_in(window, async move |this, cx| {
            let paths = if system {
                Some(existing)
            } else {
                rfd::AsyncFileDialog::new()
                    .pick_folder()
                    .await
                    .map(|folder| vec![folder.path().to_path_buf()])
            };
            let result = if let Some(mut paths) = paths {
                Some(
                    cx.background_spawn(async move {
                        if system {
                            paths.extend(vclogg_ai::system_skill_directories());
                        }
                        vclogg_ai::discover_skills(paths)
                    })
                    .await,
                )
            } else {
                None
            };
            _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                if let Some(found) = result {
                    for root in found.directories {
                        if !this
                            .settings
                            .skill_directories
                            .iter()
                            .any(|old| old.directory == root.directory)
                        {
                            this.settings.skill_directories.push(root);
                        }
                    }
                    for skill in found.skills {
                        if let Some(old) = this
                            .settings
                            .skills
                            .iter_mut()
                            .find(|old| old.directory == skill.directory)
                        {
                            old.name = skill.name;
                            old.description = skill.description;
                            old.source_directory = skill.source_directory;
                        } else {
                            this.settings.skills.push(skill);
                        }
                    }
                    this.error = if found.warnings.is_empty() {
                        crate::tr!(
                            "扫描完成，新发现的 Skills 默认关闭",
                            "Scan complete; newly discovered skills are disabled"
                        )
                        .into()
                    } else {
                        found
                            .warnings
                            .into_iter()
                            .take(3)
                            .collect::<Vec<_>>()
                            .join("\n")
                    };
                    this.save_settings(window, cx);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn refresh_skill(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings_busy(cx) {
            return;
        }
        let Some(mut skill) = self.settings.skills.iter().find(|s| s.id == id).cloned() else {
            return;
        };
        self.busy = true;
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    skill.enabled = true;
                    vclogg_ai::refresh_skill(&mut skill)?;
                    Ok::<_, anyhow::Error>(skill)
                })
                .await;
            _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(skill) => {
                        if let Some(old) = this.settings.skills.iter_mut().find(|s| s.id == id) {
                            old.name = skill.name;
                            old.description = skill.description;
                        }
                        this.save_settings(window, cx);
                    }
                    Err(error) => this.error = error.to_string(),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub(super) fn render_skill_settings(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let disabled = self.settings_busy(cx);
        let mut content = v_flex().gap_3().p_3().child(
            h_flex().gap_2().flex_wrap()
                .child(Button::new("ai-scan-skills").small().text_label(crate::tr!("扫描系统目录", "Scan system folders")).disabled(disabled)
                    .on_click(cx.listener(|this, _, window, cx| this.scan_skills(true, window, cx))))
                .child(Button::new("ai-add-skill-directory").small().text_label(crate::tr!("添加目录…", "Add folder…")).disabled(disabled)
                    .on_click(cx.listener(|this, _, window, cx| this.scan_skills(false, window, cx))))
        ).child(div().text_sm().text_color(cx.theme().muted_foreground).child(crate::tr!(
            "预置功能按文件操作、搜索与分析、标记与高亮、导航定位共享工作流。支持扫描或添加自定义目录。开关只控制是否提供工作指导，不会禁用对应工具；选择用于所有会话的后续分析。",
            "Built-in entries share file, analysis, marking and navigation workflows. Scan or add skill folders. Switches select guidance; they do not disable tools. Selections apply to future runs in all conversations."
        )));
        for root in self.settings.skill_directories.clone() {
            let id = root.id.clone();
            let remove = id.clone();
            let directory = root.directory.clone();
            content = content.child(
                v_flex()
                    .gap_1()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .pt_3()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Switch::new(SharedString::from(format!("ai-skill-root-{id}")))
                                    .label(root.name.clone())
                                    .checked(root.enabled)
                                    .disabled(disabled)
                                    .on_click(cx.listener(move |this, enabled, window, cx| {
                                        if let Some(root) = this
                                            .settings
                                            .skill_directories
                                            .iter_mut()
                                            .find(|r| r.id == id)
                                        {
                                            root.enabled = *enabled;
                                        }
                                        this.save_settings(window, cx);
                                    })),
                            )
                            .child(div().flex_1())
                            .child(
                                Button::new(SharedString::from(format!("ai-remove-root-{remove}")))
                                    .small()
                                    .ghost()
                                    .text_label(crate::tr!("移除目录", "Remove folder"))
                                    .disabled(disabled)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.settings
                                            .skill_directories
                                            .retain(|r| !r.directory.starts_with(&directory));
                                        this.settings.skills.retain(|s| {
                                            !s.directory.starts_with(&directory)
                                                && !s.source_directory.as_ref().is_some_and(
                                                    |source| source.starts_with(&directory),
                                                )
                                        });
                                        this.save_settings(window, cx);
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(root.directory.display().to_string()),
                    ),
            );
            for skill in self
                .settings
                .skills
                .clone()
                .into_iter()
                .filter(|skill| {
                    self.settings
                        .skill_directories
                        .iter()
                        .filter(|candidate| candidate.contains(skill))
                        .max_by_key(|candidate| {
                            (
                                skill.source_directory.as_ref() == Some(&candidate.directory),
                                candidate.directory.components().count(),
                            )
                        })
                        .is_some_and(|owner| owner.id == root.id)
                })
                .collect::<Vec<_>>()
            {
                content = content.child(self.skill_row(skill, disabled, cx));
            }
        }
        for skill in self
            .settings
            .skills
            .clone()
            .into_iter()
            .filter(|s| {
                !self
                    .settings
                    .skill_directories
                    .iter()
                    .any(|r| r.contains(s))
            })
            .collect::<Vec<_>>()
        {
            content = content.child(self.skill_row(skill, disabled, cx));
        }
        div()
            .id("ai-skills-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .child(content)
            .into_any_element()
    }
    fn skill_row(
        &self,
        skill: vclogg_ai::Skill,
        disabled: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = skill.id.clone();
        let remove = id.clone();
        let refresh = id.clone();
        let available = self
            .settings
            .skill_directories
            .iter()
            .filter(|root| root.contains(&skill))
            .all(|root| root.enabled);
        let path = skill.directory.join("SKILL.md");
        v_flex()
            .gap_1()
            .pl_3()
            .min_w_0()
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        Switch::new(SharedString::from(format!("ai-skill-{id}")))
                            .label(skill.name)
                            .checked(skill.enabled)
                            .disabled(disabled || !available)
                            .on_click(cx.listener(move |this, enabled, window, cx| {
                                if let Some(skill) =
                                    this.settings.skills.iter_mut().find(|s| s.id == id)
                                {
                                    skill.enabled = *enabled;
                                }
                                this.save_settings(window, cx);
                            })),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new(SharedString::from(format!("ai-view-skill-{remove}")))
                            .small()
                            .ghost()
                            .text_label(crate::tr!("查看", "View"))
                            .on_click(move |_, _, cx| cx.open_with_system(&path)),
                    )
                    .child(
                        Button::new(SharedString::from(format!("ai-refresh-skill-{refresh}")))
                            .small()
                            .ghost()
                            .text_label(crate::tr!("刷新", "Refresh"))
                            .disabled(disabled)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.refresh_skill(refresh.clone(), window, cx)
                            })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("ai-remove-skill-{remove}")))
                            .small()
                            .ghost()
                            .text_label(crate::tr!("移除", "Remove"))
                            .disabled(disabled)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.settings.skills.retain(|s| s.id != remove);
                                this.conversation.skill_ids.retain(|s| s != &remove);
                                this.save_settings(window, cx);
                            })),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(skill.description),
            )
            .into_any_element()
    }
}
