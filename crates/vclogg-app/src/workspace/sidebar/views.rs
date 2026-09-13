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
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let placement = self.layout.sides[side.ix()].clone();
        let Some(panel) = placement.active else {
            let owner = cx.entity_id();
            return div()
                .id("empty-sidebar-drop")
                .size_full()
                .bg(cx.theme().sidebar)
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
            .w(rems(3.))
            .h_full()
            .flex_shrink_0()
            .gap_1()
            .py_1()
            .items_center()
            .bg(cx.theme().sidebar)
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
                            .w_full()
                            .h(rems(2.5))
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
                        .icon(IconName::Redo)
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
        let content = self.render_panel(panel, window, cx);
        let content = v_flex()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .h_full()
            .bg(cx.theme().sidebar)
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
        let shell = h_flex().items_stretch().size_full().min_w_0().min_h_0();
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
                                    this.color_error = None;
                                    this.colors_loaded = false;
                                    this.start_colors(cx);
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
        if panel == SidebarPanelId::Files {
            return self.render_files(cx);
        }
        if matches!(
            panel,
            SidebarPanelId::Minutes | SidebarPanelId::Colors | SidebarPanelId::Minimap
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
        if matches!(panel, SidebarPanelId::Minutes | SidebarPanelId::Minimap) {
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
            if panel == SidebarPanelId::Minimap {
                return self.render_minimap(window, cx);
            }
        }
        if panel == SidebarPanelId::Colors {
            if let Some(error) = self.color_error.clone() {
                return self.empty(panel, error, Some(panel), cx);
            }
            if self.color_job.is_some() {
                return self.empty(
                    panel,
                    crate::tr!("正在加载颜色标签…", "Loading color labels…"),
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
                SidebarPanelId::Colors => {
                    crate::tr!(
                        "当前文件未应用颜色标签",
                        "No color labels applied to this file"
                    )
                }
                _ => crate::tr!("未识别到日志时间", "No log timestamps detected"),
            };
            return self.empty(panel, message, None, cx);
        }
        let scroll = self.scrolls[&panel].clone();
        let focus = self.focus[&panel].clone();
        let preview_state = cx.weak_entity();
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
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                this.list_key(panel, event, window, cx)
            }))
            .on_prepaint(move |_, _, cx| {
                if panel == SidebarPanelId::Colors {
                    let state = preview_state.clone();
                    cx.defer(move |cx| {
                        _ = state.update(cx, |state, cx| state.request_previews(cx));
                    });
                }
            })
            .child(
                uniform_list(
                    SharedString::from(format!("sidebar-rows-{panel:?}")),
                    count,
                    cx.processor(move |this, range: Range<usize>, _, cx| {
                        if panel == SidebarPanelId::Colors {
                            this.color_visible_start = range.start;
                        }
                        range
                            .map(|ix| this.render_list_item(panel, ix, cx))
                            .collect::<Vec<_>>()
                    }),
                )
                .track_scroll(&scroll)
                .size_full(),
            )
            .vertical_scrollbar(&scroll)
            .into_any_element()
    }

    fn item_count(&self, panel: SidebarPanelId) -> usize {
        match panel {
            SidebarPanelId::Favorites => self.favorites.len(),
            SidebarPanelId::History => self.history.len(),
            SidebarPanelId::Minutes => self
                .summary
                .as_ref()
                .map_or(0, |summary| summary.groups().len()),
            SidebarPanelId::Colors => self.color_item_count(),
            _ => 0,
        }
    }

    fn item_key(&self, panel: SidebarPanelId, ix: usize) -> Option<String> {
        match panel {
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
            SidebarPanelId::Colors => self.color_item(ix).map(|(group, row)| {
                format!(
                    "{}:{}",
                    self.colors[group].id,
                    row.map_or_else(|| "header".to_string(), |row| row.to_string())
                )
            }),
            _ => None,
        }
    }

    fn item_index(&self, panel: SidebarPanelId, key: &str) -> Option<usize> {
        match panel {
            SidebarPanelId::Minutes => {
                let row = key.strip_prefix("minute-")?.parse::<usize>().ok()?;
                self.summary
                    .as_ref()?
                    .groups()
                    .binary_search_by_key(&row, |group| group.first_row())
                    .ok()
            }
            SidebarPanelId::Colors => {
                let (id, row) = key.rsplit_once(':')?;
                let mut offset = 0;
                for group in self.colors.iter() {
                    if group.id == id {
                        return if row == "header" || self.collapsed_colors.contains(id) {
                            Some(offset)
                        } else {
                            group
                                .rows
                                .position(row.parse().ok()?)
                                .map(|ix| offset + 1 + ix)
                        };
                    }
                    offset += 1 + if self.collapsed_colors.contains(&group.id) {
                        0
                    } else {
                        group.rows.len()
                    };
                }
                None
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

    fn activate_item(
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
            SidebarPanelId::Colors => {
                if let Some((group, row)) = self.color_item(ix) {
                    if let Some(row) = row {
                        self.jump(row, true, window, cx);
                    } else {
                        let id = self.colors[group].id.clone();
                        if !self.collapsed_colors.remove(&id) {
                            self.collapsed_colors.insert(id);
                        }
                        self.preview_range = None;
                        self.request_previews(cx);
                    }
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

    fn list_key(
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
        if panel == SidebarPanelId::Colors {
            self.preview_range = None;
            self.request_previews(cx);
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn render_list_item(
        &mut self,
        panel: SidebarPanelId,
        ix: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(key) = self.item_key(panel, ix) else {
            return div().into_any_element();
        };
        let selected = self.selected.get(&panel) == Some(&key);
        let (title, detail, color) = match panel {
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
                    None,
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
                    None,
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
                    None,
                )
            }
            SidebarPanelId::Colors => {
                let Some((group_ix, row)) = self.color_item(ix) else {
                    return div().into_any_element();
                };
                let group = &self.colors[group_ix];
                match row {
                    Some(row) => (
                        format!("{}", row + 1),
                        self.previews
                            .get(&row)
                            .cloned()
                            .unwrap_or_else(|| crate::tr!("加载中…", "Loading…").to_string()),
                        Some(group.paint_color()),
                    ),
                    None => (
                        format!(
                            "{} {}",
                            if self.collapsed_colors.contains(&group.id) {
                                "▸"
                            } else {
                                "▾"
                            },
                            group.label
                        ),
                        crate::tr_args!("{} 行", "{} lines", group.rows.len()),
                        Some(group.paint_color()),
                    ),
                }
            }
            _ => return div().into_any_element(),
        };
        let tooltip = detail.clone();
        let generation = self.generation;
        let item_key = key.clone();
        Button::new(SharedString::from(format!("sidebar-item-{panel:?}-{key}")))
            .ghost()
            .w_full()
            .h(rems(3.25))
            .selected(selected)
            .justify_start()
            .tooltip(tooltip)
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .gap_2()
                    .when_some(color, |this, color| {
                        this.child(div().size_2().flex_shrink_0().bg(color))
                    })
                    .child(
                        v_flex()
                            .min_w_0()
                            .flex_1()
                            .items_start()
                            .child(div().w_full().truncate().text_sm().child(title))
                            .child(
                                div()
                                    .w_full()
                                    .truncate()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(detail),
                            ),
                    ),
            )
            .on_click(cx.listener(move |this, _, window, cx| {
                if matches!(panel, SidebarPanelId::Minutes | SidebarPanelId::Colors)
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
        if self.root.is_none() {
            return self.empty(
                SidebarPanelId::Files,
                crate::tr!(
                    "打开日志文件后查看所在文件夹",
                    "Open a log file to browse its folder"
                ),
                None,
                cx,
            );
        }
        if let Some(DirectoryLoad::Failed(error)) = self
            .root
            .as_ref()
            .and_then(|root| self.directories.get(root))
        {
            return self.empty(
                SidebarPanelId::Files,
                error.clone(),
                Some(SidebarPanelId::Files),
                cx,
            );
        }
        if self.tree_dirty {
            return self.empty(
                SidebarPanelId::Files,
                crate::tr!("正在加载文件夹…", "Loading folder…"),
                None,
                cx,
            );
        }
        let state = cx.entity();
        let tree = Tree::new(&self.tree, move |_, entry, selected, _, _| {
            let path = decode_persisted_path(entry.item().id.as_ref());
            let folder = entry.is_folder();
            let state = state.clone();
            ListItem::new(entry.item().id.clone())
                .selected(selected)
                .w_full()
                .h(rems(2.))
                .px_2()
                .pl(rems(0.5 + entry.depth() as f32))
                .child(
                    h_flex()
                        .min_w_0()
                        .gap_1()
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
                            .small(),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_sm()
                                .child(entry.item().label.clone()),
                        ),
                )
                .on_click(move |event, window, cx| {
                    if !folder && event.click_count() >= 2 {
                        state.update(cx, |state, cx| state.open_path(path.clone(), window, cx));
                    }
                })
        });
        div()
            .id("sidebar-file-tree")
            .track_focus(&self.focus[&SidebarPanelId::Files])
            .flex_1()
            .min_h_0()
            .min_w_0()
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
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
            .child(tree)
            .into_any_element()
    }
}

fn format_opened_at(timestamp: i64) -> String {
    DateTime::from_timestamp(timestamp, 0)
        .map(|time| time.with_timezone(&Local).format("%m-%d %H:%M").to_string())
        .unwrap_or_default()
}
