use super::search_tabs::{SearchTabId, SearchTabOwner, SearchTabState};
use super::*;
use crate::search_context::{PersistedSearchTab, SearchTabQuery};
use gpui_base::{Tab as SearchResultTab, Tabs as SearchResultTabs};

#[derive(Clone)]
struct DraggedSearchTab {
    owner: SearchTabOwner,
    id: SearchTabId,
    workspace: WeakEntity<Workspace>,
    title: SharedString,
}

impl Render for DraggedSearchTab {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_1()
            .bg(cx.theme().popover)
            .text_color(cx.theme().popover_foreground)
            .child(self.title.clone())
    }
}

impl Workspace {
    pub(super) fn render_search_tabs(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(owner) = self.search_tab_owner() else {
            return div().into_any_element();
        };
        let Some(group) = self.search_tabs.groups.get(&owner) else {
            return div().into_any_element();
        };
        let workspace = cx.entity();
        let count = group.tabs.len();
        let track = SearchResultTabs::new(format!("search-tab-track-{owner:?}"))
            .flex()
            .items_start()
            .min_w_0()
            .flex_1()
            .h_full()
            .overflow_x_scroll()
            .track_scroll(&self.search_tabs.scroll)
            .children(group.tabs.iter().map(|state| {
                let id = SearchTabId(state.saved.id);
                let title = state.title();
                let menu_workspace = workspace.clone();
                let dragged = DraggedSearchTab {
                    owner,
                    id,
                    workspace: workspace.downgrade(),
                    title: title.clone(),
                };
                let selected = group.active == id;
                // Bottom tabs join the results above: the active tab has no top border.
                SearchResultTab::new(format!("search-tab-{owner:?}-{}", id.0))
                    .selected(selected)
                    .relative()
                    .flex()
                    .items_center()
                    .flex_shrink_0()
                    .h_full()
                    .px_2()
                    .text_sm()
                    .border_t_0()
                    .border_color(cx.theme().border)
                    .text_color(cx.theme().tab_foreground)
                    .when(selected, |tab| {
                        tab.border_l_1()
                            .border_r_1()
                            .border_b_1()
                            .rounded_b(cx.theme().radius)
                            .bg(cx.theme().tab_active)
                            .text_color(cx.theme().tab_active_foreground)
                    })
                    .hover(|tab| tab.text_color(cx.theme().tab_active_foreground))
                    .child(
                        div()
                            .id(format!("search-tab-context-{owner:?}-{}", id.0))
                            .absolute()
                            .top_0()
                            .right_0()
                            .bottom_0()
                            .left_0()
                            .context_menu(move |menu, window, _| {
                                let rename_workspace = menu_workspace.clone();
                                let close_workspace = menu_workspace.clone();
                                let left_workspace = menu_workspace.clone();
                                let right_workspace = menu_workspace.clone();
                                menu.item(
                                    PopupMenuItem::new(crate::tr!("重命名…", "Rename…")).on_click(
                                        window.listener_for(
                                            &rename_workspace,
                                            move |this, _, window, cx| {
                                                this.rename_search_tab_dialog(owner, id, window, cx)
                                            },
                                        ),
                                    ),
                                )
                                .item(
                                    PopupMenuItem::new(crate::tr!("关闭", "Close"))
                                        .disabled(count <= 1)
                                        .on_click(window.listener_for(
                                            &close_workspace,
                                            move |this, _, window, cx| {
                                                this.close_search_tab(owner, id, window, cx)
                                            },
                                        )),
                                )
                                .separator()
                                .item(
                                    PopupMenuItem::new(crate::tr!("向左移动", "Move left"))
                                        .on_click(window.listener_for(
                                            &left_workspace,
                                            move |this, _, window, cx| {
                                                this.move_search_tab_by(owner, id, -1, window, cx)
                                            },
                                        )),
                                )
                                .item(
                                    PopupMenuItem::new(crate::tr!("向右移动", "Move right"))
                                        .on_click(window.listener_for(
                                            &right_workspace,
                                            move |this, _, window, cx| {
                                                this.move_search_tab_by(owner, id, 1, window, cx)
                                            },
                                        )),
                                )
                            }),
                    )
                    .accessibility_label(title.clone())
                    .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                        this.activate_search_tab(owner, id, window, cx);
                        this.search_tabs.focus.focus(window, cx);
                        if event.click_count() == 2 {
                            this.rename_search_tab_dialog(owner, id, window, cx);
                        }
                    }))
                    .on_drag(dragged, |dragged, _, _, cx| cx.new(|_| dragged.clone()))
                    .drag_over::<DraggedSearchTab>(|this, _, _, cx| {
                        this.border_l_2().border_color(cx.theme().primary)
                    })
                    .on_drop(
                        cx.listener(move |this, dragged: &DraggedSearchTab, window, cx| {
                            if dragged.owner == owner
                                && dragged.workspace == cx.entity().downgrade()
                            {
                                this.move_search_tab_before(owner, dragged.id, id, window, cx);
                            }
                        }),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .child(div().max_w_40().truncate().child(title.clone()))
                            .when(state.saved.submitted.is_some(), |this| {
                                this.child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(crate::tr!("等待完成", "Pending")),
                                )
                            })
                            .child(
                                crate::button_accessibility::with_label(
                                    Button::new(("close-search-tab", id.0))
                                        .xsmall()
                                        .ghost()
                                        .icon(IconName::Close)
                                        .disabled(count <= 1),
                                    crate::tr!("关闭搜索标签", "Close search tab"),
                                )
                                .tooltip(crate::tr_args!("关闭 {}", "Close {}", title))
                                .on_click(cx.listener(
                                    move |this, _, window, cx| {
                                        cx.stop_propagation();
                                        this.close_search_tab(owner, id, window, cx);
                                    },
                                )),
                            ),
                    )
            }));
        h_flex()
            .id("search-tabs")
            .track_focus(&self.search_tabs.focus)
            .key_context("SearchTabs")
            .min_w_0()
            .w_full()
            .h(px(30.))
            .gap_1()
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if !this.search_tabs.focus.is_focused(window) {
                    return;
                }
                let Some((owner, id)) = this.active_search_tab_key() else {
                    return;
                };
                let key = event.keystroke.key.as_str();
                match key {
                    "left" | "right" => {
                        let delta = if key == "left" { -1 } else { 1 };
                        if event.keystroke.modifiers.alt {
                            this.move_search_tab_by(owner, id, delta, window, cx);
                        } else {
                            this.select_search_tab_by(owner, id, delta, window, cx);
                        }
                    }
                    "home" | "end" => {
                        let group = &this.search_tabs.groups[&owner];
                        let target = if key == "home" {
                            group.tabs.first()
                        } else {
                            group.tabs.last()
                        }
                        .map(|tab| SearchTabId(tab.saved.id));
                        if let Some(target) = target {
                            this.activate_search_tab(owner, target, window, cx);
                        }
                    }
                    "f2" => this.rename_search_tab_dialog(owner, id, window, cx),
                    "delete" | "backspace" => this.close_search_tab(owner, id, window, cx),
                    "enter" => this.query.focus_handle(cx).focus(window, cx),
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .child(track)
            .into_any_element()
    }

    pub(super) fn render_add_search_tab(&self, cx: &mut Context<Self>) -> AnyElement {
        let can_add = self
            .search_tab_owner()
            .is_some_and(|owner| self.search_tabs.groups.contains_key(&owner));
        crate::button_accessibility::with_label(
            Button::new("add-search-tab")
                .small()
                .ghost()
                .icon(IconName::Plus)
                .disabled(!can_add),
            crate::tr!("新增搜索标签", "Add search tab"),
        )
        .tooltip(crate::tr!("新增搜索标签", "Add search tab"))
        .on_click(cx.listener(|this, _, window, cx| this.add_search_tab(window, cx)))
        .into_any_element()
    }

    fn add_search_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_search_tab(window, cx);
        self.capture_active_search_tab(cx);
        let Some((owner, current)) = self.active_search_tab_key() else {
            return;
        };
        let Some(source) = self.search_tabs.state(owner, current).cloned() else {
            return;
        };
        let group = self.search_tabs.groups.get_mut(&owner).unwrap();
        let id = SearchTabId(group.next_id);
        group.next_id = group.next_id.saturating_add(1);
        let saved = PersistedSearchTab {
            id: id.0,
            draft: SearchTabQuery {
                text: String::new(),
                ..source.saved.draft
            },
            directory: source.saved.directory,
            selected_paths: source.saved.selected_paths,
            targets_configured: source.saved.targets_configured,
            ranges: source.saved.ranges,
            context: PersistedGlobalSearchContext {
                result_mode: source.saved.context.result_mode,
                word_wrap: source.saved.context.word_wrap,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut state = SearchTabState::restored(saved);
        state.context.result_mode = ResultMode::from_database(state.saved.context.result_mode);
        state.context.word_wrap = state.saved.context.word_wrap;
        group.tabs.push(state);
        self.activate_search_tab(owner, id, window, cx);
        self.query.focus_handle(cx).focus(window, cx);
    }

    pub(super) fn activate_search_tab(
        &mut self,
        owner: SearchTabOwner,
        id: SearchTabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.search_tab_owner() != Some(owner) || self.search_tabs.state(owner, id).is_none() {
            return;
        }
        self.capture_active_search_tab(cx);
        self.search_tabs.groups.get_mut(&owner).unwrap().active = id;
        self.install_search_tab(owner, id, window, cx);
        if let Some(group) = self.search_tabs.groups.get(&owner)
            && let Some(ix) = group.tabs.iter().position(|tab| tab.saved.id == id.0)
        {
            self.search_tabs.scroll.scroll_to_item(ix);
        }
        self.persist_search_tabs(window, cx);
    }

    fn close_search_tab(
        &mut self,
        owner: SearchTabOwner,
        id: SearchTabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.search_tab_owner() != Some(owner) {
            return;
        }
        let Some(group) = self.search_tabs.groups.get(&owner) else {
            return;
        };
        if group.tabs.len() <= 1 {
            return;
        }
        let Some(ix) = group.tabs.iter().position(|tab| tab.saved.id == id.0) else {
            return;
        };
        self.capture_active_search_tab(cx);
        self.search_tabs.cancel(owner, id);
        let group = self.search_tabs.groups.get_mut(&owner).unwrap();
        group.tabs.remove(ix);
        if group.active == id {
            group.active = SearchTabId(group.tabs[ix.min(group.tabs.len() - 1)].saved.id);
            let next = group.active;
            self.install_search_tab(owner, next, window, cx);
        }
        self.persist_search_tabs(window, cx);
        self.pump_search_tab_queue(window, cx);
        cx.notify();
    }

    fn select_search_tab_by(
        &mut self,
        owner: SearchTabOwner,
        id: SearchTabId,
        delta: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(group) = self.search_tabs.groups.get(&owner) else {
            return;
        };
        let Some(ix) = group.tabs.iter().position(|tab| tab.saved.id == id.0) else {
            return;
        };
        let next = ix.saturating_add_signed(delta).min(group.tabs.len() - 1);
        let target = SearchTabId(group.tabs[next].saved.id);
        self.activate_search_tab(owner, target, window, cx);
    }

    fn move_search_tab_by(
        &mut self,
        owner: SearchTabOwner,
        id: SearchTabId,
        delta: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(group) = self.search_tabs.groups.get_mut(&owner) else {
            return;
        };
        let Some(ix) = group.tabs.iter().position(|tab| tab.saved.id == id.0) else {
            return;
        };
        let next = ix.saturating_add_signed(delta).min(group.tabs.len() - 1);
        group.tabs.swap(ix, next);
        self.persist_search_tabs(window, cx);
        cx.notify();
    }

    fn move_search_tab_before(
        &mut self,
        owner: SearchTabOwner,
        id: SearchTabId,
        target: SearchTabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if id == target {
            return;
        }
        let Some(group) = self.search_tabs.groups.get_mut(&owner) else {
            return;
        };
        let Some(from) = group.tabs.iter().position(|tab| tab.saved.id == id.0) else {
            return;
        };
        let Some(to) = group.tabs.iter().position(|tab| tab.saved.id == target.0) else {
            return;
        };
        let tab = group.tabs.remove(from);
        group.tabs.insert(to, tab);
        self.persist_search_tabs(window, cx);
        cx.notify();
    }

    fn rename_search_tab_dialog(
        &mut self,
        owner: SearchTabOwner,
        id: SearchTabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = self.search_tabs.state(owner, id) else {
            return;
        };
        let rename = cx.new(|cx| RenameTabDialog::new(&state.title(), window, cx));
        let input = rename.read(cx).input();
        input.focus_handle(cx).focus(window, cx);
        input.update(cx, |input, cx| input.select_all(window, cx));
        let workspace = cx.entity();
        window.open_dialog(cx, move |dialog, _, cx| {
            let rename_for_submit = rename.clone();
            let workspace = workspace.clone();
            dialog
                .title(crate::tr!("重命名搜索标签", "Rename search tab"))
                .close_button(false)
                .child(rename.clone())
                .footer(
                    DialogFooter::new()
                        .child(crate::dialog_focus::dialog_cancel_action(
                            "rename-search-cancel-action",
                            Button::new("rename-search-cancel").label(crate::tr!("取消", "Cancel")),
                            cx,
                        ))
                        .child(crate::dialog_focus::dialog_confirm_action(
                            "rename-search-save-action",
                            Button::new("rename-search-save")
                                .primary()
                                .label(crate::tr!("保存", "Save")),
                            cx,
                        )),
                )
                .on_ok(move |_, window, cx| {
                    let Some(title) = rename_for_submit.read(cx).title(cx) else {
                        rename_for_submit.update(cx, |rename, cx| rename.show_validation_error(cx));
                        return false;
                    };
                    workspace.update(cx, |this, cx| {
                        if let Some(state) = this.search_tabs.state_mut(owner, id) {
                            state.saved.name = Some(title);
                        }
                        this.persist_search_tabs(window, cx);
                        cx.notify();
                    });
                    true
                })
        });
    }
}
