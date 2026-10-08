use super::search_tabs::{SearchTabId, SearchTabOwner, SearchTabState};
use super::*;
use crate::search_context::{PersistedSearchTab, SearchTabQuery};
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::base::{Tab as SearchResultTab, Tabs as SearchResultTabs};
use gpui_kit::component::spinner::Spinner;

#[derive(Clone)]
struct DraggedSearchTab {
    owner: SearchTabOwner,
    id: SearchTabId,
    workspace: WeakEntity<Workspace>,
    title: SharedString,
    selected: bool,
    busy: bool,
    closable: bool,
    size: Size<Pixels>,
}

fn search_tab_surface(id: impl Into<ElementId>, selected: bool, cx: &App) -> SearchResultTab {
    // Bottom tabs join the results above: the active tab has no top border.
    SearchResultTab::new(id)
        .selected(selected)
        .relative()
        .flex()
        .items_center()
        .flex_shrink_0()
        .h_full()
        .px_2()
        .text_sm()
        .line_height(relative(1.5))
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
}

fn search_tab_contents(
    label_id: impl Into<ElementId>,
    owner: SearchTabOwner,
    title: SharedString,
    busy: bool,
    close: Button,
) -> impl IntoElement {
    h_flex()
        .gap_1()
        .child(if busy {
            Spinner::new().small().into_any_element()
        } else {
            Icon::new(owner.icon()).small().into_any_element()
        })
        .child(
            div()
                .id(label_id)
                .test_support()
                .max_w_80()
                .line_height(relative(1.5))
                .truncate()
                .child(title),
        )
        .child(close)
}

impl Render for DraggedSearchTab {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        search_tab_surface("search-tab-drag-preview", self.selected, cx)
            .w(self.size.width)
            .h(self.size.height)
            .child(search_tab_contents(
                "search-tab-drag-preview-label",
                self.owner,
                self.title.clone(),
                self.busy,
                Button::new("search-tab-drag-preview-close")
                    .disabled(!self.closable)
                    .xsmall()
                    .ghost()
                    .icon(IconName::Close),
            ))
    }
}

impl Workspace {
    pub(super) fn search_panel_visible(&self) -> bool {
        match self.active_tab_id {
            WorkspaceTabId::New(id) => self
                .new_file_drafts
                .get(&id)
                .is_none_or(|draft| !draft.active),
            WorkspaceTabId::Document(_) => self.active_document().is_none_or(|tab| {
                tab.edit.as_ref().is_none_or(|edit| !edit.active) && tab.edit_load_task.is_none()
            }),
        }
    }

    pub(super) fn search_panel_expanded(&self) -> bool {
        if self.global_search.scope != SearchScope::CurrentFile {
            return self.search_tabs.shared_panel_expanded;
        }
        self.search_tabs
            .panel_expansion
            .get(&self.active_tab_id)
            .copied()
            .unwrap_or(matches!(self.active_tab_id, WorkspaceTabId::Document(_)))
    }

    pub(super) fn set_search_panel_expanded(&mut self, expanded: bool, cx: &mut Context<Self>) {
        self.search_tabs
            .panel_expansion
            .retain(|id, _| self.tabs.contains(id));
        if self.global_search.scope == SearchScope::CurrentFile {
            self.search_tabs
                .panel_expansion
                .insert(self.active_tab_id, expanded);
        } else {
            self.search_tabs.shared_panel_expanded = expanded;
        }
        self.search_panel_resize_gesture = None;
        self.search_panel_resize_bounds.set(None);
        cx.notify();
    }

    pub(super) fn render_search_tabs(&self, cx: &mut Context<Self>) -> AnyElement {
        let keys = self.visible_search_tab_keys();
        let active = self.active_search_tab_key();
        let workspace = cx.entity();
        let track = SearchResultTabs::new("search-tab-track")
            .flex()
            .items_start()
            .min_w_0()
            .flex_1()
            .h_full()
            .overflow_x_scroll()
            .track_scroll(&self.search_tabs.scroll)
            .on_prepaint({
                let layout = self.search_tabs.layout.clone();
                move |bounds, _, _| layout.borrow_mut().track = Some(bounds)
            })
            .on_drop(cx.listener(|_, _: &DraggedSearchTab, _, _| {}))
            .children(keys.into_iter().map(|(owner, id)| {
                let state = self.search_tabs.state(owner, id).unwrap();
                let title = self.search_tab_title(owner, state);
                let menu_workspace = workspace.clone();
                let selected = active == Some((owner, id));
                let closable = self.search_tabs.groups[&owner].is_closable(owner, id);
                let dragged = DraggedSearchTab {
                    owner,
                    id,
                    workspace: workspace.downgrade(),
                    title: title.clone(),
                    selected,
                    busy: state.saved.submitted.is_some(),
                    closable,
                    size: self
                        .search_tabs
                        .layout
                        .borrow()
                        .slots
                        .get(&(owner, id))
                        .map(|bounds| bounds.size)
                        .unwrap_or_default(),
                };
                let slot_layout = self.search_tabs.layout.clone();
                let painted_layout = slot_layout.clone();
                let offset = self
                    .search_tabs
                    .motion_offsets
                    .get(&(owner, id))
                    .copied()
                    .unwrap_or_default();
                let revision = self.search_tabs.order_revision;
                let tab =
                    search_tab_surface(format!("search-tab-{owner:?}-{}", id.0), selected, cx)
                        .opacity(if self.search_tabs.hidden_drag == Some((owner, id)) {
                            0.
                        } else {
                            1.
                        })
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
                                        PopupMenuItem::new(crate::tr!("重命名…", "Rename…"))
                                            .on_click(window.listener_for(
                                                &rename_workspace,
                                                move |this, _, window, cx| {
                                                    this.rename_search_tab_dialog(
                                                        owner, id, window, cx,
                                                    )
                                                },
                                            )),
                                    )
                                    .item(
                                        PopupMenuItem::new(crate::tr!("关闭", "Close"))
                                            .disabled(!closable)
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
                                                    this.move_search_tab_by(
                                                        owner, id, -1, window, cx,
                                                    )
                                                },
                                            )),
                                    )
                                    .item(
                                        PopupMenuItem::new(crate::tr!("向右移动", "Move right"))
                                            .on_click(window.listener_for(
                                                &right_workspace,
                                                move |this, _, window, cx| {
                                                    this.move_search_tab_by(
                                                        owner, id, 1, window, cx,
                                                    )
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
                        .on_aux_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                            if event.is_middle_click() {
                                cx.stop_propagation();
                                this.close_search_tab(owner, id, window, cx);
                            }
                        }))
                        .on_drag(dragged, |dragged, cursor_offset, window, cx| {
                            let mut preview = dragged.clone();
                            _ = dragged.workspace.update(cx, |this, _| {
                                if let Some(bounds) = this
                                    .search_tabs
                                    .layout
                                    .borrow()
                                    .slots
                                    .get(&(dragged.owner, dragged.id))
                                {
                                    preview.size = bounds.size;
                                }
                            });
                            let view = cx.new(|_| preview);
                            _ = dragged.workspace.update(cx, |this, cx| {
                                this.begin_tab_drag(
                                    super::tab_drag::TabDragKey::Search(dragged.owner, dragged.id),
                                    view.clone().into(),
                                    cursor_offset,
                                    cx,
                                );
                                this.move_search_tab_drag(window.mouse_position().x, window, cx);
                            });
                            view
                        })
                        .child(search_tab_contents(
                            format!("search-tab-label-{owner:?}-{}", id.0),
                            owner,
                            title.clone(),
                            state.saved.submitted.is_some(),
                            crate::button_accessibility::with_label(
                                Button::new(format!("close-search-tab-{owner:?}-{}", id.0))
                                    .disabled(!closable)
                                    .xsmall()
                                    .ghost()
                                    .icon(IconName::Close),
                                crate::tr!("关闭搜索标签", "Close search tab"),
                            )
                            .tooltip(crate::tr_args!("关闭 {}", "Close {}", title))
                            .on_click(cx.listener(
                                move |this, _, window, cx| {
                                    cx.stop_propagation();
                                    this.close_search_tab(owner, id, window, cx);
                                },
                            )),
                        ))
                        .on_prepaint(move |bounds, _, _| {
                            painted_layout
                                .borrow_mut()
                                .painted
                                .insert((owner, id), bounds);
                        })
                        .map(|tab| {
                            if offset == px(0.) {
                                tab.into_any_element()
                            } else {
                                tab.with_animation(
                                    ("search-tab-reorder", revision),
                                    Animation::new(super::tab_drag::ANIMATION_DURATION)
                                        .with_easing(ease_out_cubic),
                                    move |tab, progress| tab.left(offset * (1. - progress)),
                                )
                                .into_any_element()
                            }
                        });
                div()
                    .id(format!("search-tab-slot-{owner:?}-{}", id.0))
                    .flex()
                    .flex_shrink_0()
                    .h_full()
                    .on_prepaint(move |bounds, _, _| {
                        slot_layout.borrow_mut().slots.insert((owner, id), bounds);
                    })
                    .child(tab)
            }));
        h_flex()
            .id("search-tabs")
            .track_focus(&self.search_tabs.focus)
            .key_context("SearchTabs")
            .min_w_0()
            .w_full()
            .h_8()
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
                        let keys = this.visible_search_tab_keys();
                        let target = if key == "home" {
                            keys.first()
                        } else {
                            keys.last()
                        };
                        if let Some(&(owner, id)) = target {
                            this.activate_search_tab(owner, id, window, cx);
                        }
                    }
                    "f2" => this.rename_search_tab_dialog(owner, id, window, cx),
                    "delete" | "backspace" => this.close_search_tab(owner, id, window, cx),
                    "enter" => this.search_input_focus_handle(cx).focus(window, cx),
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .child(track)
            .into_any_element()
    }

    pub(super) fn render_add_search_tab(&self, cx: &mut Context<Self>) -> AnyElement {
        let workspace = cx.entity();
        let file_owner = self
            .active_document()
            .filter(|tab| tab.load_state == DocumentLoadState::Ready)
            .map(|tab| SearchTabOwner::File(tab.id));
        crate::button_accessibility::with_label(
            Button::new("add-search-tab")
                .small()
                .ghost()
                .icon(IconName::Plus),
            crate::tr!("新增搜索标签", "Add search tab"),
        )
        .tooltip(crate::tr!("新增搜索标签", "Add search tab"))
        .dropdown_menu_with_anchor(gpui_kit::Anchor::BottomLeft, move |menu, window, cx| {
            let mut menu = Self::popup_menu_with_workspace_action_context(menu, &workspace, cx);
            for (label, owner, icon) in [
                (
                    crate::tr!("当前文件", "Current file"),
                    file_owner,
                    gpui_kit::assets::IconName::Search,
                ),
                (
                    crate::tr!("全局搜索", "Global search"),
                    Some(SearchTabOwner::AllOpen),
                    gpui_kit::assets::IconName::Earth,
                ),
                (
                    crate::tr!("目录搜索", "Directory search"),
                    Some(SearchTabOwner::Directory),
                    gpui_kit::assets::IconName::FolderOpen,
                ),
            ] {
                menu = menu.item(
                    PopupMenuItem::new(label)
                        .icon(icon)
                        .disabled(owner.is_none())
                        .on_click(window.listener_for(&workspace, move |this, _, window, cx| {
                            if let Some(owner) = owner {
                                this.add_search_tab(owner, window, cx);
                            }
                        })),
                );
            }
            menu
        })
        .into_any_element()
    }

    fn add_search_tab(
        &mut self,
        owner: SearchTabOwner,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.capture_active_search_tab(cx);
        let existed = self
            .search_tabs
            .groups
            .get(&owner)
            .is_some_and(|group| !group.tabs.is_empty());
        self.ensure_search_tab_group(owner, cx);
        let current = self.search_tabs.groups[&owner].active;
        if !existed {
            self.activate_search_tab(owner, current, window, cx);
            self.set_search_panel_expanded(true, cx);
            self.search_input_focus_handle(cx).focus(window, cx);
            return;
        }
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
        self.set_search_panel_expanded(true, cx);
        self.search_input_focus_handle(cx).focus(window, cx);
    }

    pub(super) fn activate_search_tab(
        &mut self,
        owner: SearchTabOwner,
        id: SearchTabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.visible_search_tab_keys().contains(&(owner, id)) {
            return;
        }
        if self.search_tabs.installed == Some((owner, id)) {
            self.set_search_panel_expanded(true, cx);
            self.cancel_search_tab_activation();
            return;
        }
        self.prepare_search_tab_activation(owner, id, window, cx);
    }

    pub(super) fn commit_search_tab_activation(
        &mut self,
        owner: SearchTabOwner,
        id: SearchTabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.capture_active_search_tab(cx);
        self.cancel_pending_tab_activation();
        if let SearchTabOwner::File(document_id) = owner
            && self.active_tab_id != WorkspaceTabId::Document(document_id)
        {
            // File activation syncs its remembered search. Preserve the preloaded frame
            // for the explicitly requested session instead of letting that sync consume it.
            let frame = self.search_tabs.prepared_frame.take();
            self.commit_workspace_tab_activation(
                WorkspaceTabId::Document(document_id),
                false,
                window,
                cx,
            );
            self.search_tabs.prepared_frame = frame;
        }
        self.global_search.scope = owner.scope();
        self.set_search_panel_expanded(true, cx);
        self.search_tabs.groups.get_mut(&owner).unwrap().active = id;
        self.install_search_tab(owner, id, window, cx);
        if let Some(ix) = self
            .visible_search_tab_keys()
            .iter()
            .position(|key| *key == (owner, id))
        {
            self.search_tabs.scroll.scroll_to_item(ix);
        }
        self.persist_search_tab_changes(owner, window, cx);
    }

    fn close_search_tab(
        &mut self,
        owner: SearchTabOwner,
        id: SearchTabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self
            .search_tabs
            .groups
            .get(&owner)
            .is_some_and(|group| group.is_closable(owner, id))
        {
            return;
        }
        let keys = self.visible_search_tab_keys();
        let Some(visible_ix) = keys.iter().position(|key| *key == (owner, id)) else {
            return;
        };
        let was_active = self.active_search_tab_key() == Some((owner, id));
        self.capture_active_search_tab(cx);
        self.cancel_search_tab_activation();
        self.search_tabs.cancel(owner, id);
        self.search_tabs
            .groups
            .get_mut(&owner)
            .unwrap()
            .close(owner, id);
        {
            let mut layout = self.search_tabs.layout.borrow_mut();
            layout.slots.remove(&(owner, id));
            layout.painted.remove(&(owner, id));
        }
        self.search_tabs.motion_offsets.remove(&(owner, id));
        if was_active {
            self.search_tabs.installed = None;
            let remaining = self.visible_search_tab_keys();
            if let Some(&(next_owner, next_id)) =
                remaining.get(visible_ix.min(remaining.len().saturating_sub(1)))
            {
                self.commit_search_tab_activation(next_owner, next_id, window, cx);
            } else {
                self.global_search.scope = SearchScope::CurrentFile;
                self.global_search.results_visible = false;
                self.view_state.active_search = None;
                self.query
                    .update(cx, |input, cx| input.set_value("", window, cx));
                self.global_table.update(cx, |table, cx| {
                    table.delegate_mut().set_groups(Vec::new());
                    table.refresh(cx);
                });
            }
        }
        self.persist_search_tab_changes(owner, window, cx);
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
        let keys = self.visible_search_tab_keys();
        let Some(ix) = keys.iter().position(|key| *key == (owner, id)) else {
            return;
        };
        let next = ix.saturating_add_signed(delta).min(keys.len() - 1);
        let (owner, id) = keys[next];
        self.activate_search_tab(owner, id, window, cx);
    }

    fn move_search_tab_by(
        &mut self,
        owner: SearchTabOwner,
        id: SearchTabId,
        delta: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let keys = self.visible_search_tab_keys();
        let Some(ix) = keys.iter().position(|key| *key == (owner, id)) else {
            return;
        };
        let next = ix.saturating_add_signed(delta).min(keys.len() - 1);
        self.reorder_search_tab((owner, id), next, window, cx);
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
        let rename =
            cx.new(|cx| RenameTabDialog::new(&self.search_tab_title(owner, state), window, cx));
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
                        this.persist_search_tab_changes(owner, window, cx);
                        cx.notify();
                    });
                    true
                })
        });
    }
}
