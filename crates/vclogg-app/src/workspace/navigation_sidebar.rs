//! Left navigation owns its tree snapshots independently of document contents.
use super::*;
use gpui_kit::component::StyledExt as _;
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
    indicator: VerticalTabState,
}

impl NavigationSidebar {
    pub(super) fn new(cx: &mut Context<Workspace>) -> Self {
        let workspace = cx.weak_entity();
        Self {
            visible: false,
            vertical_tabs: false,
            show_tabs: false,
            tree: cx.new(|cx| FileTree::new(workspace, cx)),
            split: cx.new(|_| ResizableState::default()),
            tabs_scroll: UniformListScrollHandle::new(),
            indicator: VerticalTabState::default(),
        }
    }
}

// Preserve the original segmented tab geometry and motion.
const VERTICAL_TAB_HEIGHT: Pixels = px(36.);
const VERTICAL_TAB_INDICATOR_HEIGHT: Pixels = px(28.);

#[derive(Default)]
pub(super) struct VerticalTabState {
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
    pub(super) fn navigation_sidebar_visible(&self) -> bool {
        self.navigation.visible
    }

    pub(super) fn toggle_navigation_sidebar(&mut self, cx: &mut Context<Self>) {
        self.navigation.visible = !self.navigation.visible;
        if self.navigation.visible {
            self.sync_navigation_file(cx);
        }
        cx.notify();
    }

    pub(super) fn sync_navigation_file(&mut self, cx: &mut Context<Self>) {
        if self.navigation.visible
            && !self.navigation.show_tabs
            && let Some(path) = self
                .active_document()
                .map(|tab| tab.document.path().to_path_buf())
        {
            self.navigation.tree.update(cx, |tree, cx| {
                tree.reveal(path, false, ScrollStrategy::Nearest, cx)
            });
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
        self.navigation.tree.update(cx, |tree, cx| {
            tree.reveal(path, directory, ScrollStrategy::Center, cx)
        });
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
            .w(rems(2.25))
            .h_full()
            .flex_shrink_0()
            .gap_1()
            .py_1()
            .items_center()
            .children([false, true].into_iter().map(|tabs| {
                div().w_full().px_1().child(
                    Button::new(if tabs {
                        "sidebar-tabs"
                    } else {
                        "sidebar-files"
                    })
                    .ghost()
                    .small()
                    .w_full()
                    .h_7()
                    .p_0()
                    .child(
                        Icon::new(if tabs {
                            IconName::File
                        } else {
                            IconName::Folder
                        })
                        .small(),
                    )
                    .selected(self.navigation.show_tabs == tabs)
                    .tooltip(if tabs {
                        crate::tr!("标签页", "Tabs")
                    } else {
                        crate::tr!("文件", "Files")
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.navigation.show_tabs = tabs;
                        if tabs {
                            this.navigation.vertical_tabs = true;
                        }
                        this.sync_navigation_file(cx);
                        cx.notify();
                    })),
                )
            }));
        if !self.navigation.visible {
            return div()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .child(content)
                .into_any_element();
        }
        let panel = if self.navigation.show_tabs {
            self.render_navigation_tabs(has_other_window, window, cx)
        } else {
            self.navigation.tree.clone().into_any_element()
        };
        let sidebar = h_flex()
            .items_stretch()
            .size_full()
            .min_w_0()
            .min_h_0()
            .bg(ui_theme::header_material(&ui_theme::palette(cx)))
            .child(rail)
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .h_full()
                    .border_l_1()
                    .border_color(cx.theme().border)
                    .child(panel),
            );
        div()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .child(
                h_resizable("navigation-split")
                    .with_state(&self.navigation.split)
                    .child(
                        resizable_panel()
                            .size(window.rem_size() * 20.25)
                            .size_range(window.rem_size() * 10.25..window.rem_size() * 34.25)
                            .child(sidebar),
                    )
                    .child(resizable_panel().child(content)),
            )
            .into_any_element()
    }

    pub(super) fn navigation_panel_menu(
        menu: PopupMenu,
        workspace: &Entity<Self>,
        window: &mut Window,
        cx: &mut App,
    ) -> PopupMenu {
        window.focus(&menu.focus_handle(cx), cx);
        let vertical = workspace.read(cx).navigation.vertical_tabs;
        Self::popup_menu_with_workspace_action_context(menu, workspace, cx)
            .item(
                PopupMenuItem::new(crate::tr!("定位当前文件", "Reveal current file")).on_click(
                    window.listener_for(workspace, |this, _, _, cx| {
                        this.reveal_active_navigation_file(cx)
                    }),
                ),
            )
            .item(
                PopupMenuItem::new(crate::tr!("垂直标签页", "Vertical tabs"))
                    .checked(vertical)
                    .on_click(window.listener_for(workspace, |this, _, _, cx| {
                        this.navigation.vertical_tabs = !this.navigation.vertical_tabs;
                        this.navigation.show_tabs = this.navigation.vertical_tabs;
                        this.sync_navigation_file(cx);
                        cx.notify();
                    })),
            )
            .separator()
            .item(
                PopupMenuItem::new(crate::tr!("收起侧栏", "Hide sidebar")).on_click(
                    window.listener_for(workspace, |this, _, _, cx| {
                        this.toggle_navigation_sidebar(cx)
                    }),
                ),
            )
    }

    fn render_navigation_tabs(
        &mut self,
        has_other_window: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let active_ix = self.active_workspace_tab_ix();
        let indicator_y = self
            .navigation
            .indicator
            .sample_indicator(active_ix, window, cx);
        v_flex()
            .size_full()
            .min_h_0()
            .child(
                h_flex()
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
                            .child(crate::tr!("标签页", "Tabs")),
                    )
                    .child(
                        Button::new("sidebar-panel-menu")
                            .xsmall()
                            .ghost()
                            .icon(IconName::Ellipsis)
                            .tooltip(crate::tr!("侧边栏操作", "Sidebar actions"))
                            .dropdown_menu({
                                let workspace = cx.entity();
                                move |menu, window, cx| {
                                    Self::navigation_panel_menu(menu, &workspace, window, cx)
                                }
                            }),
                    ),
            )
            .child(
                uniform_list(
                    "vertical-document-tabs",
                    self.tabs.len() + 1,
                    cx.processor(move |this, range: Range<usize>, _, cx| {
                        range
                            .map(|ix| {
                                if ix == this.tabs.len() {
                                    return h_flex()
                                        .w_full()
                                        .h(VERTICAL_TAB_HEIGHT)
                                        .px(px(5.))
                                        .child(
                                            Button::new("vertical-new-tab")
                                                .small()
                                                .ghost()
                                                .w_full()
                                                .h(VERTICAL_TAB_INDICATOR_HEIGHT)
                                                .rounded(
                                                    (cx.theme().radius_lg - px(3.)).max(px(0.)),
                                                )
                                                .icon(IconName::Plus)
                                                .tooltip(crate::tr!("新建标签页", "New tab"))
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.create_new_tab(window, cx);
                                                })),
                                        )
                                        .into_any_element();
                                }
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
                                let presentation = this.workspace_tab_presentation(tab_id);
                                div()
                                    .relative()
                                    .w_full()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .h(VERTICAL_TAB_HEIGHT)
                                    .px(px(5.))
                                    .id(id.clone())
                                    .when_some(indicator_y, |row, y| {
                                        row.child(
                                            div()
                                                .absolute()
                                                .left(px(5.))
                                                .right(px(5.))
                                                .top(
                                                    y - VERTICAL_TAB_HEIGHT * ix as f32
                                                        + (VERTICAL_TAB_HEIGHT
                                                            - VERTICAL_TAB_INDICATOR_HEIGHT)
                                                            / 2.,
                                                )
                                                .h(VERTICAL_TAB_INDICATOR_HEIGHT)
                                                .bg(cx.theme().tokens.background)
                                                .rounded(
                                                    (cx.theme().radius_lg - px(3.)).max(px(0.)),
                                                )
                                                .shadow_sm(),
                                        )
                                    })
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
                                    .child(
                                        gpui_kit::base::Tab::new(id)
                                            .relative()
                                            .flex()
                                            .items_center()
                                            .w_full()
                                            .min_w_0()
                                            .h(VERTICAL_TAB_HEIGHT)
                                            .px(px(6.))
                                            .text_color(cx.theme().tab_foreground)
                                            .styles(|styles| {
                                                styles.selected(|style| {
                                                    style.text_color(
                                                        cx.theme().tab_active_foreground,
                                                    )
                                                })
                                            })
                                            .hover(|style| {
                                                style.text_color(cx.theme().tab_active_foreground)
                                            })
                                            .accessibility_label(title)
                                            .selected(this.active_tab_id == tab_id)
                                            .set_position(ix + 1, this.tabs.len())
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.activate_workspace_tab(tab_id, window, cx);
                                            }))
                                            .on_aux_click(cx.listener(
                                                move |this, event: &ClickEvent, window, cx| {
                                                    if event.is_middle_click() {
                                                        this.request_close_workspace_tabs(
                                                            BTreeSet::from([tab_id]),
                                                            window,
                                                            cx,
                                                        );
                                                    }
                                                },
                                            ))
                                            .child(
                                                Self::render_vertical_workspace_tab_contents(
                                                    tab_id,
                                                    &presentation,
                                                    cx,
                                                )
                                                .w_full(),
                                            ),
                                    )
                                    .into_any_element()
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
