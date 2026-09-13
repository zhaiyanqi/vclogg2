use super::*;

impl ColorGroup {
    pub(super) fn paint_color(&self) -> Hsla {
        color_with_alpha(self.color, self.alpha)
    }
}

impl SidebarState {
    pub(super) fn render_side(
        &mut self,
        side: SidebarSide,
        tabs: Option<AnyElement>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let background = ui_theme::header_material(&ui_theme::palette(cx));
        let placement = self.layout.sides[side.ix()].clone();
        let Some(panel) = placement.active else {
            let owner = cx.entity_id();
            return div()
                .id("empty-sidebar-drop")
                .size_full()
                .bg(background)
                .drag_over::<DraggedSidebarPanel>(move |this, drag, _, cx| {
                    if drag.owner == owner {
                        this.bg(cx.theme().accent)
                    } else {
                        this
                    }
                })
                .on_drop(cx.listener(move |this, drag: &DraggedSidebarPanel, _, cx| {
                    this.drop_panel(drag, side, None, cx)
                }))
                .child(
                    div()
                        .p_3()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(crate::tr!(
                            "将侧边栏功能拖到此处",
                            "Drag a sidebar tool here"
                        )),
                )
                .into_any_element();
        };
        let owner = cx.entity_id();
        let rail = v_flex()
            .w(rems(SIDEBAR_RAIL_WIDTH_REM))
            .h_full()
            .flex_shrink_0()
            .gap_1()
            .py_1()
            .items_center()
            .children(placement.panels.iter().copied().map(|id| {
                let drag = DraggedSidebarPanel { owner, panel: id };
                div()
                    .id(SharedString::from(format!("sidebar-tab-{id:?}")))
                    .w_full()
                    .px_1()
                    .on_drag(drag, |drag, _, _, cx| cx.new(|_| drag.clone()))
                    .drag_over::<DraggedSidebarPanel>(move |this, drag, _, cx| {
                        if drag.owner == owner && drag.panel != id {
                            this.border_t_2().border_color(cx.theme().primary)
                        } else {
                            this
                        }
                    })
                    .on_drop(cx.listener(move |this, drag: &DraggedSidebarPanel, _, cx| {
                        this.drop_panel(drag, side, Some(id), cx);
                        cx.stop_propagation();
                    }))
                    .child(
                        Button::new(SharedString::from(format!("sidebar-tab-button-{id:?}")))
                            .ghost()
                            .small()
                            .w_full()
                            .h_7()
                            .p_0()
                            .child(id.icon())
                            .selected(id == panel)
                            .tooltip(id.title())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.activate(id, side, window, cx)
                            })),
                    )
                    .context_menu({
                        let state = cx.entity();
                        move |menu, window, cx| {
                            Self::panel_menu(menu, state.clone(), id, side, window, cx)
                        }
                    })
            }))
            .child(
                div()
                    .id("sidebar-rail-end")
                    .flex_1()
                    .w_full()
                    .drag_over::<DraggedSidebarPanel>(move |this, drag, _, cx| {
                        if drag.owner == owner {
                            this.border_t_2().border_color(cx.theme().primary)
                        } else {
                            this
                        }
                    })
                    .on_drop(cx.listener(move |this, drag: &DraggedSidebarPanel, _, cx| {
                        this.drop_panel(drag, side, None, cx)
                    })),
            );
        let menu_state = cx.entity();
        let mut header = h_flex()
            .h(ui_theme::WORKSPACE_BAR_HEIGHT)
            .flex_shrink_0()
            .gap_1()
            .px_2()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_semibold()
                    .child(panel.title()),
            );
        if panel == SidebarPanelId::Files {
            header = header
                .child(
                    Button::new("sidebar-files-refresh")
                        .xsmall()
                        .ghost()
                        .icon(crate::app_assets::AppIcon::Refresh)
                        .tooltip(crate::tr!("刷新文件夹", "Refresh folders"))
                        .on_click(cx.listener(|this, _, _, cx| this.refresh_tree(cx))),
                )
                .child(
                    Button::new("sidebar-files-collapse")
                        .xsmall()
                        .ghost()
                        .icon(IconName::ChevronUp)
                        .tooltip(crate::tr!("折叠文件夹", "Collapse folders"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.tree_reveal_active = false;
                            this.expanded.clear();
                            this.rebuild_tree(cx);
                            cx.notify();
                        })),
                );
        }
        header = header.child(
            Button::new("sidebar-panel-menu")
                .xsmall()
                .ghost()
                .icon(IconName::Ellipsis)
                .tooltip(crate::tr!("侧边栏操作", "Sidebar actions"))
                .dropdown_menu(move |menu, window, cx| {
                    Self::panel_menu(menu, menu_state.clone(), panel, side, window, cx)
                }),
        );
        let content = tabs.unwrap_or_else(|| self.render_panel(panel, window, cx));
        let content = v_flex()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .h_full()
            .border_color(cx.theme().border)
            .when(side == SidebarSide::Left, |this| this.border_l_1())
            .when(side == SidebarSide::Right, |this| this.border_r_1())
            .child(header)
            .when_some(self.persistence_error.clone(), |this, error| {
                this.child(
                    div()
                        .px_2()
                        .text_xs()
                        .text_color(cx.theme().danger)
                        .child(error),
                )
            })
            .child(content)
            .id("sidebar-content-drop")
            .drag_over::<DraggedSidebarPanel>(move |this, drag, _, cx| {
                if drag.owner == owner {
                    this.bg(cx.theme().accent)
                } else {
                    this
                }
            })
            .on_drop(cx.listener(move |this, drag: &DraggedSidebarPanel, _, cx| {
                this.drop_panel(drag, side, None, cx)
            }));
        // Paint the same window material once for both the rail and panel.
        let shell = h_flex()
            .items_stretch()
            .size_full()
            .min_w_0()
            .min_h_0()
            .bg(background);
        if side == SidebarSide::Left {
            shell.child(rail).child(content).into_any_element()
        } else {
            shell.child(content).child(rail).into_any_element()
        }
    }

    fn panel_menu(
        mut menu: PopupMenu,
        state: Entity<Self>,
        panel: SidebarPanelId,
        side: SidebarSide,
        window: &mut Window,
        cx: &mut Context<PopupMenu>,
    ) -> PopupMenu {
        let panels = state.read(cx).layout.sides[side.ix()].panels.clone();
        let ix = panels.iter().position(|id| *id == panel).unwrap_or(0);
        let target = side.other();
        let moved = state.clone();
        menu = menu.item(
            PopupMenuItem::new(if target == SidebarSide::Left {
                crate::tr!("移到左侧", "Move to left")
            } else {
                crate::tr!("移到右侧", "Move to right")
            })
            .on_click(window.listener_for(&moved, move |this, _, window, cx| {
                this.layout.move_panel(panel, target, None);
                if panel == SidebarPanelId::History {
                    this.refresh_history(cx);
                }
                this.changed_layout(cx);
                this.focus_panel(panel, window, cx);
            })),
        );
        for (up, label) in [
            (true, crate::tr!("上移", "Move up")),
            (false, crate::tr!("下移", "Move down")),
        ] {
            let before = if up {
                ix.checked_sub(1).and_then(|ix| panels.get(ix)).copied()
            } else {
                panels.get(ix + 2).copied()
            };
            let disabled = if up { ix == 0 } else { ix + 1 >= panels.len() };
            menu = menu.item(PopupMenuItem::new(label).disabled(disabled).on_click(
                window.listener_for(&state, move |this, _, window, cx| {
                    this.layout.move_panel(panel, side, before);
                    this.changed_layout(cx);
                    this.focus_panel(panel, window, cx);
                }),
            ));
        }
        if matches!(panel, SidebarPanelId::History | SidebarPanelId::Favorites) {
            let workspace = state.read(cx).workspace.clone();
            menu = menu.separator().item(
                PopupMenuItem::new(crate::tr!("管理历史记录…", "Manage history…")).on_click(
                    move |_, window, cx| {
                        _ = workspace.update(cx, |workspace, cx| {
                            workspace.open_history_dialog(window, cx)
                        });
                    },
                ),
            );
            if panel == SidebarPanelId::History {
                menu = menu.item(
                    PopupMenuItem::new(crate::tr!("刷新历史记录", "Refresh history")).on_click(
                        window.listener_for(&state, |this, _, _, cx| this.refresh_history(cx)),
                    ),
                );
            }
        }
        if state.read(cx).persistence_error.is_some() {
            menu = menu.item(
                PopupMenuItem::new(crate::tr!("重新保存布局", "Retry saving layout")).on_click(
                    window.listener_for(&state, |this, _, _, cx| this.changed_layout(cx)),
                ),
            );
        }
        menu.separator()
            .item(
                PopupMenuItem::new(crate::tr!("关闭侧边栏", "Close sidebar")).on_click(
                    window.listener_for(&state, move |this, _, window, cx| {
                        this.toggle(side, window, cx)
                    }),
                ),
            )
            .item(
                PopupMenuItem::new(crate::tr!("恢复默认布局", "Reset layout")).on_click(
                    window.listener_for(&state, |this, _, window, cx| {
                        this.layout = SidebarLayout::default();
                        this.changed_layout(cx);
                        _ = this.workspace.update(cx, |workspace, cx| {
                            workspace.log_viewer.focus_handle.focus(window, cx);
                        });
                    }),
                ),
            )
    }

    pub(super) fn empty(
        &self,
        panel: SidebarPanelId,
        message: impl Into<SharedString>,
        retry: Option<SidebarPanelId>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        v_flex()
            .id("sidebar-empty-state")
            .track_focus(&self.focus[&panel])
            .tab_index(0)
            .flex_1()
            .min_h_0()
            .p_3()
            .gap_2()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(message.into())
            .when_some(retry, |this, panel| {
                this.child(
                    Button::new("sidebar-retry")
                        .small()
                        .label(crate::tr!("重试", "Retry"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            match panel {
                                SidebarPanelId::History => this.refresh_history(cx),
                                SidebarPanelId::Files => this.refresh_tree(cx),
                                SidebarPanelId::Colors => {
                                    this.retry_colors(cx);
                                }
                                _ => {
                                    this.summary_error = None;
                                    this.start_summary(cx);
                                }
                            }
                            cx.notify();
                        })),
                )
            })
            .into_any_element()
    }

    fn render_panel(
        &mut self,
        panel: SidebarPanelId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if panel == SidebarPanelId::LogColoring {
            return self.render_log_coloring(cx);
        }
        if panel == SidebarPanelId::Files {
            return self.render_files(cx);
        }
        if matches!(
            panel,
            SidebarPanelId::Minutes
                | SidebarPanelId::Colors
                | SidebarPanelId::Minimap
                | SidebarPanelId::Marks
        ) {
            let Some((_, document)) = &self.document else {
                return self.empty(
                    panel,
                    crate::tr!("打开日志文件后查看", "Open a log file to inspect it"),
                    None,
                    cx,
                );
            };
            if !document.has_complete_line_index() {
                return self.empty(
                    panel,
                    crate::tr!("正在建立文件索引…", "Building the file index…"),
                    None,
                    cx,
                );
            }
        }
        if matches!(panel, SidebarPanelId::Minimap | SidebarPanelId::Colors) {
            return self.render_overview(panel, window, cx);
        }
        if panel == SidebarPanelId::Minutes {
            if let Some(error) = self.summary_error.clone() {
                return self.empty(panel, error, Some(panel), cx);
            }
            if self.summary_job.is_some() || self.summary.is_none() {
                let scanned = self.progress.load(Ordering::Relaxed);
                let total = self
                    .document
                    .as_ref()
                    .map_or(0, |(_, document)| document.source_line_count());
                return self.empty(
                    panel,
                    crate::tr_args!(
                        "正在建立导航索引：{scanned}/{total}",
                        "Indexing navigation: {scanned}/{total}"
                    ),
                    None,
                    cx,
                );
            }
        }
        if panel == SidebarPanelId::History {
            if let Some(error) = self.history_error.clone() {
                return self.empty(panel, error, Some(panel), cx);
            }
            if self.history_task.is_some() && self.history.is_empty() {
                return self.empty(panel, crate::tr!("加载中…", "Loading…"), None, cx);
            }
        }
        let count = self.item_count(panel);
        if count == 0 {
            let message = match panel {
                SidebarPanelId::Favorites => crate::tr!(
                    "尚无收藏，使用工具栏星标收藏当前文件",
                    "No favorites. Use the toolbar star to favorite the current file"
                ),
                SidebarPanelId::History => crate::tr!("暂无历史记录", "No history yet"),
                SidebarPanelId::Marks => crate::tr!(
                    "暂无标记，可在日志行添加行标记或文字标记",
                    "No marks. Add a row mark or text mark to a log line"
                ),
                _ => crate::tr!("未识别到日志时间", "No log timestamps detected"),
            };
            return self.empty(panel, message, None, cx);
        }
        let scroll = self.scrolls[&panel].clone();
        let focus = self.focus[&panel].clone();
        div()
            .id(SharedString::from(format!("sidebar-list-{panel:?}")))
            .flex_1()
            .min_h_0()
            .min_w_0()
            .relative()
            .track_focus(&focus)
            .tab_index(0)
            .border_1()
            .border_color(cx.theme().transparent)
            .focus_visible(|style| style.border_color(cx.theme().ring))
            .capture_any_mouse_down(|event, window, cx| {
                // Suppress default focus transfer for rows and blank space in every shared list.
                if event.button != MouseButton::Left {
                    window.prevent_default();
                    cx.stop_propagation();
                }
            })
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                this.list_key(panel, event, window, cx)
            }))
            .child(
                uniform_list(
                    SharedString::from(format!("sidebar-rows-{panel:?}")),
                    count,
                    cx.processor(move |this, range: Range<usize>, window, cx| {
                        if panel == SidebarPanelId::Marks {
                            this.defer_mark_previews(range.clone(), window, cx);
                        }
                        range
                            .map(|ix| this.render_list_item(panel, ix, cx))
                            .collect::<Vec<_>>()
                    }),
                )
                .track_scroll(&scroll)
                // Keep each edge of a focused row inside UniformList's content mask.
                .p_1()
                .size_full(),
            )
            .vertical_scrollbar(&scroll)
            .into_any_element()
    }

    fn item_count(&self, panel: SidebarPanelId) -> usize {
        match panel {
            SidebarPanelId::Marks => self.marks.rows.len(),
            SidebarPanelId::LogColoring => self.log_coloring.groups.len(),
            SidebarPanelId::Favorites => self.favorites.len(),
            SidebarPanelId::History => self.history.len(),
            SidebarPanelId::Minutes => self
                .summary
                .as_ref()
                .map_or(0, |summary| summary.groups().len()),
            _ => 0,
        }
    }

    fn item_key(&self, panel: SidebarPanelId, ix: usize) -> Option<String> {
        match panel {
            SidebarPanelId::Marks => self.mark_key(ix),
            SidebarPanelId::LogColoring => self
                .log_coloring
                .groups
                .get(ix)
                .map(|group| group.id.clone()),
            SidebarPanelId::Favorites => self
                .favorites
                .get(ix)
                .map(|file| encode_persisted_path(&file.path)),
            SidebarPanelId::History => self
                .history
                .get(ix)
                .map(|file| format!("history-{}", file.id)),
            SidebarPanelId::Minutes => self
                .summary
                .as_ref()?
                .groups()
                .get(ix)
                .map(|group| format!("minute-{}", group.first_row())),
            _ => None,
        }
    }

    fn item_index(&self, panel: SidebarPanelId, key: &str) -> Option<usize> {
        match panel {
            SidebarPanelId::Marks => self.mark_index(key),
            SidebarPanelId::LogColoring => self
                .log_coloring
                .groups
                .iter()
                .position(|group| group.id == key),
            SidebarPanelId::Minutes => {
                let row = key.strip_prefix("minute-")?.parse::<usize>().ok()?;
                self.summary
                    .as_ref()?
                    .groups()
                    .binary_search_by_key(&row, |group| group.first_row())
                    .ok()
            }
            SidebarPanelId::History => {
                let id = key.strip_prefix("history-")?.parse::<i64>().ok()?;
                self.history.iter().position(|file| file.id == id)
            }
            SidebarPanelId::Favorites => {
                let path = decode_persisted_path(key);
                self.favorites
                    .iter()
                    .position(|file| paths_match(&file.path, &path))
            }
            _ => None,
        }
    }

    pub(super) fn activate_item(
        &mut self,
        panel: SidebarPanelId,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(key) = self.item_key(panel, ix) {
            self.selected.insert(panel, key);
        }
        match panel {
            SidebarPanelId::Marks => self.activate_mark(ix, window, cx),
            SidebarPanelId::LogColoring => {
                if let Some(group) = self.log_coloring.groups.get(ix) {
                    let id = group.id.clone();
                    _ = self.workspace.update(cx, |workspace, cx| {
                        workspace.select_log_coloring_group(id, window, cx)
                    });
                }
            }

            SidebarPanelId::Favorites | SidebarPanelId::History => {
                let path = if panel == SidebarPanelId::Favorites {
                    self.favorites.get(ix).map(|file| file.path.clone())
                } else {
                    self.history.get(ix).map(|file| file.path.clone())
                };
                if let Some(path) = path {
                    self.open_path(path, window, cx);
                }
            }
            SidebarPanelId::Minutes => {
                if let Some(group) = self
                    .summary
                    .as_ref()
                    .and_then(|summary| summary.groups().get(ix))
                {
                    self.jump(group.first_row(), true, window, cx);
                }
            }
            _ => {}
        }
        cx.notify();
    }

    fn open_path(&self, path: PathBuf, window: &mut Window, cx: &mut App) {
        _ = self.workspace.update(cx, |workspace, cx| {
            workspace.open_recent_file(path, window, cx)
        });
    }

    pub(super) fn list_key(
        &mut self,
        panel: SidebarPanelId,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let count = self.item_count(panel);
        if count == 0 {
            return;
        }
        let current = self
            .selected
            .get(&panel)
            .and_then(|key| self.item_index(panel, key))
            .unwrap_or(0);
        let next = match event.keystroke.key.as_str() {
            "up" => current.saturating_sub(1),
            "down" => (current + 1).min(count - 1),
            "home" => 0,
            "end" => count - 1,
            "pageup" => current.saturating_sub(10),
            "pagedown" => (current + 10).min(count - 1),
            "enter" => {
                self.activate_item(panel, current, window, cx);
                cx.stop_propagation();
                return;
            }
            _ => return,
        };
        if let Some(key) = self.item_key(panel, next) {
            self.selected.insert(panel, key);
        }
        self.scrolls[&panel].scroll_to_item(next, ScrollStrategy::Nearest);
        cx.stop_propagation();
        cx.notify();
    }

    fn render_list_item(
        &mut self,
        panel: SidebarPanelId,
        ix: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if panel == SidebarPanelId::Marks {
            return self.render_mark(ix, cx);
        }
        let Some(key) = self.item_key(panel, ix) else {
            return div().into_any_element();
        };
        let selected = self.selected.get(&panel) == Some(&key);
        let (title, detail) = match panel {
            SidebarPanelId::Favorites => {
                let Some(file) = self.favorites.get(ix) else {
                    return div().into_any_element();
                };
                (
                    file.path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                    file.path.display().to_string(),
                )
            }
            SidebarPanelId::History => {
                let Some(file) = self.history.get(ix) else {
                    return div().into_any_element();
                };
                (
                    file.path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                    format!(
                        "{} · {}",
                        format_opened_at(file.last_opened_at),
                        file.path.display()
                    ),
                )
            }
            SidebarPanelId::Minutes => {
                let Some(group) = self
                    .summary
                    .as_ref()
                    .and_then(|summary| summary.groups().get(ix))
                else {
                    return div().into_any_element();
                };
                (
                    group.label(),
                    crate::tr_args!(
                        "{} 条 · 第 {} 行",
                        "{} entries · line {}",
                        group.count(),
                        group.first_row() + 1
                    ),
                )
            }
            _ => return div().into_any_element(),
        };
        let generation = self.generation;
        let item_key = key.clone();
        Button::new(SharedString::from(format!("sidebar-item-{panel:?}-{key}")))
            .ghost()
            .w_full()
            .h(rems(3.25))
            .py_1()
            .selected(selected)
            .justify_start()
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .gap_2()
                    .child(
                        v_flex()
                            .min_w_0()
                            .flex_1()
                            .items_start()
                            .gap_0p5()
                            // Keep both text lines within the fixed virtual-list row,
                            // independent of inherited line height and flex shrinking.
                            .child(
                                div()
                                    .w_full()
                                    .h(rems(1.25))
                                    .flex_shrink_0()
                                    .text_sm()
                                    .line_height(rems(1.25))
                                    .truncate()
                                    .child(title),
                            )
                            .child(
                                div()
                                    .w_full()
                                    .h(rems(1.125))
                                    .flex_shrink_0()
                                    .text_xs()
                                    .line_height(rems(1.125))
                                    .truncate()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(detail),
                            ),
                    ),
            )
            .on_click(cx.listener(move |this, event, window, cx| {
                if matches!(event, ClickEvent::Mouse(event) if event.down.button != MouseButton::Left || event.up.button != MouseButton::Left) {
                    return;
                }
                if panel == SidebarPanelId::Minutes
                    && this.generation != generation
                {
                    return;
                }
                if let Some(ix) = this.item_index(panel, &item_key) {
                    this.activate_item(panel, ix, window, cx);
                }
            }))
            .into_any_element()
    }

    fn render_files(&mut self, cx: &mut Context<Self>) -> AnyElement {
        use file_tree::{TREE_GAP_REM, TREE_ICON_REM, TREE_PADDING_REM, TREE_TEXT_REM, tree_label};

        let menu_state = cx.entity();
        let state = cx.entity();
        let scroll = self.tree.read(cx).scroll_handle().clone();
        let tree = Tree::new(&self.tree, move |_, entry, selected, _, _| {
            let path = decode_persisted_path(entry.item().id.as_ref());
            let folder = entry.is_folder();
            let disabled = entry.is_disabled();
            let state = state.clone();
            ListItem::new(entry.item().id.clone())
                .selected(selected)
                .w_full()
                .h(rems(2.))
                .px(rems(TREE_PADDING_REM))
                .pl(rems(TREE_PADDING_REM + entry.depth() as f32))
                .child(
                    h_flex()
                        .flex_shrink_0()
                        .gap(rems(TREE_GAP_REM))
                        .child(
                            Icon::new(if folder {
                                if entry.is_expanded() {
                                    IconName::ChevronDown
                                } else {
                                    IconName::ChevronRight
                                }
                            } else {
                                IconName::File
                            })
                            .size(rems(TREE_ICON_REM)),
                        )
                        .child(
                            div()
                                .flex_shrink_0()
                                .whitespace_nowrap()
                                .text_size(rems(TREE_TEXT_REM))
                                .child(tree_label(&entry.item().label)),
                        ),
                )
                .on_click(move |event, window, cx| {
                    if disabled {
                        return;
                    }
                    state.update(cx, |state, cx| {
                        state.tree_reveal_active = false;
                        state.tree.update(cx, |tree, cx| tree.focus(window, cx));
                        if !folder && event.click_count() >= 2 {
                            state.open_path(path.clone(), window, cx);
                        }
                    });
                })
        })
        .context_menu(move |_, entry, menu, window, cx| {
            if entry.is_disabled() {
                return menu;
            }
            let path = decode_persisted_path(entry.item().id.as_ref());
            let folder = entry.is_folder();
            let open_path = path.clone();
            let mut menu = menu.item(
                PopupMenuItem::new(crate::tr!("打开所在位置", "Show in folder")).on_click(
                    window.listener_for(&menu_state, move |state, _, window, cx| {
                        _ = state.workspace.update(cx, |workspace, cx| {
                            let command = &workspace.app_settings.open_directory_command;
                            let result = if folder {
                                crate::open_directory::launch_custom_directory(command, &open_path)
                            } else {
                                crate::open_directory::launch_custom(command, &open_path)
                            };
                            match result {
                                Ok(true) => {}
                                Ok(false) => {
                                    let directory = if folder {
                                        Some(open_path.as_path())
                                    } else {
                                        open_path.parent()
                                    };
                                    if let Some(directory) = directory {
                                        cx.open_with_system(directory);
                                    }
                                }
                                Err(error) => window.notify_message(error.to_string(), cx),
                            }
                        });
                    }),
                ),
            );
            if folder
                && matches!(
                    menu_state.read(cx).directories.get(&path),
                    Some(DirectoryLoad::Failed(_))
                )
            {
                menu = menu.item(PopupMenuItem::new(crate::tr!("重试", "Retry")).on_click(
                    window.listener_for(&menu_state, move |state, _, _, cx| {
                        state.retry_tree_directory(path.clone(), cx)
                    }),
                ));
            }
            menu
        });
        div()
            .id("sidebar-file-tree")
            .track_focus(&self.focus[&SidebarPanelId::Files])
            .flex_1()
            .min_h_0()
            .min_w_0()
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if matches!(
                    event.keystroke.key.as_str(),
                    "up" | "down" | "left" | "right" | "enter"
                ) {
                    this.tree_reveal_active = false;
                }
                if event.keystroke.key == "enter" {
                    let path = this
                        .tree
                        .read(cx)
                        .selected_entry()
                        .filter(|entry| !entry.is_folder() && !entry.is_disabled())
                        .map(|entry| decode_persisted_path(entry.item().id.as_ref()));
                    if let Some(path) = path {
                        this.open_path(path, window, cx);
                        cx.stop_propagation();
                    }
                }
            }))
            .v_flex()
            .when(
                self.roots.is_empty() && self.roots_error.is_none(),
                |this| {
                    this.child(
                        div()
                            .px_2()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(crate::tr!("正在加载设备…", "Loading device…")),
                    )
                },
            )
            .when_some(
                self.tree_error.clone().or_else(|| self.roots_error.clone()),
                |this, error| {
                    this.child(
                        h_flex()
                            .px_2()
                            .py_1()
                            .flex_none()
                            .gap_1()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(error),
                            )
                            .child(
                                Button::new("sidebar-tree-error-retry")
                                    .small()
                                    .ghost()
                                    .label(crate::tr!("重试", "Retry"))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.tree_reveal_active = this.tree_target.is_some();
                                        this.refresh_tree(cx);
                                    })),
                            ),
                    )
                },
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .child(tree)
                    .horizontal_scrollbar(&scroll),
            )
            .into_any_element()
    }
}

fn format_opened_at(timestamp: i64) -> String {
    DateTime::from_timestamp(timestamp, 0)
        .map(|time| time.with_timezone(&Local).format("%m-%d %H:%M").to_string())
        .unwrap_or_default()
}
