use super::*;

pub(super) fn ensure_default_workspace(settings: &mut vclogg_ai::AiSettings) -> bool {
    if settings.workspace_directory.is_some() {
        return false;
    }
    install_default_workspace(settings, crate::app_paths::default_ai_workspace_dir())
}

fn install_default_workspace(
    settings: &mut vclogg_ai::AiSettings,
    directory: Option<PathBuf>,
) -> bool {
    if settings.workspace_directory.is_some() {
        return false;
    }
    let Some(directory) = directory else {
        return false;
    };
    // Older releases inserted this exact default into the source-project list.
    // Move only that known default; never reinterpret user-selected project paths.
    settings
        .project_directories
        .retain(|path| path != &directory);
    settings.workspace_directory = Some(directory);
    true
}

impl AiPanel {
    fn choose_ai_directory(&mut self, project: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings_busy(cx) {
            return;
        }
        self.busy = true;
        cx.spawn_in(window, async move |this, cx| {
            let selected = rfd::AsyncFileDialog::new().pick_folder().await;
            let canonical = cx
                .background_spawn(async move {
                    selected
                        .map(|folder| folder.path().canonicalize())
                        .transpose()
                })
                .await;
            _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match canonical {
                    Ok(Some(path)) => {
                        let overlaps = if project {
                            this.settings.workspace_directory.as_ref() == Some(&path)
                        } else {
                            this.settings.project_directories.contains(&path)
                        };
                        if overlaps {
                            this.error = crate::tr!(
                                "工作区目录与项目目录不能相同",
                                "Choose separate workspace and project folders"
                            )
                            .into();
                        } else {
                            if project {
                                if !this.settings.project_directories.contains(&path) {
                                    this.settings.project_directories.push(path);
                                }
                            } else {
                                this.settings.workspace_directory = Some(path);
                            }
                            this.save_settings(window, cx);
                        }
                    }
                    Ok(None) => {}
                    Err(error) => this.error = error.to_string(),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn render_workspace_settings(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let disabled = self.settings_busy(cx);
        let workspace = self
            .settings
            .workspace_directory
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| {
                crate::tr!(
                    "工作区不可用，请选择目录",
                    "Workspace unavailable; choose a folder"
                )
                .into()
            });
        let mut content = v_flex().gap_3().p_3()
            .child(div().text_sm().font_semibold().child(crate::tr!("工作区目录", "Workspace folder")))
            .child(div().text_sm().child(workspace))
            .child(Button::new("ai-change-workspace-directory").small()
                .text_label(crate::tr!("更改工作区…", "Change workspace…"))
                .disabled(disabled)
                .on_click(cx.listener(|this, _, window, cx| this.choose_ai_directory(false, window, cx))))
            .child(div().text_sm().text_color(cx.theme().muted_foreground).child(crate::tr!(
                "唯一的临时文件、输出和转储目录，也是 Shell 的默认执行目录。需要编辑源文件时，先复制到此目录；不会自动清空。写入仍需确认，目录变更从下一轮生效。",
                "The single folder for temporary copies, output and dumps, and the default Shell working directory. Copy source files here before editing. Files are not automatically removed. Writes require confirmation; folder changes apply to the next run."
            )))
            .child(div().border_t_1().border_color(cx.theme().border).pt_3().text_sm().font_semibold()
                .child(crate::tr!("项目目录", "Project folders")))
            .child(div().text_sm().text_color(cx.theme().muted_foreground).child(crate::tr!(
                "源码所在目录，用于项目搜索、符号、定义和引用分析，不作为临时输出目录。读取已知绝对路径不需要添加项目。",
                "Source folders for project searches, symbols, definitions and references—not temporary output. Reading a known absolute path does not require a project."
            )))
            .child(Button::new("ai-add-project-directory").small()
                .text_label(crate::tr!("添加项目目录…", "Add project folder…"))
                .disabled(disabled)
                .on_click(cx.listener(|this, _, window, cx| this.choose_ai_directory(true, window, cx))));
        if self.settings.project_directories.is_empty() {
            content = content.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(crate::tr!("尚未添加项目目录", "No project folders added")),
            );
        }
        for path in self.settings.project_directories.clone() {
            let label = path.display().to_string();
            content = content.child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(div().flex_1().min_w_0().text_sm().child(label.clone()))
                    .child(
                        Button::new(SharedString::from(format!("ai-remove-project-{label}")))
                            .small()
                            .ghost()
                            .text_label(crate::tr!("移除", "Remove"))
                            .disabled(disabled)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.settings
                                    .project_directories
                                    .retain(|candidate| candidate != &path);
                                this.save_settings(window, cx);
                            })),
                    ),
            );
        }
        div()
            .id("ai-workspaces-scroll")
            .size_full()
            .overflow_y_scroll()
            .child(content)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_is_single_and_independent_of_projects() {
        let mut settings = vclogg_ai::AiSettings::default();
        let project = PathBuf::from("project");
        let fallback = PathBuf::from("fallback-workspace");
        settings.project_directories = vec![fallback.clone(), project.clone()];
        assert!(install_default_workspace(
            &mut settings,
            Some(fallback.clone())
        ));
        assert_eq!(settings.workspace_directory, Some(fallback.clone()));
        assert_eq!(settings.project_directories, vec![project]);
        assert!(!install_default_workspace(
            &mut settings,
            Some(PathBuf::from("replacement"))
        ));
        settings.project_directories.clear();
        assert!(!install_default_workspace(
            &mut settings,
            Some(PathBuf::from("replacement"))
        ));
        assert_eq!(settings.workspace_directory, Some(fallback));
        assert!(settings.project_directories.is_empty());
    }
}
