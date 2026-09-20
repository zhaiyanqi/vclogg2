use super::*;

// Match the horizontal Large Segmented tab, without repeating the bar's outer inset.
const VERTICAL_TAB_HEIGHT: Pixels = px(36.);
const VERTICAL_TAB_INDICATOR_HEIGHT: Pixels = px(28.);

#[derive(Clone, Copy)]
enum VerticalTabMenuTarget {
    Tab(WorkspaceTabId),
    End,
}

#[derive(Default)]
pub(super) struct VerticalTabState {
    scroll: UniformListScrollHandle,
    pub(super) revealed: Cell<Option<WorkspaceTabId>>,
    indicator: Option<TabIndicatorMotion>,
}

struct TabIndicatorMotion {
    state: gpui_kit::SpringState,
    target: f32,
    updated_at: std::time::Instant,
}

impl VerticalTabState {
    fn sample_indicator(
        &mut self,
        active_ix: Option<usize>,
        window: &mut Window,
        cx: &App,
    ) -> Option<Pixels> {
        let Some(ix) = active_ix else {
            self.indicator = None;
            return None;
        };
        let target = VERTICAL_TAB_HEIGHT.as_f32() * ix as f32;
        let now = cx.background_executor().now();
        let motion = self.indicator.get_or_insert(TabIndicatorMotion {
            state: gpui_kit::SpringState {
                position: target,
                velocity: 0.,
            },
            target,
            updated_at: now,
        });
        // Match gpui-component's segmented TabBar: 250 ms response, damping 0.85,
        // and 0.1 px settling tolerance. Product motion does not follow OS preferences.
        let frequency = std::f32::consts::TAU / 0.25;
        let spring = gpui_kit::SpringConfig::new(frequency * frequency, 2. * 0.85 * frequency, 1.);
        motion.state = spring.step(
            motion.state,
            motion.target,
            now.saturating_duration_since(motion.updated_at)
                .as_secs_f32(),
        );
        motion.target = target;
        motion.updated_at = now;
        if spring.is_settled(motion.state, target, 0.1) {
            motion.state = gpui_kit::SpringState {
                position: target,
                velocity: 0.,
            };
        } else {
            window.request_animation_frame();
        }
        Some(px(motion.state.position))
    }
}

impl Workspace {
    pub(super) fn tab_orientation_menu(
        menu: PopupMenu,
        enabled: bool,
        workspace: Entity<Self>,
        window: &mut Window,
    ) -> PopupMenu {
        menu.separator().item(
            PopupMenuItem::new(crate::tr!("垂直标签页", "Vertical tabs"))
                .checked(enabled)
                .on_click(window.listener_for(&workspace, |this, _, window, cx| {
                    this.toggle_vertical_tabs(window, cx);
                })),
        )
    }

    pub(super) fn render_vertical_tabs(
        &mut self,
        focus: FocusHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let active_ix = self.active_workspace_tab_ix();
        let indicator_y = self
            .vertical_tab_state
            .sample_indicator(active_ix, window, cx);
        let scroll = self.vertical_tab_state.scroll.clone();
        let tab_count = self.tabs.len();
        let reveal = self.vertical_tab_state.revealed.get() != Some(self.active_tab_id)
            || self.pending_document_tab_reveal.get().is_some();
        let reveal_tab = self
            .pending_document_tab_reveal
            .get()
            .map(WorkspaceTabId::Document)
            .unwrap_or(self.active_tab_id);
        if reveal && let Some(ix) = self.tabs.iter().position(|tab| *tab == reveal_tab) {
            scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
            self.vertical_tab_state
                .revealed
                .set(Some(self.active_tab_id));
            self.pending_document_tab_reveal.set(None);
        }
        let layout = self.tab_drop_layout.clone();
        {
            let mut layout = layout.borrow_mut();
            layout.vertical = true;
            layout.tabs.resize(tab_count, Bounds::default());
            layout.tabs.fill(Bounds::default());
            layout.end = Bounds::default();
            layout.viewport = None;
        }
        let viewport_layout = layout.clone();
        let menu_target = Rc::new(Cell::new(None));
        let clicked_target = menu_target.clone();
        let rendered_tabs = self.tabs.clone();
        let menu_workspace = cx.entity();
        v_flex()
            .id("vertical-document-tabs")
            .role(gpui_kit::Role::TabList)
            .size_full()
            .min_h_0()
            .min_w_0()
            .track_focus(&focus)
            .tab_index(0)
            .border_1()
            .border_color(cx.theme().transparent)
            .focus_visible(|style| style.border_color(cx.theme().ring))
            .on_mouse_down(MouseButton::Right, move |event, _, _| {
                let layout = layout.borrow();
                let target = layout
                    .viewport
                    .filter(|viewport| viewport.contains(&event.position))
                    .and_then(|_| {
                        layout
                            .tabs
                            .iter()
                            .position(|bounds| bounds.contains(&event.position))
                            .and_then(|ix| rendered_tabs.get(ix).copied())
                            .map(VerticalTabMenuTarget::Tab)
                            .or_else(|| {
                                layout
                                    .end
                                    .contains(&event.position)
                                    .then_some(VerticalTabMenuTarget::End)
                            })
                    });
                // Capture the clicked domain identity before the deferred menu builder runs.
                clicked_target.set(target);
            })
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.modifiers.control
                    || event.keystroke.modifiers.platform
                    || event.keystroke.modifiers.alt
                {
                    return;
                }
                let ix = this.active_workspace_tab_ix().unwrap_or(0);
                let last = this.tabs.len().saturating_sub(1);
                let next = match event.keystroke.key.as_str() {
                    "up" => ix.saturating_sub(1),
                    "down" => (ix + 1).min(last),
                    "home" => 0,
                    "end" => last,
                    _ => return,
                };
                if let Some(tab_id) = this.tabs.get(next).copied() {
                    this.activate_workspace_tab(tab_id, window, cx);
                }
                cx.stop_propagation();
            }))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .on_prepaint(move |bounds, _, _| {
                        viewport_layout.borrow_mut().viewport = Some(bounds);
                    })
                    .child(
                        uniform_list(
                            "vertical-document-tab-rows",
                            tab_count + 1,
                            cx.processor(move |this, range: Range<usize>, _, cx| {
                                range
                                    .map(|ix| {
                                        if ix == this.tabs.len() {
                                            this.render_vertical_tab_end(cx)
                                        } else {
                                            this.render_vertical_tab_row(ix, indicator_y, cx)
                                        }
                                    })
                                    .collect::<Vec<_>>()
                            }),
                        )
                        .track_scroll(&scroll)
                        .size_full(),
                    )
                    .vertical_scrollbar(&scroll),
            )
            // UniformList lays out visible rows during prepaint. A row-owned ContextMenu
            // would change focus after another node has claimed the frame's a11y focus.
            // Keep the host outside the list so it focuses the menu during root layout.
            .context_menu(move |menu, window, cx| {
                Self::build_vertical_tab_menu(
                    menu,
                    menu_target.get(),
                    menu_workspace.clone(),
                    window,
                    cx,
                )
            })
            .into_any_element()
    }

    fn build_vertical_tab_menu(
        menu: PopupMenu,
        target: Option<VerticalTabMenuTarget>,
        workspace: Entity<Self>,
        window: &mut Window,
        cx: &mut Context<PopupMenu>,
    ) -> PopupMenu {
        let tab_id = match target {
            Some(VerticalTabMenuTarget::Tab(tab_id)) => tab_id,
            Some(VerticalTabMenuTarget::End) => {
                return Self::tab_orientation_menu(menu, true, workspace, window);
            }
            None => return menu,
        };
        let this = workspace.read(cx);
        let Some(tab_ix) = this.tabs.iter().position(|id| *id == tab_id) else {
            return menu;
        };
        let document_id = tab_id.document_id();
        let state = TabMenuState {
            tab_ix,
            tab_count: this.tabs.len(),
            can_restore_title: document_id.is_some_and(|id| {
                this.documents
                    .iter()
                    .find(|tab| tab.id == id)
                    .is_some_and(|tab| tab.file.custom_title.is_some())
            }),
            has_other_window: cx
                .global::<WorkspaceWindowRegistry>()
                .previous_window(window.window_handle())
                .is_some(),
            vertical_tabs: this.vertical_tabs_enabled(cx),
            vertical: true,
            can_edit: document_id
                .and_then(|id| this.documents.iter().find(|tab| tab.id == id))
                .is_some_and(|tab| {
                    tab.document.metadata().file_size <= document_editing::MAX_EDIT_BYTES
                }),
            editing: document_id
                .and_then(|id| this.documents.iter().find(|tab| tab.id == id))
                .and_then(|tab| tab.edit.as_ref())
                .is_some_and(|edit| edit.active),
        };
        match document_id {
            Some(id) => Self::build_tab_menu(menu, id, state, workspace, window),
            None => Self::build_new_tab_menu(menu, tab_id, state, workspace, window),
        }
    }

    fn render_vertical_tab_end(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let tab_count = self.tabs.len();
        let layout = self.tab_drop_layout.clone();
        h_flex()
            .id("vertical-document-tab-end-drop")
            .w_full()
            .h(VERTICAL_TAB_HEIGHT)
            .flex_shrink_0()
            .px(px(5.))
            .when(self.cross_window_drop_ix == Some(tab_count), |this| {
                this.border_t_2().border_color(cx.theme().primary)
            })
            .on_prepaint(move |bounds, _, _| layout.borrow_mut().end = bounds)
            .drag_over::<DraggedTab>(|this, _, _, cx| {
                this.border_t_2().border_color(cx.theme().primary)
            })
            .on_drop(cx.listener(move |this, dragged: &DraggedTab, window, cx| {
                this.reorder_tab(dragged.tab_id, tab_count, window, cx);
            }))
            .child(
                Button::new("vertical-new-workspace-tab")
                    .small()
                    .ghost()
                    .w_full()
                    .h(VERTICAL_TAB_INDICATOR_HEIGHT)
                    .rounded((cx.theme().radius_lg - px(3.)).max(px(0.)))
                    .icon(IconName::Plus)
                    .tooltip(crate::tr!("新建标签页", "New tab"))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.create_new_tab(window, cx);
                    })),
            )
            .into_any_element()
    }

    fn render_vertical_tab_row(
        &mut self,
        ix: usize,
        indicator_y: Option<Pixels>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(tab_id) = self.tabs.get(ix).copied() else {
            return div().into_any_element();
        };
        let key = match tab_id {
            WorkspaceTabId::Document(id) => format!("document-{id}"),
            WorkspaceTabId::New(id) => format!("new-{id}"),
        };
        // Match the horizontal segmented TabBar's frame; only the stack axis changes.
        div()
            .id(SharedString::from(format!("vertical-tab-{key}")))
            .relative()
            .w_full()
            .min_w_0()
            .overflow_hidden()
            .h(VERTICAL_TAB_HEIGHT)
            .px(px(5.))
            .when_some(indicator_y, |this, y| {
                this.child(
                    div()
                        .absolute()
                        .left(px(5.))
                        .right(px(5.))
                        .top(
                            y - VERTICAL_TAB_HEIGHT * ix as f32
                                + (VERTICAL_TAB_HEIGHT - VERTICAL_TAB_INDICATOR_HEIGHT) / 2.,
                        )
                        .h(VERTICAL_TAB_INDICATOR_HEIGHT)
                        .bg(cx.theme().tokens.background)
                        .rounded((cx.theme().radius_lg - px(3.)).max(px(0.)))
                        .shadow_sm(),
                )
            })
            .child(self.render_vertical_tab_shell(ix, cx))
            .into_any_element()
    }

    pub(super) fn render_workspace_tab(
        &self,
        ix: usize,
        has_other_window: bool,
        vertical: bool,
        cx: &mut Context<Self>,
    ) -> Tab {
        let tab_id = self.tabs[ix];
        let tab_count = self.tabs.len();
        let workspace = cx.entity();
        let source_workspace = cx.weak_entity();
        let tab_drop_layout = self.tab_drop_layout.clone();
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
            vertical_tabs: self.vertical_tabs_enabled(cx),
            vertical,
            can_edit: match tab_id {
                WorkspaceTabId::Document(id) => self
                    .documents
                    .iter()
                    .find(|tab| tab.id == id)
                    .is_some_and(|tab| {
                        tab.document.metadata().file_size <= document_editing::MAX_EDIT_BYTES
                    }),
                WorkspaceTabId::New(id) => self.new_file_drafts.contains_key(&id),
            },
            editing: match tab_id {
                WorkspaceTabId::Document(id) => self
                    .documents
                    .iter()
                    .find(|tab| tab.id == id)
                    .and_then(|tab| tab.edit.as_ref())
                    .is_some_and(|edit| edit.active),
                WorkspaceTabId::New(id) => self
                    .new_file_drafts
                    .get(&id)
                    .is_some_and(|draft| draft.active),
            },
        };
        let context_workspace = workspace.clone();
        let tab_layout = tab_drop_layout.clone();
        let selected = self.active_tab_id == tab_id;
        let (_, context_target_id) = match tab_id {
            WorkspaceTabId::Document(id) => (
                ElementId::from(("close-document-tab", id)),
                ElementId::from(("document-tab-context-target", id)),
            ),
            WorkspaceTabId::New(id) => (
                ElementId::from(("close-new-tab", id)),
                ElementId::from(("new-tab-context-target", id)),
            ),
        };
        Tab::new()
            .aria_label(tab_title.clone())
            .selected(selected)
            .on_click(cx.listener(move |this, _, window, cx| {
                this.activate_workspace_tab(tab_id, window, cx);
            }))
            .when(self.cross_window_drop_ix == Some(ix), |this| {
                if vertical {
                    this.border_t_2().border_color(cx.theme().primary)
                } else {
                    this.border_l_2().border_color(cx.theme().primary)
                }
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
                } else if vertical {
                    this.border_t_2().border_color(cx.theme().primary)
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
                self.render_workspace_tab_body(tab_id, vertical, cx)
                    .mx(px(-6.)),
            )
    }

    fn render_vertical_tab_shell(&self, ix: usize, cx: &mut Context<Self>) -> gpui_kit::base::Tab {
        let tab_id = self.tabs[ix];
        let tab_count = self.tabs.len();
        let source_workspace = cx.weak_entity();
        let tab_drop_layout = self.tab_drop_layout.clone();
        let tab_title = self.workspace_tab_title(tab_id);
        let dragged_tab = DraggedTab::new(tab_id, tab_title.clone(), source_workspace.clone());
        let tab_layout = tab_drop_layout.clone();
        let selected = self.active_tab_id == tab_id;
        let shell_id = match tab_id {
            WorkspaceTabId::Document(id) => ElementId::from(("document-tab-context-target", id)),
            WorkspaceTabId::New(id) => ElementId::from(("new-tab-context-target", id)),
        };
        gpui_kit::base::Tab::new(shell_id)
            .relative()
            .flex()
            .items_center()
            .w_full()
            .min_w_0()
            .h(VERTICAL_TAB_HEIGHT)
            .px(px(6.))
            .text_color(cx.theme().tab_foreground)
            .styles(|styles| {
                styles.selected(|style| style.text_color(cx.theme().tab_active_foreground))
            })
            .hover(|style| style.text_color(cx.theme().tab_active_foreground))
            .accessibility_label(tab_title.clone())
            .selected(selected)
            .set_position(ix + 1, tab_count)
            .on_click(cx.listener(move |this, _, window, cx| {
                this.activate_workspace_tab(tab_id, window, cx);
            }))
            .when(self.cross_window_drop_ix == Some(ix), |this| {
                this.border_t_2().border_color(cx.theme().primary)
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
                    this.border_t_2().border_color(cx.theme().primary)
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
            .child(self.render_workspace_tab_body(tab_id, true, cx))
    }

    fn render_workspace_tab_body(
        &self,
        tab_id: WorkspaceTabId,
        vertical: bool,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Div {
        let tab_title = self.workspace_tab_title(tab_id);
        let selected = self.active_tab_id == tab_id;
        let dirty = match tab_id {
            WorkspaceTabId::Document(document_id) => self
                .documents
                .iter()
                .find(|tab| tab.id == document_id)
                .and_then(|tab| tab.edit.as_ref())
                .is_some_and(|edit| edit.dirty),
            WorkspaceTabId::New(id) => self
                .new_file_drafts
                .get(&id)
                .is_some_and(|draft| draft.dirty),
        };
        let visible_title = if dirty {
            format!("{tab_title} *")
        } else {
            tab_title.to_string()
        };
        let file_icon_color = if dirty {
            cx.theme().danger
        } else if selected {
            cx.theme().tab_active_foreground
        } else {
            cx.theme().tab_foreground
        };
        let close_button_id = match tab_id {
            WorkspaceTabId::Document(id) => ElementId::from(("close-document-tab", id)),
            WorkspaceTabId::New(id) => ElementId::from(("close-new-tab", id)),
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
                cx.stop_propagation();
            }));
        h_flex()
            .when(vertical, |this| this.w_full().min_w_0())
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
                    .opacity(if dirty { 1. } else { 0.72 }),
            )
            .child(
                div()
                    .min_w_0()
                    .when(vertical, |this| this.flex_1())
                    .truncate()
                    .line_height(relative(1.5))
                    .child(visible_title),
            )
            .child(close_button)
    }
}
