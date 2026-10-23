//! Left navigation owns its tree snapshots independently of document contents.
use super::*;
use gpui_kit::component::resizable::h_resizable;
mod file_tree;
use file_tree::FileTree;

pub(super) struct NavigationSidebar {
    visible: bool,
    vertical_tabs: bool,
    show_tabs: bool,
    tree: Entity<FileTree>,
    split: Entity<ResizableState>,
    tabs_scroll: UniformListScrollHandle,
}

impl NavigationSidebar {
    pub(super) fn new(cx: &mut Context<Workspace>) -> Self {
        let workspace = cx.weak_entity();
        Self {
            visible: true,
            vertical_tabs: false,
            show_tabs: false,
            tree: cx.new(|cx| FileTree::new(workspace, cx)),
            split: cx.new(|_| ResizableState::default()),
            tabs_scroll: UniformListScrollHandle::new(),
        }
    }
}

#[derive(Clone)]
struct NavigationTabDrag {
    tab_id: WorkspaceTabId,
    source: WeakEntity<Workspace>,
    title: SharedString,
}

impl Render for NavigationTabDrag {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .p_2()
            .bg(cx.theme().popover)
            .text_color(cx.theme().foreground)
            .child(self.title.clone())
    }
}

impl Workspace {
    pub(super) fn sync_navigation_file(&mut self, cx: &mut Context<Self>) {
        if self.navigation.visible
            && !self.navigation.show_tabs
            && let Some(path) = self
                .active_document()
                .map(|tab| tab.document.path().to_path_buf())
        {
            self.navigation
                .tree
                .update(cx, |tree, cx| tree.reveal(path, false, cx));
        }
        if self.navigation.vertical_tabs {
            self.navigation.tabs_scroll.scroll_to_item(
                self.active_workspace_tab_ix().unwrap_or_default(),
                ScrollStrategy::Nearest,
            );
        }
    }

    pub(super) fn reveal_navigation_path(
        &mut self,
        path: PathBuf,
        directory: bool,
        cx: &mut Context<Self>,
    ) {
        self.navigation.visible = true;
        self.navigation.show_tabs = false;
        self.navigation
            .tree
            .update(cx, |tree, cx| tree.reveal(path, directory, cx));
        cx.notify();
    }

    pub(super) fn reveal_active_navigation_file(&mut self, cx: &mut Context<Self>) {
        if let Some(path) = self
            .active_document()
            .map(|tab| tab.document.path().to_path_buf())
        {
            self.reveal_navigation_path(path, false, cx);
        }
    }

    pub(super) fn render_navigation_workspace(
        &mut self,
        has_other_window: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.navigation.vertical_tabs {
            let mut layout = self.tab_drop_layout.borrow_mut();
            layout.tabs.clear();
            layout.end = Bounds::default();
            layout.viewport = None;
        }
        let content = v_flex()
            .size_full()
            .min_w_0()
            .min_h_0()
            .when(!self.navigation.vertical_tabs, |view| {
                view.child(self.render_tabs(has_other_window, cx))
            })
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_tab_workspace(window, cx)),
            );
        let rail = v_flex()
            .h_full()
            .gap_1()
            .p_1()
            .border_r_1()
            .border_color(cx.theme().border)
            .child(
                Button::new("sidebar-files")
                    .small()
                    .ghost()
                    .icon(IconName::Folder)
                    .selected(self.navigation.visible && !self.navigation.show_tabs)
                    .tooltip(crate::tr!("文件树", "File tree"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        if this.navigation.visible && !this.navigation.show_tabs {
                            this.navigation.visible = false;
                        } else {
                            this.navigation.visible = true;
                            this.navigation.show_tabs = false;
                            this.reveal_active_navigation_file(cx);
                        }
                        cx.notify();
                    })),
            )
            .child(
                Button::new("sidebar-tabs")
                    .small()
                    .ghost()
                    .icon(IconName::File)
                    .selected(self.navigation.visible && self.navigation.show_tabs)
                    .tooltip(crate::tr!("纵向标签栏", "Vertical tabs"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.navigation.visible = true;
                        this.navigation.show_tabs = true;
                        this.navigation.vertical_tabs = true;
                        this.navigation.tabs_scroll.scroll_to_item(
                            this.active_workspace_tab_ix().unwrap_or_default(),
                            ScrollStrategy::Nearest,
                        );
                        cx.notify();
                    })),
            );
        if !self.navigation.visible {
            return h_flex()
                .flex_1()
                .min_h_0()
                .items_stretch()
                .child(rail)
                .child(div().flex_1().min_w_0().h_full().child(content))
                .into_any_element();
        }
        let panel = if self.navigation.show_tabs {
            self.render_navigation_tabs(has_other_window, cx)
        } else {
            self.navigation.tree.clone().into_any_element()
        };
        h_flex()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .items_stretch()
            .child(rail)
            .child(
                div().flex_1().min_w_0().h_full().child(
                    h_resizable("navigation-split")
                        .with_state(&self.navigation.split)
                        .child(
                            resizable_panel()
                                .size(window.rem_size() * 16.)
                                .size_range(window.rem_size() * 8. ..window.rem_size() * 32.)
                                .child(panel),
                        )
                        .child(resizable_panel().child(content)),
                ),
            )
            .into_any_element()
    }

    fn render_navigation_tabs(&self, has_other_window: bool, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .size_full()
            .min_h_0()
            .bg(cx.theme().sidebar)
            .child(
                h_flex()
                    .p_1()
                    .gap_1()
                    .child(
                        Button::new("vertical-new-tab")
                            .small()
                            .ghost()
                            .icon(IconName::Plus)
                            .tooltip(crate::tr!("新建标签页", "New tab"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.create_new_tab(window, cx);
                            })),
                    )
                    .child(
                        Button::new("horizontal-tabs")
                            .small()
                            .ghost()
                            .label(crate::tr!("横向标签栏", "Horizontal tabs"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.navigation.vertical_tabs = false;
                                this.navigation.show_tabs = false;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                uniform_list(
                    "vertical-document-tabs",
                    self.tabs.len(),
                    cx.processor(move |this, range: Range<usize>, _, cx| {
                        range
                            .map(|ix| {
                                let tab_id = this.tabs[ix];
                                let title = this.workspace_tab_title(tab_id);
                                let id: SharedString = format!("vertical-tab-{tab_id:?}").into();
                                let workspace = cx.entity();
                                let dragged = NavigationTabDrag {
                                    tab_id,
                                    source: cx.weak_entity(),
                                    title: title.clone(),
                                };
                                let menu_state =
                                    this.workspace_tab_menu_state(ix, has_other_window);
                                h_flex()
                                    .id(id.clone())
                                    .on_drag(dragged, |dragged, _, _, cx| {
                                        cx.new(|_| dragged.clone())
                                    })
                                    .on_drop(cx.listener(
                                        move |this, dragged: &NavigationTabDrag, window, cx| {
                                            if dragged
                                                .source
                                                .upgrade()
                                                .is_some_and(|source| source == cx.entity())
                                            {
                                                let source_ix = this
                                                    .tabs
                                                    .iter()
                                                    .position(|tab| *tab == dragged.tab_id)
                                                    .unwrap_or(ix);
                                                this.reorder_tab(
                                                    dragged.tab_id,
                                                    if source_ix < ix { ix + 1 } else { ix },
                                                    window,
                                                    cx,
                                                );
                                            }
                                        },
                                    ))
                                    .context_menu(move |menu, window, cx| {
                                        window.focus(&menu.focus_handle(cx), cx);
                                        match tab_id.document_id() {
                                            Some(document_id) => Self::build_tab_menu(
                                                menu,
                                                document_id,
                                                menu_state,
                                                workspace.clone(),
                                                window,
                                            ),
                                            None => Self::build_new_tab_menu(
                                                menu,
                                                tab_id,
                                                menu_state,
                                                workspace.clone(),
                                                window,
                                            ),
                                        }
                                    })
                                    .w_full()
                                    .p_1()
                                    .gap_1()
                                    .child(
                                        Button::new(id)
                                            .small()
                                            .ghost()
                                            .flex_1()
                                            .min_w_0()
                                            .label(title.clone())
                                            .tooltip(title)
                                            .selected(this.active_tab_id == tab_id)
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.activate_workspace_tab(tab_id, window, cx);
                                            })),
                                    )
                                    .child(
                                        Button::new(SharedString::from(format!(
                                            "vertical-close-{tab_id:?}"
                                        )))
                                        .small()
                                        .ghost()
                                        .icon(IconName::Close)
                                        .tooltip(crate::tr!("关闭标签", "Close tab"))
                                        .on_click(
                                            cx.listener(move |this, _, window, cx| {
                                                this.request_close_workspace_tabs(
                                                    BTreeSet::from([tab_id]),
                                                    window,
                                                    cx,
                                                );
                                            }),
                                        ),
                                    )
                            })
                            .collect()
                    }),
                )
                .track_scroll(&self.navigation.tabs_scroll)
                .flex_1()
                .min_h_0(),
            )
            .into_any_element()
    }
}
