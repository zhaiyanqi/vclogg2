use super::*;

impl Workspace {
    pub(super) fn render_workspace_tab(
        &self,
        ix: usize,
        has_other_window: bool,
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
            .child(self.render_workspace_tab_body(tab_id, cx).mx(px(-6.)))
    }

    fn render_workspace_tab_body(
        &self,
        tab_id: WorkspaceTabId,
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
        let editing = match tab_id {
            WorkspaceTabId::Document(document_id) => self
                .documents
                .iter()
                .find(|tab| tab.id == document_id)
                .and_then(|tab| tab.edit.as_ref())
                .is_some_and(|edit| edit.active),
            WorkspaceTabId::New(id) => self
                .new_file_drafts
                .get(&id)
                .is_some_and(|draft| draft.active),
        };
        let tab_icon: &'static [u8] = if editing {
            include_bytes!("../../assets/icons/file-pen-line.svg")
        } else {
            include_bytes!("../../assets/icons/document-text-20-regular.svg")
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
            .gap(px(8.))
            .items_center()
            .text_size(px(12.))
            .child(
                div()
                    .size(px(20.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        svg()
                            .data(tab_icon)
                            .size(px(if editing { 16. } else { 20. }))
                            .text_color(file_icon_color)
                            .opacity(if dirty { 1. } else { 0.72 }),
                    ),
            )
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .line_height(relative(1.5))
                    .child(visible_title),
            )
            .child(close_button)
    }
}
