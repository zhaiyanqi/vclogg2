use super::*;

// Paint after priority-0 table chrome, while staying below component popups and dialogs.
const WORKSPACE_OVERLAY_PRIORITY: usize = 1;
const _: () = assert!(WORKSPACE_OVERLAY_PRIORITY < POPUP_PRIORITY);

pub(super) fn deferred_workspace_overlay(child: impl IntoElement) -> impl IntoElement {
    deferred(child).with_priority(WORKSPACE_OVERLAY_PRIORITY)
}

impl Workspace {
    /// 菜单按钮为 28px 高、11px 横向内边距、8px 圆角和 12px 常规字重。
    /// `Button` 的 size 预设只有 24/32px 两档，够不到 28px；独立文字层同时避免
    /// 英文字母在组件标签的紧凑行高中被裁切。
    pub(super) fn title_bar_menu_button(
        id: &'static str,
        label: &'static str,
    ) -> TitleBarMenuButton {
        TitleBarMenuButton {
            button: Button::new(id)
                .small()
                .ghost()
                .h(px(28.))
                .px(px(11.))
                .rounded(px(8.))
                .font_weight(FontWeight::NORMAL)
                .child(
                    div()
                        .text_size(px(12.))
                        .line_height(relative(1.25))
                        .child(label),
                ),
        }
        .accessibility_id(id)
        .aria_label(label)
    }

    /// 下拉菜单使用已挂载的工作区根焦点查询动作快捷键，确保首帧就按完整内容计算宽度。
    /// 弹层自身的焦点要到下一帧才进入分发树，不能作为稳定的菜单布局依据。
    pub(super) fn popup_menu_with_workspace_action_context(
        menu: PopupMenu,
        workspace: &Entity<Self>,
        cx: &App,
    ) -> PopupMenu {
        menu.action_context(workspace.read(cx).focus_handle.clone())
    }

    pub(super) fn render_title_bar(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let _performance_scope = crate::ui_performance::scope("Workspace::render_title_bar");
        let workspace = cx.entity();
        let has_document = self.active_document().is_some();
        let has_selected_log_rows = if self.active_log_region == LogRegion::GlobalResults
            && self.global_search.results_visible
        {
            self.global_table.read(cx).delegate().selected_rows_count() > 0
        } else {
            self.selected_source_row.is_some()
        };
        let active_file_is_pinned = self.active_file_is_pinned();
        let auto_follow = self
            .active_document()
            .is_some_and(|tab| tab.view.auto_follow);
        let show_line_numbers = self
            .active_document()
            .is_none_or(|tab| tab.view.show_line_numbers);
        let show_row_separators = self
            .active_document()
            .is_some_and(|tab| tab.view.show_row_separators);
        let word_wrap = if self.active_log_region == LogRegion::GlobalResults
            && self.global_search.results_visible
            && self.global_search.scope.owns_global_word_wrap()
        {
            self.global_viewport.is_wrapped()
        } else {
            self.active_document().is_some_and(|tab| tab.view.word_wrap)
        };
        let show_full_path = self.app_settings.show_full_path;
        let highlight_log_levels = self.app_settings.highlight_log_levels;
        let highlight_matches = self.app_settings.highlight_matches;
        let case_sensitive = self.case_sensitive;
        let regex = self.regex;
        let tab_count = self.tabs.len();
        let active_encoding = self.active_document().map(|tab| {
            (
                tab.id,
                SharedString::from(if tab.load_state == DocumentLoadState::Opening {
                    crate::tr!("检测中", "Detecting").to_string()
                } else {
                    tab.document.metadata().encoding_name.clone()
                }),
            )
        });

        let file_workspace = workspace.clone();
        let file_menu = Self::title_bar_menu_button("title-menu-file", crate::tr!("文件", "File"))
            .dropdown_menu(move |menu, window, cx| {
                let menu =
                    Self::popup_menu_with_workspace_action_context(menu, &file_workspace, cx);
                let open = window.listener_for(&file_workspace, |this, _, window, cx| {
                    this.open_files(&OpenFiles, window, cx);
                });
                let new_window = window.listener_for(&file_workspace, |this, _, window, cx| {
                    this.new_window(&NewWindow, window, cx);
                });
                let reload = window.listener_for(&file_workspace, |this, _, window, cx| {
                    this.reload_active(&ReloadActive, window, cx);
                });
                let close = window.listener_for(&file_workspace, |this, _, window, cx| {
                    this.close_active_tab(&CloseActiveTab, window, cx);
                });
                let history = window.listener_for(&file_workspace, |this, _, window, cx| {
                    this.open_history_dialog(window, cx);
                });
                let reveal = window.listener_for(&file_workspace, |this, _, window, cx| {
                    let Some(document_id) = this.active_document().map(|tab| tab.id) else {
                        return;
                    };
                    this.reveal_tab_file(document_id, window, cx);
                });
                let close_others = window.listener_for(&file_workspace, |this, _, window, cx| {
                    this.close_tab_group(this.active_tab_id, TabCloseGroup::Others, window, cx);
                });
                let close_all = window.listener_for(&file_workspace, |this, _, window, cx| {
                    this.close_tab_group(this.active_tab_id, TabCloseGroup::All, window, cx);
                });
                menu.item(
                    PopupMenuItem::new(crate::tr!("打开…", "Open…"))
                        .icon(IconName::FolderOpen)
                        .action(Box::new(OpenFiles))
                        .on_click(open),
                )
                .item(
                    PopupMenuItem::new(crate::tr!("新窗口", "New window"))
                        .action(Box::new(NewWindow))
                        .on_click(new_window),
                )
                .item(
                    PopupMenuItem::new(crate::tr!("重新加载", "Reload"))
                        .action(Box::new(ReloadActive))
                        .disabled(!has_document)
                        .on_click(reload),
                )
                .item(
                    PopupMenuItem::new(crate::tr!(
                        "在文件资源管理器中显示",
                        "Show in File Explorer"
                    ))
                    .disabled(!has_document)
                    .on_click(reveal),
                )
                .item(
                    PopupMenuItem::new(crate::tr!("历史…", "History…"))
                        .disabled(file_workspace.read_with(cx, |this, _| {
                            this.persistence.store.is_none() || this.history_dialog_loading
                        }))
                        .on_click(history),
                )
                .separator()
                .item(
                    PopupMenuItem::new(crate::tr!("关闭标签", "Close tab"))
                        .action(Box::new(CloseActiveTab))
                        .on_click(close),
                )
                .item(
                    PopupMenuItem::new(crate::tr!("关闭其他标签页", "Close other tabs"))
                        .disabled(tab_count < 2)
                        .on_click(close_others),
                )
                .item(
                    PopupMenuItem::new(crate::tr!("关闭全部标签页", "Close all tabs"))
                        .on_click(close_all),
                )
                .separator()
                .item(
                    PopupMenuItem::new(crate::tr!("退出 VCLogg2", "Quit VCLogg2"))
                        .on_click(|_, _, cx| cx.quit()),
                )
            });

        let edit_workspace = workspace.clone();
        let edit_menu = Self::title_bar_menu_button("title-menu-edit", crate::tr!("编辑", "Edit"))
            .dropdown_menu(move |menu, window, cx| {
                let menu =
                    Self::popup_menu_with_workspace_action_context(menu, &edit_workspace, cx);
                let copy = window.listener_for(&edit_workspace, |this, _, window, cx| {
                    this.copy_current_line(&CopyCurrentLine, window, cx);
                });
                let copy_with_number =
                    window.listener_for(&edit_workspace, |this, _, window, cx| {
                        this.copy_current_line_with_number(&CopyCurrentLineWithNumber, window, cx);
                    });
                let select_all = window.listener_for(&edit_workspace, |this, _, window, cx| {
                    this.select_all_rows(&SelectAllRows, window, cx);
                });
                let copy_path = window.listener_for(&edit_workspace, |this, _, window, cx| {
                    this.copy_file_path(&CopyFilePath, window, cx);
                });
                let go_to_line = window.listener_for(&edit_workspace, |this, _, window, cx| {
                    this.open_go_to_line(&GoToLine, window, cx);
                });
                let find = window.listener_for(&edit_workspace, |this, _, window, cx| {
                    this.focus_search(&FocusSearch, window, cx);
                });
                let clear_search = window.listener_for(&edit_workspace, |this, _, window, cx| {
                    this.clear_search(window, cx);
                });
                menu.item(
                    PopupMenuItem::new(crate::tr!("复制当前行", "Copy current line"))
                        .action(Box::new(CopyCurrentLine))
                        .disabled(!has_document || !has_selected_log_rows)
                        .on_click(copy),
                )
                .item(
                    PopupMenuItem::new(crate::tr!(
                        "复制当前行（含行号）",
                        "Copy current line with number"
                    ))
                    .action(Box::new(CopyCurrentLineWithNumber))
                    .disabled(!has_document || !has_selected_log_rows)
                    .on_click(copy_with_number),
                )
                .item(
                    PopupMenuItem::new(crate::tr!("复制文件路径", "Copy file path"))
                        .action(Box::new(CopyFilePath))
                        .disabled(!has_document)
                        .on_click(copy_path),
                )
                .item(
                    PopupMenuItem::new(crate::tr!("全选行", "Select all lines"))
                        .action(Box::new(SelectAllRows))
                        .disabled(!has_document)
                        .on_click(select_all),
                )
                .separator()
                .item(
                    PopupMenuItem::new(crate::tr!("查找", "Find"))
                        .action(Box::new(FocusSearch))
                        .disabled(!has_document)
                        .on_click(find),
                )
                .item(
                    PopupMenuItem::new(crate::tr!("转到行…", "Go to line…"))
                        .action(Box::new(GoToLine))
                        .disabled(!has_document)
                        .on_click(go_to_line),
                )
                .item(
                    PopupMenuItem::new(crate::tr!("清除搜索结果", "Clear search results"))
                        .action(Box::new(ClearSearch))
                        .disabled(!has_document)
                        .on_click(clear_search),
                )
            });

        let view_workspace = workspace.clone();
        let view_menu = Self::title_bar_menu_button("title-menu-view", crate::tr!("视图", "View"))
            .dropdown_menu(move |menu, window, cx| {
                let menu =
                    Self::popup_menu_with_workspace_action_context(menu, &view_workspace, cx);
                let fullscreen_label = if window.is_fullscreen() {
                    crate::tr!("退出全屏", "Exit full screen")
                } else {
                    crate::tr!("进入全屏", "Enter full screen")
                };
                let toggle_auto_follow = {
                    let workspace = view_workspace.clone();
                    window.listener_for(&workspace, |this, _, window, cx| {
                        this.toggle_auto_follow(window, cx);
                    })
                };
                let toggle_line_numbers = {
                    let workspace = view_workspace.clone();
                    window.listener_for(&workspace, |this, _, window, cx| {
                        this.toggle_line_numbers(window, cx);
                    })
                };
                let toggle_row_separators = window
                    .listener_for(&view_workspace, |this, _, window, cx| {
                        this.toggle_row_separators(window, cx)
                    });
                let toggle_word_wrap =
                    window.listener_for(&view_workspace, |this, _, window, cx| {
                        this.toggle_word_wrap(&ToggleWordWrap, window, cx);
                    });
                let jump_to_start = window.listener_for(&view_workspace, |this, _, window, cx| {
                    this.jump_to_start(&JumpToStart, window, cx);
                });
                let jump_to_end = window.listener_for(&view_workspace, |this, _, window, cx| {
                    this.jump_to_end(&JumpToEnd, window, cx);
                });
                let toggle_fullscreen =
                    window.listener_for(&view_workspace, |this, _, window, cx| {
                        this.toggle_fullscreen(&ToggleFullscreen, window, cx);
                    });
                let toggle_full_path =
                    window.listener_for(&view_workspace, |this, _, window, cx| {
                        this.update_app_setting(
                            |settings| settings.show_full_path = !settings.show_full_path,
                            window,
                            cx,
                        );
                    });
                menu.item(
                    PopupMenuItem::new(crate::tr!("自动换行", "Word wrap"))
                        .action(Box::new(ToggleWordWrap))
                        .checked(word_wrap)
                        .disabled(!has_document)
                        .on_click(toggle_word_wrap),
                )
                .item(
                    PopupMenuItem::new(crate::tr!("显示行号", "Show line numbers"))
                        .checked(show_line_numbers)
                        .disabled(!has_document)
                        .on_click(toggle_line_numbers),
                )
                .item(
                    PopupMenuItem::new(crate::tr!("日志分隔线", "Log separators"))
                        .checked(show_row_separators)
                        .disabled(!has_document)
                        .on_click(toggle_row_separators),
                )
                .item(
                    PopupMenuItem::new(crate::tr!("显示完整路径", "Show full path"))
                        .checked(show_full_path)
                        .on_click(toggle_full_path),
                )
                .item(
                    PopupMenuItem::new(crate::tr!("末尾跟随", "Follow end"))
                        .checked(auto_follow)
                        .disabled(!has_document)
                        .on_click(toggle_auto_follow),
                )
                .separator()
                .item(
                    PopupMenuItem::new(crate::tr!("文件开头", "Start of file"))
                        .action(Box::new(JumpToStart))
                        .disabled(!has_document)
                        .on_click(jump_to_start),
                )
                .item(
                    PopupMenuItem::new(crate::tr!("文件末尾", "End of file"))
                        .action(Box::new(JumpToEnd))
                        .disabled(!has_document)
                        .on_click(jump_to_end),
                )
                .separator()
                .item(
                    PopupMenuItem::new(fullscreen_label)
                        .action(Box::new(ToggleFullscreen))
                        .on_click(toggle_fullscreen),
                )
            });

        let tools_workspace = workspace.clone();
        let tools_menu =
            Self::title_bar_menu_button("title-menu-tools", crate::tr!("工具", "Tools"))
                .dropdown_menu(move |menu, window, cx| {
                    let menu =
                        Self::popup_menu_with_workspace_action_context(menu, &tools_workspace, cx);
                    let predefined_filters =
                        window.listener_for(&tools_workspace, |this, _, window, cx| {
                            this.open_predefined_filters_dialog(window, cx);
                        });
                    let clear_history =
                        window.listener_for(&tools_workspace, |this, _, window, cx| {
                            this.replace_search_history(Vec::new(), window, cx);
                            window.push_notification(
                                crate::tr!("已清除搜索历史", "Search history cleared"),
                                cx,
                            );
                        });
                    let settings = window.listener_for(&tools_workspace, |this, _, window, cx| {
                        this.open_settings_dialog(None, window, cx);
                    });
                    let (history_empty, settings_saving) = tools_workspace
                        .read_with(cx, |this, _| {
                            (this.search_history.is_empty(), this.settings_saving)
                        });
                    menu.item(
                        PopupMenuItem::new(crate::tr!("预定义过滤器…", "Predefined filters…"))
                            .icon(IconName::Settings2)
                            .on_click(predefined_filters),
                    )
                    .item(
                        PopupMenuItem::new(crate::tr!("清除搜索历史", "Clear search history"))
                            .disabled(history_empty)
                            .on_click(clear_history),
                    )
                    .separator()
                    .item(
                        PopupMenuItem::new(crate::tr!("设置", "Settings"))
                            .action(Box::new(OpenSettings))
                            .disabled(settings_saving)
                            .on_click(settings),
                    )
                });

        let highlight_workspace = workspace.clone();
        let highlight_menu =
            Self::title_bar_menu_button("title-menu-highlight", crate::tr!("高亮", "Highlight"))
                .dropdown_menu(move |menu, window, cx| {
                    let menu = Self::popup_menu_with_workspace_action_context(
                        menu,
                        &highlight_workspace,
                        cx,
                    );
                    let manage_labels =
                        window.listener_for(&highlight_workspace, |this, _, window, cx| {
                            this.open_color_labels_dialog(window, cx);
                        });
                    let toggle_marked =
                        window.listener_for(&highlight_workspace, |this, _, window, cx| {
                            this.toggle_marked_row(&ToggleMarkedRow, window, cx);
                        });
                    let cycle_color =
                        window.listener_for(&highlight_workspace, |this, _, window, cx| {
                            this.cycle_color_label(&CycleColorLabel, window, cx);
                        });
                    let toggle_levels =
                        window.listener_for(&highlight_workspace, |this, _, window, cx| {
                            this.update_app_setting(
                                |settings| {
                                    settings.highlight_log_levels = !settings.highlight_log_levels
                                },
                                window,
                                cx,
                            );
                        });
                    let toggle_match_highlight =
                        window.listener_for(&highlight_workspace, |this, _, window, cx| {
                            this.update_app_setting(
                                |settings| settings.highlight_matches = !settings.highlight_matches,
                                window,
                                cx,
                            );
                        });
                    let toggle_case =
                        window.listener_for(&highlight_workspace, |this, _, window, cx| {
                            this.toggle_case_sensitive(&ToggleCaseSensitive, window, cx);
                        });
                    let toggle_regex =
                        window.listener_for(&highlight_workspace, |this, _, window, cx| {
                            this.toggle_regex(&ToggleRegex, window, cx);
                        });
                    let clear_highlight =
                        window.listener_for(&highlight_workspace, |this, _, window, cx| {
                            this.clear_search(window, cx);
                        });
                    menu.item(
                        PopupMenuItem::new(crate::tr!("日志级别着色", "Log-level coloring"))
                            .checked(highlight_log_levels)
                            .on_click(toggle_levels),
                    )
                    .item(
                        PopupMenuItem::new(crate::tr!("高亮搜索匹配", "Highlight search matches"))
                            .checked(highlight_matches)
                            .on_click(toggle_match_highlight),
                    )
                    .separator()
                    .item(
                        PopupMenuItem::new(crate::tr!("标记行", "Mark lines"))
                            .action(Box::new(ToggleMarkedRow))
                            .disabled(!has_document || !has_selected_log_rows)
                            .on_click(toggle_marked),
                    )
                    .item(
                        PopupMenuItem::new(crate::tr!("轮换颜色", "Cycle color"))
                            .action(Box::new(CycleColorLabel))
                            .disabled(!has_document || !has_selected_log_rows)
                            .on_click(cycle_color),
                    )
                    .item(
                        PopupMenuItem::new(crate::tr!("高亮配置…", "Highlight settings…"))
                            .icon(IconName::Settings2)
                            .disabled(highlight_workspace.read_with(cx, |this, _| {
                                this.history_loading || this.color_labels_saving
                            }))
                            .on_click(manage_labels),
                    )
                    .separator()
                    .item(
                        PopupMenuItem::new(crate::tr!("搜索时区分大小写", "Case-sensitive search"))
                            .action(Box::new(ToggleCaseSensitive))
                            .checked(case_sensitive)
                            .disabled(!has_document)
                            .on_click(toggle_case),
                    )
                    .item(
                        PopupMenuItem::new(crate::tr!("使用正则表达式", "Use regular expressions"))
                            .action(Box::new(ToggleRegex))
                            .checked(regex)
                            .disabled(!has_document)
                            .on_click(toggle_regex),
                    )
                    .item(
                        PopupMenuItem::new(crate::tr!("清除搜索高亮", "Clear search highlighting"))
                            .action(Box::new(ClearSearch))
                            .disabled(!has_document)
                            .on_click(clear_highlight),
                    )
                });

        let encoding_menu = if let Some((document_id, encoding_name)) = active_encoding.clone() {
            let workspace = workspace.clone();
            let menu_encoding_name = encoding_name.clone();
            Self::title_bar_menu_button("title-menu-encoding", crate::tr!("编码", "Encoding"))
                .disabled(self.open_task.is_some())
                .dropdown_menu(move |menu, window, cx| {
                    Self::build_encoding_menu(
                        Self::popup_menu_with_workspace_action_context(menu, &workspace, cx),
                        document_id,
                        menu_encoding_name.clone(),
                        workspace.clone(),
                        window,
                    )
                })
                .into_any_element()
        } else {
            Self::title_bar_menu_button("title-menu-encoding", crate::tr!("编码", "Encoding"))
                .disabled(true)
                .into_any_element()
        };

        let favorite_workspace = workspace.clone();
        let favorite_menu =
            Self::title_bar_menu_button("title-menu-favorite", crate::tr!("收藏", "Favorites"))
                .dropdown_menu(move |menu, window, cx| {
                    let menu = Self::popup_menu_with_workspace_action_context(
                        menu,
                        &favorite_workspace,
                        cx,
                    );
                    let toggle = window.listener_for(&favorite_workspace, |this, _, window, cx| {
                        this.toggle_active_file_pinned(window, cx);
                    });
                    let clear = window.listener_for(&favorite_workspace, |this, _, window, cx| {
                        this.clear_pinned_files(window, cx);
                    });
                    let (pinned_files, busy, pinned_updating) =
                        favorite_workspace.read_with(cx, |this, _| {
                            (
                                this.pinned_files.clone(),
                                this.open_task.is_some(),
                                this.history_loading || this.pinned_updating,
                            )
                        });
                    let mut menu = menu
                        .item(
                            PopupMenuItem::new(if active_file_is_pinned {
                                crate::tr!("取消收藏文件", "Remove from favorites")
                            } else {
                                crate::tr!("收藏当前文件", "Favorite current file")
                            })
                            .checked(active_file_is_pinned)
                            .disabled(!has_document || pinned_updating)
                            .on_click(toggle),
                        )
                        .separator();
                    if pinned_files.is_empty() {
                        menu = menu.item(
                            PopupMenuItem::new(crate::tr!("暂无收藏文件", "No favorite files"))
                                .disabled(true),
                        );
                    } else {
                        for file in &pinned_files {
                            let path = file.path.clone();
                            let open = window.listener_for(
                                &favorite_workspace,
                                move |this, _, window, cx| {
                                    this.open_recent_file(path.clone(), window, cx);
                                },
                            );
                            menu = menu.item(
                                PopupMenuItem::new(recent_file_label(file))
                                    .disabled(busy)
                                    .on_click(open),
                            );
                        }
                    }
                    menu.separator().item(
                        PopupMenuItem::new(crate::tr!("清空收藏", "Clear favorites"))
                            .disabled(pinned_files.is_empty() || pinned_updating)
                            .on_click(clear),
                    )
                });

        let help_workspace = workspace;
        let help_menu = Self::title_bar_menu_button("title-menu-help", crate::tr!("帮助", "Help"))
            .dropdown_menu(move |menu, window, cx| {
                let menu =
                    Self::popup_menu_with_workspace_action_context(menu, &help_workspace, cx);
                let open_releases = window.listener_for(&help_workspace, |_, _, _, cx| {
                    cx.open_url(GITHUB_RELEASES_URL);
                });
                let about = window.listener_for(&help_workspace, |this, _, window, cx| {
                    this.open_settings_dialog(Some(SettingsCategory::About), window, cx);
                });
                let shortcuts = window.listener_for(&help_workspace, |this, _, window, cx| {
                    this.open_settings_dialog(Some(SettingsCategory::Shortcuts), window, cx);
                });
                let settings_saving = help_workspace.read_with(cx, |this, _| this.settings_saving);
                menu.item(
                    PopupMenuItem::new(crate::tr!("键盘快捷键", "Keyboard shortcuts"))
                        .disabled(settings_saving)
                        .on_click(shortcuts),
                )
                .item(
                    PopupMenuItem::new(crate::tr!("检查更新", "Check for updates"))
                        .on_click(open_releases),
                )
                .separator()
                .item(
                    PopupMenuItem::new(format!(
                        "{} ver.{}",
                        crate::tr!("关于", "About"),
                        crate::build_info::VERSION
                    ))
                    .disabled(settings_saving)
                    .on_click(about),
                )
            });

        let colors = ui_theme::palette(cx);
        TitleBar::new()
            .when(cfg!(target_os = "macos") && window.is_fullscreen(), |bar| {
                bar.pl_0()
            })
            .h(px(36.))
            .border_b_0()
            .bg(ui_theme::header_material(&colors))
            .child(
                h_flex()
                    .relative()
                    .w_full()
                    .h_full()
                    .items_center()
                    .child(ui_theme::glass_sheen_layer(&colors))
                    .child(ui_theme::material_highlight_line(&colors))
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .right_0()
                            .bottom_0()
                            .left_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(12.))
                            .font_weight(FontWeight(620.))
                            .text_color(colors.foreground.opacity(0.86))
                            .child("VCLogg2"),
                    )
                    .child(
                        h_flex()
                            .h_full()
                            .gap_0()
                            .child(file_menu)
                            .child(edit_menu)
                            .child(view_menu)
                            .child(tools_menu)
                            .child(highlight_menu)
                            .child(encoding_menu)
                            .child(favorite_menu)
                            .child(help_menu),
                    ),
            )
    }

    pub(super) fn render_file_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let _performance_scope = crate::ui_performance::scope("Workspace::render_file_toolbar");
        let open_files_tooltip = if cfg!(target_os = "macos") {
            crate::tr!("打开日志…（Cmd+O）", "Open log… (Cmd+O)")
        } else {
            crate::tr!("打开日志…（Ctrl+O）", "Open log… (Ctrl+O)")
        };
        let auto_follow = self
            .active_document()
            .is_some_and(|tab| tab.view.auto_follow);
        let follow_available = self
            .active_document()
            .is_some_and(|tab| tab.load_state == DocumentLoadState::Ready);
        let workspace = cx.entity();
        let has_document = self.active_document().is_some();
        let active_file_is_pinned = self.active_file_is_pinned();
        let active_encoding = self.active_document().map(|tab| {
            (
                tab.id,
                SharedString::from(if tab.load_state == DocumentLoadState::Opening {
                    crate::tr!("检测中", "Detecting").to_string()
                } else {
                    tab.document.metadata().encoding_name.clone()
                }),
            )
        });
        let file_size = self
            .active_document()
            .map(|tab| format_bytes(tab.document.metadata().file_size));
        let line_position = self.active_document().map(|tab| {
            format!(
                "Ln {}/{}",
                self.selected_source_row.map_or(1, |row| row + 1),
                tab.document.source_line_count()
            )
        });

        let colors = ui_theme::palette(cx);
        // 工具栏内四个圆角方钮为 34px 见方、3px 间距，与 `Button` 的 24/32px
        // 预设都对不上，所以逐个显式给尺寸。
        let toolbar_icon_button = |button: Button| {
            button
                .ghost()
                .w(px(34.))
                .h(px(34.))
                .rounded(px(10.))
                .flex_shrink_0()
        };
        // file-meta 的每一项之间是一条 `--divider-soft` 竖线，首项不画。
        let file_meta_item = |text: String, leading_divider: bool| {
            div()
                .px(px(9.))
                .text_size(px(11.))
                .text_color(colors.muted_foreground)
                .when(leading_divider, |item| {
                    item.border_l_1().border_color(colors.divider)
                })
                .child(text)
        };

        h_flex()
            .relative()
            .w_full()
            .min_h(px(44.))
            .flex_shrink_0()
            .items_center()
            .gap(px(9.))
            .px(px(12.))
            .py(px(4.))
            .bg(ui_theme::header_material(&colors))
            .border_b_1()
            .border_color(cx.theme().border)
            .child(ui_theme::glass_sheen_layer(&colors))
            .child(
                h_flex()
                    .gap(px(3.))
                    .flex_shrink_0()
                    .child(toolbar_icon_button(
                        Button::new("open-files")
                            .icon(IconName::FolderOpen)
                            .tooltip(open_files_tooltip)
                            .loading(matches!(self.activity, Activity::Opening))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_files(&OpenFiles, window, cx);
                            })),
                    ))
                    .child(toolbar_icon_button(
                        Button::new("reload-active-file")
                            .icon(IconName::Redo)
                            .tooltip(crate::tr!("重新加载（F5）", "Reload (F5)"))
                            .disabled(!has_document)
                            .loading(matches!(self.activity, Activity::Opening))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.reload_active(&ReloadActive, window, cx);
                            })),
                    ))
                    .child(toolbar_icon_button(
                        Button::new("toggle-file-pinned")
                            .icon(if active_file_is_pinned {
                                IconName::StarFill
                            } else {
                                IconName::Star
                            })
                            .selected(active_file_is_pinned)
                            .tooltip(if active_file_is_pinned {
                                crate::tr!("取消收藏当前文件", "Remove current file from favorites")
                            } else {
                                crate::tr!("收藏当前文件", "Favorite current file")
                            })
                            .loading(self.pinned_updating)
                            .disabled(!has_document || self.history_loading)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_active_file_pinned(window, cx);
                            })),
                    ))
                    .child(toolbar_icon_button(
                        Button::new("toggle-auto-follow")
                            .icon(IconName::ArrowDown)
                            .selected(auto_follow)
                            .tooltip(if auto_follow {
                                crate::tr!("关闭末尾跟随", "Disable follow end")
                            } else {
                                crate::tr!("开启末尾跟随", "Enable follow end")
                            })
                            .disabled(!follow_available)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_auto_follow(window, cx);
                            })),
                    )),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .h(px(36.))
                    .items_center()
                    .gap(px(8.))
                    .px(px(12.))
                    .rounded(px(12.))
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(colors.control_surface)
                    .text_size(px(12.))
                    .text_color(cx.theme().foreground)
                    .child(
                        div()
                            .text_color(cx.theme().primary)
                            .child(Icon::new(IconName::File).xsmall()),
                    )
                    .child(div().min_w_0().flex_1().truncate().child(
                        self.active_document().map_or_else(
                            || crate::tr!("未打开文件", "No file open").to_string(),
                            |tab| {
                                if self.app_settings.show_full_path {
                                    tab.document.path().display().to_string()
                                } else {
                                    tab.file.title.to_string()
                                }
                            },
                        ),
                    )),
            )
            .child(
                h_flex()
                    .h(px(36.))
                    .flex_shrink_0()
                    .items_center()
                    .when_some(file_size, |meta, file_size| {
                        meta.child(file_meta_item(file_size, false))
                    })
                    .when_some(active_encoding, |meta, (document_id, encoding_name)| {
                        let menu_encoding_name = encoding_name.clone();
                        let workspace = workspace.clone();
                        meta.child(
                            Button::new("document-encoding")
                                .small()
                                .ghost()
                                .label(encoding_name)
                                .h(px(26.))
                                .px(px(9.))
                                .rounded(px(8.))
                                .text_size(px(11.))
                                .disabled(self.open_task.is_some())
                                .dropdown_menu(move |menu, window, cx| {
                                    Self::build_encoding_menu(
                                        Self::popup_menu_with_workspace_action_context(
                                            menu, &workspace, cx,
                                        ),
                                        document_id,
                                        menu_encoding_name.clone(),
                                        workspace.clone(),
                                        window,
                                    )
                                }),
                        )
                    })
                    .when_some(line_position, |meta, line_position| {
                        meta.child(file_meta_item(line_position, true))
                    }),
            )
    }

    pub(super) fn render_tabs(
        &self,
        has_other_window: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let _performance_scope = crate::ui_performance::scope("Workspace::render_tabs");
        let workspace = cx.entity();
        let source_workspace = cx.weak_entity();
        let tab_count = self.tabs.len();
        let active_tab_id = self.active_tab_id;
        let active_tab_ix = self.active_workspace_tab_ix();
        self.reveal_pending_document_tab();
        let tab_list_items = self
            .tabs
            .iter()
            .map(|tab_id| (*tab_id, self.workspace_tab_title(*tab_id)))
            .collect::<Vec<_>>();
        let tab_list_workspace = workspace.clone();
        let tab_drop_layout = self.tab_drop_layout.clone();
        {
            let mut layout = tab_drop_layout.borrow_mut();
            layout.tabs.resize(tab_count, Bounds::default());
            layout.end = Bounds::default();
        }
        let colors = ui_theme::palette(cx);
        // Large 档的 segmented 标签外框为 36px；指示层内芯由组件按档位固定为 28px，
        // 无法从外部覆写。
        let mut tabs = TabBar::new("document-tabs")
            .w_full()
            .track_scroll(&self.document_tab_scroll)
            .with_size(gpui_component::Size::Large)
            .segmented()
            .h(px(48.))
            .p(px(5.))
            .gap(px(2.))
            .rounded_none()
            .bg(ui_theme::header_material(&colors))
            .suffix(
                Button::new("document-tab-list")
                    .small()
                    .ghost()
                    .dropdown_caret(true)
                    .tooltip(crate::tr!("所有标签", "All tabs"))
                    .dropdown_menu(move |menu, window, cx| {
                        let mut menu = Self::popup_menu_with_workspace_action_context(
                            menu,
                            &tab_list_workspace,
                            cx,
                        );
                        menu = menu.scrollable(true);
                        for (tab_id, title) in &tab_list_items {
                            let tab_id = *tab_id;
                            let workspace = tab_list_workspace.clone();
                            let activate =
                                window.listener_for(&workspace, move |this, _, window, cx| {
                                    this.activate_workspace_tab(tab_id, window, cx);
                                });
                            menu = menu.item(
                                PopupMenuItem::new(title.clone())
                                    .checked(active_tab_id == tab_id)
                                    .on_click(activate),
                            );
                        }
                        menu
                    }),
            )
            .on_click(cx.listener(|this, ix: &usize, window, cx| {
                if let Some(tab_id) = this.tabs.get(*ix).copied() {
                    this.activate_workspace_tab(tab_id, window, cx);
                }
            }));
        if let Some(active_ix) = active_tab_ix {
            tabs = tabs.selected_index(active_ix);
        }

        tabs.children(self.tabs.iter().enumerate().map(|(ix, tab_id)| {
            let tab_id = *tab_id;
            let document_id = tab_id.document_id();
            let tab_title = self.workspace_tab_title(tab_id);
            let can_restore_title = document_id.is_some_and(|document_id| {
                self.documents
                    .iter()
                    .find(|tab| tab.id == document_id)
                    .is_some_and(|tab| tab.file.custom_title.is_some())
            });
            let dragged_tab = DraggedTab::new(tab_id, tab_title.clone(), source_workspace.clone());
            let tab_menu_state = TabMenuState {
                tab_ix: ix,
                tab_count,
                can_restore_title,
                has_other_window,
            };
            let context_workspace = workspace.clone();
            let tab_layout = tab_drop_layout.clone();
            let selected = self.active_tab_id == tab_id;
            let file_icon_color = if selected {
                cx.theme().tab_active_foreground
            } else {
                cx.theme().tab_foreground
            };
            let (close_button_id, context_target_id) = match tab_id {
                WorkspaceTabId::Document(id) => (
                    ElementId::from(("close-document-tab", id)),
                    ElementId::from(("document-tab-context-target", id)),
                ),
                WorkspaceTabId::New(id) => (
                    ElementId::from(("close-new-tab", id)),
                    ElementId::from(("new-tab-context-target", id)),
                ),
            };
            let close_button = Button::new(close_button_id)
                .xsmall()
                .ghost()
                .icon(IconName::Close)
                .rounded(px(7.))
                .text_color(cx.theme().muted_foreground)
                .tooltip(crate::tr_args!("关闭 {tab_title}", "Close {tab_title}"))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.request_close_workspace_tabs(BTreeSet::from([tab_id]), window, cx);
                }));
            Tab::new()
                .aria_label(tab_title.clone())
                .selected(selected)
                .when(self.cross_window_drop_ix == Some(ix), |this| {
                    this.border_l_2().border_color(cx.theme().primary)
                })
                .on_prepaint(move |bounds, _, _| {
                    if let Some(slot) = tab_layout.borrow_mut().tabs.get_mut(ix) {
                        *slot = bounds;
                    }
                })
                .on_drag(dragged_tab, |dragged, position, _, cx| {
                    cx.new(|_| dragged.clone().position(position))
                })
                .drag_over::<DraggedTab>(move |this, dragged, _, cx| {
                    if dragged.tab_id == tab_id {
                        this
                    } else {
                        this.border_l_2().border_color(cx.theme().primary)
                    }
                })
                .on_drop(cx.listener(move |this, dragged: &DraggedTab, window, cx| {
                    this.reorder_tab(dragged.tab_id, ix, window, cx);
                }))
                .on_aux_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                    if event.is_middle_click() {
                        this.request_close_workspace_tabs(BTreeSet::from([tab_id]), window, cx);
                    }
                }))
                .child(
                    div()
                        .id(context_target_id)
                        .absolute()
                        .top_0()
                        .right_0()
                        .bottom_0()
                        .left_0()
                        .context_menu(move |menu, window, _| match document_id {
                            Some(document_id) => Self::build_tab_menu(
                                menu,
                                document_id,
                                tab_menu_state,
                                context_workspace.clone(),
                                window,
                            ),
                            None => Self::build_new_tab_menu(
                                menu,
                                tab_id,
                                tab_menu_state,
                                context_workspace.clone(),
                                window,
                            ),
                        }),
                )
                .child(
                    h_flex()
                        // Large tabs own a fixed 16px inset with no per-tab override. Reduce the
                        // effective horizontal inset to 10px without changing height or type size.
                        .mx(px(-6.))
                        .gap(px(8.))
                        .items_center()
                        .text_size(px(12.))
                        .child(
                            svg()
                                .data(include_bytes!(
                                    "../../assets/icons/document-text-20-regular.svg"
                                ))
                                .size(px(20.))
                                .text_color(file_icon_color)
                                .opacity(0.72),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .line_height(relative(1.5))
                                .child(tab_title),
                        )
                        .child(close_button),
                )
        }))
        .last_empty_space(
            h_flex()
                .id("document-tab-end-drop")
                .h_full()
                .min_w_12()
                .flex_grow_1()
                .when(self.cross_window_drop_ix == Some(tab_count), |this| {
                    this.border_l_2().border_color(cx.theme().primary)
                })
                .on_prepaint({
                    let tab_drop_layout = tab_drop_layout.clone();
                    move |bounds, _, _| tab_drop_layout.borrow_mut().end = bounds
                })
                .drag_over::<DraggedTab>(|this, _, _, cx| {
                    this.border_l_2().border_color(cx.theme().primary)
                })
                .on_drop(cx.listener(move |this, dragged: &DraggedTab, window, cx| {
                    this.reorder_tab(dragged.tab_id, tab_count, window, cx);
                }))
                .child(
                    Button::new("new-workspace-tab")
                        .small()
                        .ghost()
                        .icon(IconName::Plus)
                        .tooltip(crate::tr!("新建标签页", "New tab"))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.create_new_tab(window, cx);
                        })),
                ),
        )
        .map(|tabs| {
            div()
                .id("document-tab-scroll-wheel")
                .w_full()
                .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, window, cx| {
                    this.scroll_document_tabs_from_wheel(event, window, cx);
                }))
                .child(tabs)
        })
    }

    pub(super) fn highlighted_search_suggestion(value: &str, needle: &str, cx: &App) -> StyledText {
        let normalized_value = value.to_lowercase();
        let normalized_needle = needle.to_lowercase();
        if normalized_needle.is_empty() {
            return StyledText::new(value.to_string());
        }
        let highlights = normalized_value
            .match_indices(&normalized_needle)
            .filter_map(|(start, matched)| {
                let end = start + matched.len();
                (value.is_char_boundary(start) && value.is_char_boundary(end)).then_some((
                    start..end,
                    HighlightStyle {
                        background_color: Some(ui_theme::suggestion_match_highlight(cx)),
                        ..HighlightStyle::default()
                    },
                ))
            })
            .collect::<Vec<_>>();
        StyledText::new(value.to_string()).with_highlights(highlights)
    }

    pub(super) fn render_search_suggestions(
        &self,
        suggestions: Vec<SearchSuggestion>,
        query: String,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let _performance_scope =
            crate::ui_performance::scope("Workspace::render_search_suggestions");
        let selected_ix = self.search_suggestion_ix;
        let needle = search_autocomplete_needle(&query);
        let suggestion_count = suggestions.len();
        let popup_height = rems(
            SEARCH_SUGGESTION_ROW_HEIGHT_REMS
                * suggestion_count.min(SEARCH_SUGGESTION_MAX_VISIBLE_ROWS) as f32,
        );
        let suggestions = Rc::new(suggestions);
        let workspace = cx.entity();
        let suggestions =
            uniform_list(
                "search-autocomplete-suggestions",
                suggestion_count,
                move |visible_range, _, cx| {
                    visible_range
                        .map(|ix| {
                            let suggestion = suggestions[ix].clone();
                            let selected = selected_ix == Some(ix);
                            let value = suggestion.value.clone();
                            let choose = suggestion.clone();
                            let source = match &suggestion.source {
                                SearchSuggestionSource::History => {
                                    crate::tr!("历史记录", "History").to_string()
                                }
                                SearchSuggestionSource::PredefinedFilter { name } => {
                                    crate::tr_args!(
                                        "预定义过滤器 · {name}",
                                        "Predefined filter · {name}"
                                    )
                                }
                            };
                            let workspace = workspace.clone();
                            v_flex()
                                .id(format!("search-autocomplete-suggestion:{value}"))
                                .w_full()
                                .h(rems(SEARCH_SUGGESTION_ROW_HEIGHT_REMS))
                                .justify_center()
                                .gap_1()
                                .px_3()
                                .when(selected, |row| {
                                    row.border_l_2()
                                        .border_color(cx.theme().primary)
                                        .bg(cx.theme().list_active)
                                })
                                .when(!selected, |row| {
                                    row.hover(|style| style.bg(cx.theme().tokens.list_hover))
                                })
                                .active(|row| row.bg(cx.theme().tokens.list_active))
                                .on_click(move |_, window, cx| {
                                    workspace.update(cx, |this, cx| {
                                        this.accept_search_suggestion(choose.clone(), window, cx);
                                    });
                                })
                                .child(div().w_full().min_w_0().truncate().child(
                                    Self::highlighted_search_suggestion(&value, &needle, cx),
                                ))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(source),
                                )
                                .into_any_element()
                        })
                        .collect()
                },
            )
            .absolute()
            .left_0()
            .right_0()
            .top(relative(1.))
            .mt_1()
            .h(popup_height)
            .track_scroll(&self.search_suggestion_scroll)
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().popover)
            .shadow_lg()
            .occlude()
            .with_animation(
                "search-suggestions-enter",
                Animation::new(TRANSIENT_SURFACE_ENTER_DURATION).with_easing(ease_out_cubic),
                |popup, delta| popup.opacity(delta),
            );
        deferred(suggestions)
            .with_priority(POPUP_PRIORITY)
            .into_any_element()
    }

    fn search_toolbar_button_label(
        &self,
        button: Button,
        label: impl Into<SharedString>,
    ) -> Button {
        let label = label.into();
        button.aria_label(label.clone()).child(
            div()
                .min_w_0()
                .truncate()
                .text_size(px(f32::from(self.app_settings.search_toolbar_font_size)))
                .line_height(relative(1.25))
                .child(label),
        )
    }

    pub(super) fn render_predefined_filters_popover(
        &self,
        has_document: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let _performance_scope =
            crate::ui_performance::scope("Workspace::render_predefined_filters_popover");
        let workspace = cx.entity();
        let filters = self.predefined_filters.clone();
        let query = self.query.read(cx).value().to_string();
        let saving = self.predefined_filters_saving;

        Popover::new("predefined-filters-popover")
            .w_80()
            .p_0()
            .text_sm()
            .trigger(
                Button::new("predefined-filters")
                    .small()
                    .h(px(f32::from(
                        self.app_settings.search_toolbar_control_height(),
                    )))
                    .outline()
                    .icon(IconName::BookOpen)
                    .map(|button| {
                        self.search_toolbar_button_label(button, crate::tr!("过滤器", "Filters"))
                    })
                    .dropdown_caret(true)
                    .loading(saving)
                    .disabled(!has_document)
                    .tooltip(crate::tr!(
                        "选择、组合或编辑预定义过滤器",
                        "Select, combine, or edit predefined filters"
                    )),
            )
            .content(move |_, window, popover_cx| {
                let filters = filters.clone();
                let filter_count = filters.len();
                let mut options = v_flex().w_full().gap_1().p_2().pr_4();

                if filters.is_empty() {
                    options = options.child(
                        v_flex()
                            .items_center()
                            .gap_1()
                            .px_4()
                            .py_6()
                            .text_color(popover_cx.theme().muted_foreground)
                            .child(crate::tr!("尚未配置过滤器", "No filters configured"))
                            .child(div().text_xs().child(crate::tr!(
                                "可从下方进入编辑器添加",
                                "Open the editor below to add one"
                            ))),
                    );
                } else {
                    for filter in filters {
                        let checked = query_includes_filter(&query, &filter.value);
                        let selected_filter = filter.clone();
                        let filter_value = filter.value.clone();
                        let choose_workspace = workspace.clone();
                        let choose = window.listener_for(
                            &choose_workspace,
                            move |this, checked: &bool, window, cx| {
                                this.choose_predefined_filter(
                                    selected_filter.clone(),
                                    *checked,
                                    window,
                                    cx,
                                );
                            },
                        );

                        options = options.child(
                            Checkbox::new(format!("predefined-filter-option:{}", filter.id))
                                .small()
                                .w_full()
                                .px_3()
                                .py_2()
                                .rounded(popover_cx.theme().radius)
                                .checked(checked)
                                .label(filter.name.clone())
                                .tooltip(filter_value.clone())
                                .when(checked, |option| option.bg(popover_cx.theme().list_active))
                                .when(!checked, |option| {
                                    option.hover(|style| {
                                        style.bg(popover_cx.theme().tokens.list_hover)
                                    })
                                })
                                .on_click(choose)
                                .child(
                                    h_flex()
                                        .w_full()
                                        .min_w_0()
                                        .gap_2()
                                        .child(
                                            div()
                                                .min_w_0()
                                                .flex_1()
                                                .truncate()
                                                .text_xs()
                                                .text_color(popover_cx.theme().muted_foreground)
                                                .child(filter_value),
                                        )
                                        .when(filter.use_regex, |preview| {
                                            preview.child(
                                                div()
                                                    .flex_none()
                                                    .rounded_full()
                                                    .px_2()
                                                    .py_0p5()
                                                    .bg(popover_cx.theme().primary.opacity(0.12))
                                                    .text_xs()
                                                    .text_color(popover_cx.theme().primary)
                                                    .child(".*"),
                                            )
                                        }),
                                ),
                        );
                    }
                }

                let list = options
                    .when(filter_count > 4, |list| list.h_64())
                    .when(filter_count <= 4, |list| list.max_h_64())
                    .overflow_y_scrollbar()
                    .id("predefined-filter-options-scroll");

                let edit_workspace = workspace.clone();
                let edit = popover_cx.listener(move |popover, _, window, cx| {
                    popover.dismiss(window, cx);
                    edit_workspace.update(cx, |workspace, cx| {
                        workspace.open_predefined_filters_dialog(window, cx);
                    });
                });

                v_flex().w_full().child(list).child(
                    div()
                        .w_full()
                        .border_t_1()
                        .border_color(popover_cx.theme().border)
                        .p_2()
                        .child(
                            Button::new("edit-predefined-filters")
                                .small()
                                .ghost()
                                .w_full()
                                .justify_start()
                                .icon(IconName::Settings2)
                                .label(crate::tr!("编辑预定义过滤器…", "Edit predefined filters…"))
                                .on_click(edit),
                        ),
                )
            })
            .into_any_element()
    }

    pub(super) fn render_search_scope_menu_row(
        label: &'static str,
        icon: IconName,
        selected: bool,
        cx: &mut App,
    ) -> AnyElement {
        const POPUP_MENU_ITEM_HORIZONTAL_INSET: Pixels = px(8.);
        const POPUP_MENU_ITEM_RADIUS_CAP: Pixels = px(8.);

        h_flex()
            .relative()
            .self_stretch()
            .w_full()
            .min_w_0()
            .justify_between()
            .gap_3()
            .when(selected, |row| {
                row.text_color(cx.theme().accent_foreground).child(
                    // PopupMenu wraps custom content with an 8 px horizontal inset.
                    // Expand this layer back to the owning item so its geometry is
                    // identical to the menu's native hover background.
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left(-POPUP_MENU_ITEM_HORIZONTAL_INSET)
                        .right(-POPUP_MENU_ITEM_HORIZONTAL_INSET)
                        .rounded(cx.theme().radius.min(POPUP_MENU_ITEM_RADIUS_CAP))
                        .bg(cx.theme().tokens.accent),
                )
            })
            .child(
                h_flex()
                    .relative()
                    .min_w_0()
                    .flex_1()
                    .gap_1()
                    .child(Icon::new(icon).xsmall())
                    .child(div().min_w_0().flex_1().child(label)),
            )
            .into_any_element()
    }

    pub(super) fn render_search_scope_control(
        &self,
        has_document: bool,
        tooltip: String,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let _performance_scope =
            crate::ui_performance::scope("Workspace::render_search_scope_control");
        let workspace = cx.entity();
        let menu_workspace = workspace.clone();
        let selected_scope = self.global_search.scope;
        let has_scope_settings = matches!(
            self.global_search.scope,
            SearchScope::AllOpenFiles | SearchScope::Directory
        );
        let settings_button = match self.global_search.scope {
            SearchScope::CurrentFile => None,
            SearchScope::AllOpenFiles => Some(
                Button::new("search-scope-global-settings")
                    .small()
                    .h(px(f32::from(
                        self.app_settings.search_toolbar_control_height(),
                    )))
                    .outline()
                    .rounded_l_none()
                    .border_l_0()
                    .icon(IconName::Settings2)
                    .disabled(!has_document)
                    .tooltip(crate::tr!("配置全局搜索…", "Configure global search…"))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_global_search_files_dialog(window, cx);
                    })),
            ),
            SearchScope::Directory => Some(
                Button::new("search-scope-directory-settings")
                    .small()
                    .h(px(f32::from(
                        self.app_settings.search_toolbar_control_height(),
                    )))
                    .outline()
                    .rounded_l_none()
                    .border_l_0()
                    .icon(IconName::Settings2)
                    .disabled(!has_document)
                    .tooltip(crate::tr!("配置目录搜索…", "Configure directory search…"))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_directory_search_dialog(window, cx);
                    })),
            ),
        };
        let scope_button = Button::new("search-scope")
            .small()
            .h(px(f32::from(
                self.app_settings.search_toolbar_control_height(),
            )))
            .outline()
            .dropdown_caret(true)
            .when(has_scope_settings, |button| button.rounded_r_none())
            .icon(match self.global_search.scope {
                SearchScope::CurrentFile => IconName::Search,
                SearchScope::AllOpenFiles => IconName::File,
                SearchScope::Directory => IconName::FolderOpen,
            })
            .map(|button| {
                self.search_toolbar_button_label(
                    button,
                    match self.global_search.scope {
                        SearchScope::CurrentFile => crate::tr!("当前", "Current"),
                        SearchScope::AllOpenFiles => crate::tr!("全局", "Global"),
                        SearchScope::Directory => crate::tr!("目录", "Directory"),
                    },
                )
            })
            .disabled(!has_document)
            .tooltip(tooltip)
            .dropdown_menu(move |menu, window, cx| {
                let menu =
                    Self::popup_menu_with_workspace_action_context(menu, &menu_workspace, cx);
                let current_workspace = menu_workspace.clone();
                let multi_workspace = menu_workspace.clone();
                let directory_workspace = menu_workspace.clone();

                menu.min_w(window.rem_size() * 10.)
                    .item(
                        PopupMenuItem::element(move |_, cx| {
                            Self::render_search_scope_menu_row(
                                crate::tr!("当前文件", "Current file"),
                                IconName::Search,
                                selected_scope == SearchScope::CurrentFile,
                                cx,
                            )
                        })
                        .on_click(window.listener_for(
                            &current_workspace,
                            |this, _, window, cx| {
                                this.set_search_scope(SearchScope::CurrentFile, window, cx)
                            },
                        )),
                    )
                    .item(
                        PopupMenuItem::element(move |_, cx| {
                            Self::render_search_scope_menu_row(
                                crate::tr!("全局搜索", "Global search"),
                                IconName::File,
                                selected_scope == SearchScope::AllOpenFiles,
                                cx,
                            )
                        })
                        .on_click(window.listener_for(
                            &multi_workspace,
                            |this, _, window, cx| {
                                this.set_search_scope(SearchScope::AllOpenFiles, window, cx)
                            },
                        )),
                    )
                    .item(
                        PopupMenuItem::element(move |_, cx| {
                            Self::render_search_scope_menu_row(
                                crate::tr!("目录搜索", "Directory search"),
                                IconName::FolderOpen,
                                selected_scope == SearchScope::Directory,
                                cx,
                            )
                        })
                        .on_click(window.listener_for(
                            &directory_workspace,
                            |this, _, window, cx| {
                                this.set_search_scope(SearchScope::Directory, window, cx)
                            },
                        )),
                    )
            });

        h_flex()
            .flex_none()
            .child(scope_button)
            .when_some(settings_button, |control, settings_button| {
                control.child(settings_button)
            })
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, _, window, cx| {
                    if !has_document {
                        return;
                    }

                    window.prevent_default();
                    cx.stop_propagation();
                    GlobalState::suppress_text_selection(cx);
                    match this.global_search.scope {
                        SearchScope::CurrentFile => {}
                        SearchScope::AllOpenFiles => {
                            this.open_global_search_files_dialog(window, cx);
                        }
                        SearchScope::Directory => {
                            this.open_directory_search_dialog(window, cx);
                        }
                    }
                }),
            )
            .into_any_element()
    }

    pub(super) fn render_quick_find_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let _performance_scope = crate::ui_performance::scope("Workspace::render_quick_find_bar");
        let colors = ui_theme::palette(cx);
        let query_empty = self.quick_find.query.read(cx).value().is_empty();
        let invalid_message = self.quick_find.error.as_deref();
        let target_label = match self.quick_find.target {
            Some(QuickFindTarget::Log(_)) => crate::tr!("正文", "Log"),
            Some(QuickFindTarget::Results(_)) => crate::tr!("当前结果", "Current results"),
            Some(QuickFindTarget::GlobalResults) => crate::tr!("全局结果", "Global results"),
            None => crate::tr!("当前视图", "Current view"),
        };
        let boundary_message = (!self.quick_find.no_match && invalid_message.is_none())
            .then(|| {
                self.quick_find.boundary.map(|boundary| match boundary {
                    QuickFindBoundary::Start => crate::tr!(
                        "已到达开头，没有更早的匹配项",
                        "Reached the beginning; there are no earlier matches"
                    ),
                    QuickFindBoundary::End => crate::tr!(
                        "已到达末尾，没有更多匹配项",
                        "Reached the end; there are no more matches"
                    ),
                })
            })
            .flatten();
        let status_message = invalid_message.or_else(|| {
            self.quick_find
                .no_match
                .then_some(crate::tr!("没有找到匹配项", "No matches found"))
                .or(boundary_message)
        });
        let input_label = status_message.map_or_else(
            || crate::tr_args!("在{target_label}中查找", "Find in {target_label}"),
            |message| {
                crate::tr_args!(
                    "在{target_label}中查找；{message}",
                    "Find in {target_label}; {message}"
                )
            },
        );
        let previous_tooltip = if let Some(message) = invalid_message {
            message
        } else if self.quick_find.no_match {
            crate::tr!("没有找到匹配项", "No matches found")
        } else if self.quick_find.boundary == Some(QuickFindBoundary::Start) {
            crate::tr!(
                "已到达开头，没有更早的匹配项",
                "Reached the beginning; there are no earlier matches"
            )
        } else {
            crate::tr!(
                "查找上一处（Shift+Enter / Shift+F3）",
                "Find previous (Shift+Enter / Shift+F3)"
            )
        };
        let next_tooltip = if let Some(message) = invalid_message {
            message
        } else if self.quick_find.no_match {
            crate::tr!("没有找到匹配项", "No matches found")
        } else if self.quick_find.boundary == Some(QuickFindBoundary::End) {
            crate::tr!(
                "已到达末尾，没有更多匹配项",
                "Reached the end; there are no more matches"
            )
        } else {
            crate::tr!("查找下一处（Enter / F3）", "Find next (Enter / F3)")
        };
        let controls_disabled = query_empty || self.quick_find.busy || invalid_message.is_some();
        let case_sensitive_tooltip = if self.quick_find.case_sensitive {
            crate::tr!("关闭匹配大小写", "Turn off match case")
        } else {
            crate::tr!("匹配大小写", "Match case")
        };
        let whole_word_tooltip = if self.quick_find.whole_word {
            crate::tr!("关闭全词匹配", "Turn off whole-word matching")
        } else {
            crate::tr!("全词匹配", "Whole-word matching")
        };
        let regex_tooltip = if self.quick_find.regex {
            crate::tr!("关闭正则表达式", "Turn off regular expressions")
        } else {
            crate::tr!("使用正则表达式", "Use regular expressions")
        };
        let option_icon_color = |selected| {
            if selected {
                cx.theme().primary
            } else {
                cx.theme().foreground.opacity(0.9)
            }
        };

        div()
            .id("quick-find-overlay-anchor")
            .absolute()
            .top_3()
            .left_6()
            .right_6()
            .flex()
            .justify_end()
            .child(
                h_flex()
                    .id("quick-find-bar")
                    .w_96()
                    .max_w_full()
                    .min_w_0()
                    .gap_0p5()
                    .px_1p5()
                    .py_1p5()
                    .rounded(cx.theme().radius_lg)
                    .border_1()
                    .border_color(if self.quick_find.no_match || invalid_message.is_some() {
                        cx.theme().danger
                    } else if boundary_message.is_some() {
                        cx.theme().warning
                    } else {
                        cx.theme().border
                    })
                    .bg(colors.surface)
                    .text_color(cx.theme().popover_foreground)
                    .shadow_lg()
                    .occlude()
                    .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                        match event.keystroke.key.as_str() {
                            "escape" => {
                                this.close_quick_find(window, cx);
                                cx.stop_propagation();
                            }
                            "f3" => {
                                this.start_quick_find(
                                    if event.keystroke.modifiers.shift {
                                        QuickFindDirection::Backward
                                    } else {
                                        QuickFindDirection::Forward
                                    },
                                    false,
                                    window,
                                    cx,
                                );
                                cx.stop_propagation();
                            }
                            _ => {}
                        }
                    }))
                    .child(
                        div().flex_1().min_w_0().h_8().child(
                            Input::new(&self.quick_find.query)
                                .size_full()
                                .bg(colors.control_surface)
                                .accessibility_id("A150")
                                .aria_label(input_label)
                                .prefix(Icon::new(IconName::Search))
                                .suffix(
                                    h_flex()
                                        .flex_none()
                                        .gap_0p5()
                                        .child(
                                            Button::new("quick-find-case-sensitive")
                                                .xsmall()
                                                .compact()
                                                .text()
                                                .size(px(20.))
                                                .p_0()
                                                .rounded(px(4.))
                                                .accessibility_id("A463")
                                                .selected(self.quick_find.case_sensitive)
                                                .toggled(self.quick_find.case_sensitive)
                                                .tooltip(case_sensitive_tooltip)
                                                .child(
                                                    h_flex()
                                                        .relative()
                                                        .size_full()
                                                        .justify_center()
                                                        .child(
                                                            svg()
                                                                .data(include_bytes!(
                                                                        "../../assets/icons/case-sensitive.svg"
                                                                ))
                                                                .size(px(15.))
                                                                .text_color(option_icon_color(
                                                                    self.quick_find.case_sensitive,
                                                                )),
                                                        )
                                                        .child(
                                                            div()
                                                                .absolute()
                                                                .w_0()
                                                                .h_0()
                                                                .overflow_hidden()
                                                                .opacity(0.)
                                                                .child(crate::tr!("匹配大小写", "Match case")),
                                                        ),
                                                )
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.toggle_quick_find_case_sensitive(
                                                        window, cx,
                                                    );
                                                })),
                                        )
                                        .child(
                                            Button::new("quick-find-whole-word")
                                                .xsmall()
                                                .compact()
                                                .text()
                                                .size(px(20.))
                                                .p_0()
                                                .rounded(px(4.))
                                                .accessibility_id("A464")
                                                .selected(self.quick_find.whole_word)
                                                .toggled(self.quick_find.whole_word)
                                                .tooltip(whole_word_tooltip)
                                                .child(
                                                    h_flex()
                                                        .relative()
                                                        .size_full()
                                                        .justify_center()
                                                        .child(
                                                            svg()
                                                                .data(include_bytes!(
                                                                        "../../assets/icons/whole-word.svg"
                                                                ))
                                                                .size(px(15.))
                                                                .text_color(option_icon_color(
                                                                    self.quick_find.whole_word,
                                                                )),
                                                        )
                                                        .child(
                                                            div()
                                                                .absolute()
                                                                .w_0()
                                                                .h_0()
                                                                .overflow_hidden()
                                                                .opacity(0.)
                                                                .child(crate::tr!("全词匹配", "Whole word")),
                                                        ),
                                                )
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.toggle_quick_find_whole_word(window, cx);
                                                })),
                                        )
                                        .child(
                                            Button::new("quick-find-regex")
                                                .xsmall()
                                                .compact()
                                                .text()
                                                .size(px(20.))
                                                .p_0()
                                                .rounded(px(4.))
                                                .accessibility_id("A465")
                                                .selected(self.quick_find.regex)
                                                .toggled(self.quick_find.regex)
                                                .tooltip(regex_tooltip)
                                                .child(
                                                    h_flex()
                                                        .relative()
                                                        .size_full()
                                                        .justify_center()
                                                        .child(
                                                            svg()
                                                                .data(include_bytes!(
                                                                        "../../assets/icons/regex.svg"
                                                                ))
                                                                .size(px(15.))
                                                                .text_color(option_icon_color(
                                                                    self.quick_find.regex,
                                                                )),
                                                        )
                                                        .child(
                                                            div()
                                                                .absolute()
                                                                .w_0()
                                                                .h_0()
                                                                .overflow_hidden()
                                                                .opacity(0.)
                                                                .child(crate::tr!("正则表达式", "Regular expression")),
                                                        ),
                                                )
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.toggle_quick_find_regex(window, cx);
                                                })),
                                        ),
                                ),
                        ),
                    )
                    .child(
                        Button::new("quick-find-previous")
                            .ghost()
                            .icon(IconName::ArrowUp)
                            .loading(
                                self.quick_find.busy
                                    && self.quick_find.direction
                                        == Some(QuickFindDirection::Backward),
                            )
                            .disabled(controls_disabled)
                            .tooltip(previous_tooltip)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.start_quick_find(
                                    QuickFindDirection::Backward,
                                    false,
                                    window,
                                    cx,
                                );
                            })),
                    )
                    .child(
                        Button::new("quick-find-next")
                            .ghost()
                            .icon(IconName::ArrowDown)
                            .loading(
                                self.quick_find.busy
                                    && self.quick_find.direction
                                        == Some(QuickFindDirection::Forward),
                            )
                            .disabled(controls_disabled)
                            .tooltip(next_tooltip)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.start_quick_find(
                                    QuickFindDirection::Forward,
                                    false,
                                    window,
                                    cx,
                                );
                            })),
                    )
                    .child(
                        Button::new("quick-find-close")
                            .ghost()
                            .icon(IconName::Close)
                            .tooltip(crate::tr!("关闭页内查找（Esc）", "Close find (Esc)"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.close_quick_find(window, cx);
                            })),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn render_search_bar(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let _performance_scope = crate::ui_performance::scope("Workspace::render_search_bar");
        let colors = ui_theme::palette(cx);
        let control_height = px(f32::from(self.app_settings.search_toolbar_control_height()));
        let font_size = px(f32::from(self.app_settings.search_toolbar_font_size));
        let font_scale = f32::from(self.app_settings.search_toolbar_font_size) / 13.;
        let search_history_empty = self.search_history.is_empty();
        let has_document = self.active_document().is_some();
        let predefined_filters = self.render_predefined_filters_popover(has_document, cx);
        let active_document_ready = self
            .active_document()
            .is_some_and(|tab| tab.load_state == DocumentLoadState::Ready);
        let query_value = self.query.read(cx).value().to_string();
        let query_empty = query_value.is_empty();
        let query_focused = self.query.focus_handle(cx).is_focused(window);
        let search_suggestions = self.search_autocomplete_suggestions(cx);
        let show_search_suggestions = !search_suggestions.is_empty() && query_focused;
        let search_history_open = self.search_autocomplete_mode == SearchAutocompleteMode::History;
        let global_selected_count = self.global_search.selected_documents.len();
        let search_scope_tooltip = match self.global_search.scope {
            SearchScope::CurrentFile => crate::tr!(
                "选择搜索范围（当前文件）",
                "Choose search scope (current file)"
            )
            .to_string(),
            SearchScope::AllOpenFiles => crate::tr_args!(
                "左键选择搜索范围；右键配置全局搜索（{} / {}）",
                "Left-click to choose scope; right-click to configure global search ({} / {})",
                global_selected_count,
                self.documents.len(),
            ),
            SearchScope::Directory => self
                .global_search
                .directory_options
                .directory
                .as_deref()
                .map(|directory| {
                    crate::tr_args!(
                        "左键选择搜索范围；右键配置目录搜索：{}",
                        "Left-click to choose scope; right-click to configure directory search: {}",
                        directory.display(),
                    )
                })
                .unwrap_or_else(|| {
                    crate::tr!(
                        "左键选择搜索范围；右键配置目录搜索",
                        "Left-click to choose scope; right-click to configure directory search"
                    )
                    .to_string()
                }),
        };
        let case_sensitive_variant = ButtonCustomVariant::new(cx)
            .color(cx.theme().transparent)
            .foreground(if self.case_sensitive {
                cx.theme().primary
            } else {
                cx.theme().foreground
            })
            .hover(cx.theme().muted)
            .active(cx.theme().primary.opacity(0.18));
        let regex_variant = ButtonCustomVariant::new(cx)
            .color(cx.theme().transparent)
            .foreground(if self.regex {
                cx.theme().primary
            } else {
                cx.theme().foreground
            })
            .hover(cx.theme().muted)
            .active(cx.theme().primary.opacity(0.18));
        let (result_mode_select, result_count_label, committed_results_visible) =
            match self.global_search.scope {
                SearchScope::CurrentFile => self.active_document().map_or(
                    (None, crate::tr!("0 条结果", "0 results").to_string(), false),
                    |tab| {
                        let truncation =
                            if tab.search_result.truncated && tab.result_mode.includes_matches() {
                                crate::tr!(" · 已截断", " · truncated")
                            } else {
                                ""
                            };
                        (
                            Some(tab.result_mode_select.clone()),
                            crate::tr_args!(
                                "{} 条结果{truncation}",
                                "{} results{truncation}",
                                tab.result_row_count(cx)
                            ),
                            tab.results_visible,
                        )
                    },
                ),
                SearchScope::AllOpenFiles | SearchScope::Directory => {
                    let delegate = self.global_table.read(cx).delegate();
                    let truncation = if delegate.has_truncated_results() {
                        crate::tr!(" · 已截断", " · truncated")
                    } else {
                        ""
                    };
                    (
                        Some(self.global_search.result_mode_select.clone()),
                        crate::tr_args!(
                            "{} 条 · {} 个文件{truncation}",
                            "{} results · {} files{truncation}",
                            delegate.results_count(),
                            delegate.groups_count(),
                        ),
                        self.global_search.results_visible,
                    )
                }
            };
        let search_disabled = match self.global_search.scope {
            SearchScope::CurrentFile => !active_document_ready,
            SearchScope::AllOpenFiles => {
                !has_document
                    || global_selected_count == 0
                    || self.documents.iter().any(|tab| {
                        self.global_search.selected_documents.contains(&tab.id)
                            && tab.load_state != DocumentLoadState::Ready
                    })
            }
            SearchScope::Directory => self.global_search.directory_options.directory.is_none(),
        };
        let clear_disabled = !has_document
            || (query_empty
                && match self.global_search.scope {
                    SearchScope::CurrentFile => self
                        .active_document()
                        .is_none_or(|tab| !tab.results_visible),
                    SearchScope::AllOpenFiles | SearchScope::Directory => {
                        !self.global_search.results_visible
                    }
                });
        let active_document_id = self.active_document().map(|tab| tab.id);
        let searching_current_scope = match self.global_search.scope {
            SearchScope::CurrentFile => active_document_id.is_some_and(|document_id| {
                self.searches
                    .has_target(SearchTarget::Document(document_id))
            }),
            SearchScope::AllOpenFiles => self.searches.has_target(SearchTarget::AllOpenFiles),
            SearchScope::Directory => self.searches.has_target(SearchTarget::Directory),
        };
        let search_scope_control =
            self.render_search_scope_control(has_document, search_scope_tooltip, cx);

        v_flex()
            .w_full()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .relative()
                    .w_full()
                    .min_h(control_height + SEARCH_BAR_VERTICAL_INSET * 2.)
                    .items_center()
                    .gap(px(6.))
                    .px(px(12.))
                    .py(SEARCH_BAR_VERTICAL_INSET)
                    .bg(ui_theme::header_material(&colors))
                    .child(ui_theme::glass_sheen_layer(&colors))
                    .when_some(result_mode_select, |controls, result_mode_select| {
                        controls.child(
                            div()
                                .w(px(110.) * font_scale)
                                .h(control_height)
                                .flex_none()
                                .child(
                                    Select::new(&result_mode_select)
                                        .small()
                                        .text_size(font_size)
                                        .line_height(relative(1.25))
                                        .h(control_height)
                                        .focus_ring(false),
                                ),
                        )
                    })
                    .child(
                        Button::new("case-sensitive")
                            .small()
                            .w(px(34.).max(control_height))
                            .h(control_height)
                            .p_0()
                            .rounded(px(10.))
                            .font_weight(FontWeight(700.))
                            .custom(case_sensitive_variant)
                            .map(|button| self.search_toolbar_button_label(button, "Aa"))
                            .selected(self.case_sensitive)
                            .toggled(self.case_sensitive)
                            .disabled(!has_document)
                            .tooltip(crate::tr!("区分大小写（Alt+C）", "Case-sensitive (Alt+C)"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_case_sensitive(&ToggleCaseSensitive, window, cx);
                            })),
                    )
                    .child(
                        Button::new("regular-expression")
                            .small()
                            .w(px(34.).max(control_height))
                            .h(control_height)
                            .p_0()
                            .rounded(px(10.))
                            .font_weight(FontWeight(700.))
                            .custom(regex_variant)
                            .map(|button| self.search_toolbar_button_label(button, ".*"))
                            .selected(self.regex)
                            .toggled(self.regex)
                            .disabled(!has_document)
                            .tooltip(crate::tr!("使用正则表达式", "Use regular expressions"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_regex(&ToggleRegex, window, cx);
                            })),
                    )
                    .child(predefined_filters)
                    .child(search_scope_control)
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(180.))
                            .h(control_height)
                            .relative()
                            .capture_key_down(cx.listener(
                                |this, event: &KeyDownEvent, window, cx| {
                                    if !this.query.focus_handle(cx).is_focused(window)
                                        || event.keystroke.modifiers.control
                                        || event.keystroke.modifiers.platform
                                    {
                                        return;
                                    }
                                    if this.navigate_search_autocomplete_by_key(
                                        event.keystroke.key.as_str(),
                                        cx,
                                    ) {
                                        cx.stop_propagation();
                                    }
                                },
                            ))
                            .on_scroll_wheel(cx.listener(
                                |this, event: &ScrollWheelEvent, window, cx| {
                                    let delta_y = event.delta.pixel_delta(window.line_height()).y;
                                    if delta_y == px(0.)
                                        || event.modifiers.control
                                        || event.modifiers.platform
                                    {
                                        return;
                                    }
                                    this.query.focus_handle(cx).focus(window, cx);
                                    if this.navigate_search_history_by_wheel(
                                        delta_y > px(0.),
                                        window,
                                        cx,
                                    ) {
                                        cx.stop_propagation();
                                    }
                                },
                            ))
                            .child(
                                Input::new(&self.query)
                                    .small()
                                    .text_size(font_size)
                                    .line_height(relative(1.25))
                                    .size_full()
                                    .cleanable(true)
                                    .prefix(div().child(Icon::new(IconName::Search).small()))
                                    .suffix(
                                        Button::new("search-history")
                                            .text()
                                            .icon(IconName::ChevronDown)
                                            .xsmall()
                                            .selected(search_history_open)
                                            .disabled(search_history_empty)
                                            .tooltip(if search_history_empty {
                                                crate::tr!("暂无搜索历史", "No search history")
                                            } else if search_history_open {
                                                crate::tr!("收起搜索历史", "Hide search history")
                                            } else {
                                                crate::tr!("显示搜索历史", "Show search history")
                                            })
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.toggle_search_history_popup(window, cx);
                                            })),
                                    ),
                            )
                            .when(show_search_suggestions, |input| {
                                input.child(self.render_search_suggestions(
                                    search_suggestions,
                                    query_value,
                                    cx,
                                ))
                            }),
                    )
                    .child(
                        Button::new("start-search")
                            .small()
                            .primary()
                            .icon(IconName::Search)
                            .map(|button| {
                                self.search_toolbar_button_label(
                                    button,
                                    crate::tr!("搜索", "Search"),
                                )
                            })
                            .min_w(px(88.) * font_scale)
                            .h(control_height)
                            .rounded(px(10.))
                            .loading(searching_current_scope)
                            .disabled(search_disabled)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.start_search(window, cx);
                            })),
                    )
                    .child(
                        Button::new("clear-search")
                            .small()
                            .ghost()
                            .icon(IconName::Close)
                            .w(px(34.).max(control_height))
                            .h(control_height)
                            .rounded(px(10.))
                            .disabled(clear_disabled)
                            .tooltip(crate::tr!("清除搜索结果", "Clear search results"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.clear_search(window, cx);
                            })),
                    )
                    .when(committed_results_visible, |controls| {
                        controls.child(
                            h_flex()
                                .flex_shrink_0()
                                .text_color(cx.theme().muted_foreground)
                                .child(
                                    h_flex()
                                        .min_w(px(68.))
                                        .h(control_height)
                                        .justify_center()
                                        .px(px(8.))
                                        .rounded(px(999.))
                                        .border_1()
                                        .border_color(cx.theme().primary.opacity(0.24))
                                        .bg(cx.theme().primary.opacity(0.08))
                                        .text_size(font_size)
                                        .font_weight(FontWeight(650.))
                                        .text_color(cx.theme().primary)
                                        .child(result_count_label),
                                ),
                        )
                    }),
            )
    }

    pub(super) fn render_pinned_files(
        &self,
        opening: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let _performance_scope = crate::ui_performance::scope("Workspace::render_pinned_files");
        let hidden_count = self.pinned_files.len().saturating_sub(8);
        v_flex()
            .w_full()
            .rounded(cx.theme().radius_lg * 2.)
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().group_box)
            .shadow_lg()
            .child(
                h_flex()
                    .justify_between()
                    .gap_2()
                    .h(rems(EMPTY_WORKSPACE_CARD_HEADER_HEIGHT_REMS))
                    .px_5()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .text_sm()
                            .child(crate::tr!("收藏文件", "Favorite files")),
                    )
                    .child(
                        Button::new("clear-pinned-files")
                            .xsmall()
                            .ghost()
                            .text_color(cx.theme().primary)
                            .label(crate::tr!("清空收藏", "Clear favorites"))
                            .loading(self.pinned_updating)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.clear_pinned_files(window, cx);
                            })),
                    ),
            )
            .child(
                v_flex()
                    .children(self.pinned_files.iter().take(8).map(|file| {
                        let path = file.path.clone();
                        Button::new(("pinned-file", file.id.unsigned_abs()))
                            .small()
                            .ghost()
                            .w_full()
                            .h(rems(EMPTY_WORKSPACE_FILE_ROW_HEIGHT_REMS))
                            .px_5()
                            .rounded(ButtonRounded::None)
                            .child(empty_file_button_content(
                                &file.path,
                                Some(file.last_opened_at),
                                cx,
                            ))
                            .disabled(opening)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_recent_file(path.clone(), window, cx);
                            }))
                    }))
                    .when(hidden_count > 0, |this| {
                        this.child(
                            div()
                                .px_5()
                                .py_2()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(crate::tr_args!(
                                    "另有 {hidden_count} 个收藏文件",
                                    "{hidden_count} more favorite files"
                                )),
                        )
                    }),
            )
    }

    pub(super) fn render_last_workspace_files(
        &self,
        opening: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let _performance_scope =
            crate::ui_performance::scope("Workspace::render_last_workspace_files");
        let hidden_count = self.last_workspace_files.len().saturating_sub(8);
        v_flex()
            .w_full()
            .rounded(cx.theme().radius_lg * 2.)
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().group_box)
            .shadow_lg()
            .child(
                h_flex()
                    .justify_between()
                    .gap_2()
                    .h(rems(EMPTY_WORKSPACE_CARD_HEADER_HEIGHT_REMS))
                    .px_5()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .text_sm()
                            .child(crate::tr!("上一次文件", "Previous files")),
                    )
                    .child(
                        Button::new("restore-last-workspace")
                            .xsmall()
                            .ghost()
                            .text_color(cx.theme().primary)
                            .label(crate::tr!("全部打开", "Open all"))
                            .disabled(opening)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.restore_last_workspace(window, cx);
                            })),
                    ),
            )
            .child(
                v_flex()
                    .children(self.last_workspace_files.iter().take(8).map(|file| {
                        let path = file.path.clone();
                        Button::new(("last-workspace-file", file.id.unsigned_abs()))
                            .small()
                            .ghost()
                            .w_full()
                            .h(rems(EMPTY_WORKSPACE_FILE_ROW_HEIGHT_REMS))
                            .px_5()
                            .rounded(ButtonRounded::None)
                            .child(empty_file_button_content(
                                &file.path,
                                Some(file.last_opened_at),
                                cx,
                            ))
                            .disabled(opening)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_recent_file(path.clone(), window, cx);
                            }))
                    }))
                    .when(hidden_count > 0, |this| {
                        this.child(
                            div()
                                .px_5()
                                .py_2()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(crate::tr_args!(
                                    "另有 {hidden_count} 个文件，可使用全部打开",
                                    "{hidden_count} more files; use Open all to open them"
                                )),
                        )
                    }),
            )
    }

    pub(super) fn render_recent_files(
        &self,
        opening: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let _performance_scope = crate::ui_performance::scope("Workspace::render_recent_files");
        v_flex()
            .w_full()
            .rounded(cx.theme().radius_lg * 2.)
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().group_box)
            .shadow_lg()
            .child(
                h_flex()
                    .justify_between()
                    .gap_2()
                    .h(rems(EMPTY_WORKSPACE_CARD_HEADER_HEIGHT_REMS))
                    .px_5()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .text_sm()
                            .child(crate::tr!("最近文件", "Recent files")),
                    )
                    .when(
                        !self.history_loading && !self.recent_files.is_empty(),
                        |this| {
                            this.child(
                                Button::new("open-file-history")
                                    .xsmall()
                                    .ghost()
                                    .text_color(cx.theme().primary)
                                    .label(crate::tr!("查看全部", "View all"))
                                    .loading(self.history_dialog_loading)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.open_history_dialog(window, cx);
                                    })),
                            )
                        },
                    ),
            )
            .child(
                v_flex()
                    .when(self.history_loading, |this| {
                        this.child(
                            div()
                                .px_5()
                                .py_3()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(crate::tr!("正在读取最近文件…", "Reading recent files…")),
                        )
                    })
                    .when(
                        !self.history_loading && self.recent_files.is_empty(),
                        |this| {
                            this.child(
                                div()
                                    .px_5()
                                    .py_3()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(crate::tr!("暂无最近文件", "No recent files")),
                            )
                        },
                    )
                    .children(self.recent_files.iter().map(|recent| {
                        let path = recent.path.clone();
                        Button::new(("recent-file", recent.id.unsigned_abs()))
                            .small()
                            .ghost()
                            .w_full()
                            .h(rems(EMPTY_WORKSPACE_FILE_ROW_HEIGHT_REMS))
                            .px_5()
                            .rounded(ButtonRounded::None)
                            .child(empty_file_button_content(
                                &recent.path,
                                Some(recent.last_opened_at),
                                cx,
                            ))
                            .disabled(opening)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_recent_file(path.clone(), window, cx);
                            }))
                    })),
            )
    }
}

#[cfg(test)]
mod tests {
    use gpui::{Context, Render, TestAppContext, rgb};

    use super::*;

    const CONTENT_COLOR: u32 = 0x12_34_56;
    const OVERLAY_COLOR: u32 = 0x65_43_21;

    struct DeferredOverlayHarness;

    impl Render for DeferredOverlayHarness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .child(deferred(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .w_64()
                        .h_64()
                        .bg(rgb(CONTENT_COLOR)),
                ))
                .child(deferred_workspace_overlay(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .w_32()
                        .h_32()
                        .bg(rgb(OVERLAY_COLOR)),
                ))
        }
    }

    #[gpui::test]
    fn workspace_overlay_paints_above_deferred_content(cx: &mut TestAppContext) {
        let (_, cx) = cx.add_window_view(|_, _| DeferredOverlayHarness);
        cx.update(|window, cx| window.draw(cx).clear(cx));

        let (content_order, overlay_order) = cx.update(|window, _| {
            let quads = window.painted_quads();
            let order_for = |color| {
                quads
                    .iter()
                    .find(|quad| quad.background == rgb(color).into())
                    .unwrap_or_else(|| panic!("missing {color:#x} quad in {quads:#?}"))
                    .order
            };
            (order_for(CONTENT_COLOR), order_for(OVERLAY_COLOR))
        });

        assert!(overlay_order > content_order);
    }
}
