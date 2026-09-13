//! Retained sidebar tools; document ownership stays with Workspace.
use super::*;
use gpui::{EventEmitter, Hsla};
use gpui_component::{
    list::ListItem,
    resizable::h_resizable,
    tree::{Tree, TreeEvent, TreeItem, TreeState},
};
use std::sync::atomic::AtomicUsize;
use vclogg_core::{CancellationToken, DirectoryEntry, NavigationSummary};

mod layout;
mod minimap;
mod overview;
mod tasks;
mod views;
use layout::{
    DraggedSidebarPanel, SIDEBAR_MAX_WIDTH_REM, SIDEBAR_MIN_WIDTH_REM, SIDEBAR_RAIL_WIDTH_REM,
    SidebarLayout, SidebarPanelId, SidebarSide,
};

pub(super) struct SidebarChanged;

struct SidebarJob {
    cancellation: CancellationToken,
    _task: Task<()>,
}
impl Drop for SidebarJob {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

#[derive(Clone)]
struct ColorGroup {
    id: String,
    label: String,
    color: u32,
    alpha: u8,
    rows: CompressedRows,
}

#[derive(Clone)]
enum DirectoryLoad {
    Loading,
    Ready(Arc<Vec<DirectoryEntry>>),
    Failed(String),
}

pub(super) struct SidebarState {
    workspace: WeakEntity<Workspace>,
    layout: SidebarLayout,
    layout_loaded: bool,
    layout_modified: bool,
    layout_revision: u64,
    store: Option<Arc<StateStore>>,
    persistence_task: Option<Task<()>>,
    persistence_error: Option<String>,
    document: Option<(u64, Arc<LogDocument>)>,
    viewport: Option<VirtualLogViewport<LogRowKey>>,
    table_subscription: Option<Subscription>,
    summary: Option<Arc<NavigationSummary>>,
    summary_source: Option<Arc<LogDocument>>,
    overview_marks: Vec<Option<usize>>,
    overview_job: Option<SidebarJob>,
    overview_request: u64,
    overview_dirty: bool,
    append_pair: Option<(Arc<LogDocument>, Arc<LogDocument>)>,
    summary_job: Option<SidebarJob>,
    summary_request: u64,
    summary_error: Option<String>,
    progress: Arc<AtomicUsize>,
    progress_task: Option<Task<()>>,
    generation: u64,
    tree: Entity<TreeState>,
    root: Option<PathBuf>,
    directories: BTreeMap<PathBuf, DirectoryLoad>,
    directory_jobs: BTreeMap<PathBuf, SidebarJob>,
    expanded: BTreeSet<PathBuf>,
    tree_generation: u64,
    tree_reveal_active: bool,
    tree_job: Option<SidebarJob>,
    tree_request: u64,
    tree_dirty: bool,
    favorites: Vec<RecentFile>,
    history: Vec<crate::state_store::HistorySession>,
    history_signature: Vec<(PathBuf, i64)>,
    history_task: Option<Task<()>>,
    history_request: u64,
    history_error: Option<String>,
    rules: Arc<Vec<KeywordColorRule>>,
    rules_version: Option<Arc<ResolvedColorRules>>,
    labels: Vec<ColorLabel>,
    colors: Arc<Vec<ColorGroup>>,
    collapsed_colors: BTreeSet<String>,
    color_job: Option<SidebarJob>,
    color_request: u64,
    color_revision: u64,
    color_error: Option<String>,
    colors_loaded: bool,
    previews: BTreeMap<usize, String>,
    preview_job: Option<SidebarJob>,
    preview_request: u64,
    preview_range: Option<Range<usize>>,
    color_visible_start: usize,
    scrolls: BTreeMap<SidebarPanelId, UniformListScrollHandle>,
    focus: BTreeMap<SidebarPanelId, FocusHandle>,
    selected: BTreeMap<SidebarPanelId, String>,
    minimap_drag: Option<f32>,
    minimap_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<SidebarChanged> for SidebarState {}

impl SidebarState {
    pub(super) fn new(workspace: WeakEntity<Workspace>, cx: &mut Context<Self>) -> Self {
        let tree = cx.new(|cx| TreeState::new(cx));
        let owner = workspace.upgrade().expect("sidebar owner exists");
        let subscriptions = vec![
            cx.observe(&owner, |this, owner, cx| this.sync(&owner, cx)),
            cx.subscribe(&tree, |this, _, event: &TreeEvent, cx| {
                let (id, expanded) = match event {
                    TreeEvent::Expanded(id) => (id, true),
                    TreeEvent::Collapsed(id) => (id, false),
                };
                let path = decode_persisted_path(id.as_ref());
                if expanded {
                    this.expanded.insert(path.clone());
                    this.load_directory(path, cx);
                } else {
                    this.expanded.remove(&path);
                }
                this.rebuild_tree(cx);
            }),
        ];
        Self {
            workspace,
            layout: SidebarLayout::default(),
            layout_loaded: false,
            layout_modified: false,
            layout_revision: 0,
            store: None,
            persistence_task: None,
            persistence_error: None,
            document: None,
            viewport: None,
            table_subscription: None,
            summary: None,
            summary_source: None,
            overview_marks: Vec::new(),
            overview_job: None,
            overview_request: 0,
            overview_dirty: true,
            append_pair: None,
            summary_job: None,
            summary_request: 0,
            summary_error: None,
            progress: Arc::new(AtomicUsize::new(0)),
            progress_task: None,
            generation: 0,
            tree,
            root: None,
            directories: BTreeMap::new(),
            directory_jobs: BTreeMap::new(),
            expanded: BTreeSet::new(),
            tree_generation: 0,
            tree_reveal_active: true,
            tree_job: None,
            tree_request: 0,
            tree_dirty: true,
            favorites: Vec::new(),
            history: Vec::new(),
            history_signature: Vec::new(),
            history_task: None,
            history_request: 0,
            history_error: None,
            rules: Arc::default(),
            rules_version: None,
            labels: Vec::new(),
            colors: Arc::default(),
            collapsed_colors: BTreeSet::new(),
            color_job: None,
            color_request: 0,
            color_revision: 0,
            color_error: None,
            colors_loaded: false,
            previews: BTreeMap::new(),
            preview_job: None,
            preview_request: 0,
            preview_range: None,
            color_visible_start: 0,
            scrolls: SidebarPanelId::ALL
                .into_iter()
                .map(|id| (id, UniformListScrollHandle::new()))
                .collect(),
            focus: SidebarPanelId::ALL
                .into_iter()
                .map(|id| (id, cx.focus_handle().tab_stop(true)))
                .collect(),
            selected: BTreeMap::new(),
            minimap_drag: None,
            minimap_bounds: Rc::default(),
            _subscriptions: subscriptions,
        }
    }

    fn is_showing(&self, panel: SidebarPanelId) -> bool {
        self.layout
            .sides
            .iter()
            .any(|side| side.visible && side.active == Some(panel))
    }

    fn sync(&mut self, owner: &Entity<Workspace>, cx: &mut Context<Self>) {
        let workspace = owner.read(cx);
        let source = workspace
            .active_document()
            .map(|tab| (tab.id, tab.document.clone()));
        let changed = match (&source, &self.document) {
            (Some((id, document)), Some((old_id, old))) => {
                id != old_id || !Arc::ptr_eq(document, old)
            }
            (None, None) => false,
            _ => true,
        };
        let changed_file =
            source.as_ref().map(|(id, _)| *id) != self.document.as_ref().map(|(id, _)| *id);
        let rules = workspace
            .active_document()
            .map(|tab| &tab.file.keyword_color_rules);
        let rules_version = workspace
            .active_document()
            .map(|tab| tab.file.resolved_color_rules.clone());
        let colors_changed = changed
            || match (&rules_version, &self.rules_version) {
                (Some(new), Some(old)) => !Arc::ptr_eq(new, old),
                (None, None) => false,
                _ => true,
            }
            || self.labels != workspace.color_labels;
        if colors_changed {
            tasks::release_in_background(
                std::mem::replace(&mut self.rules_version, rules_version),
                cx,
            );
            let old = std::mem::replace(
                &mut self.rules,
                Arc::new(rules.cloned().unwrap_or_default()),
            );
            tasks::release_in_background(old, cx);
            self.labels = workspace.color_labels.clone();
        }
        self.favorites = workspace.pinned_files.clone();
        let signature = workspace
            .recent_files
            .iter()
            .map(|file| (file.path.clone(), file.last_opened_at))
            .collect::<Vec<_>>();
        let history_changed = signature != self.history_signature;
        self.history_signature = signature;
        let store = workspace.persistence.store.clone();
        let table = workspace.active_document().map(|tab| tab.log_table.clone());
        if changed {
            self.generation += 1;
            self.summary_job.take();
            self.progress_task.take();
            self.overview_job.take();
            self.color_job.take();
            self.preview_job.take();
            self.summary_error = None;
            self.color_error = None;
            self.document = source;
            if changed_file {
                self.tree_reveal_active = true;
                self.selected.remove(&SidebarPanelId::Minutes);
                self.selected.remove(&SidebarPanelId::Colors);
            }
            self.minimap_drag = None;
            self.viewport = table
                .as_ref()
                .map(|table| table.read(cx).viewport().clone());
            self.table_subscription = table
                .as_ref()
                .map(|table| cx.observe(table, |_, _, cx| cx.notify()));
            let root = self
                .document
                .as_ref()
                .and_then(|(_, document)| document.path().parent())
                .map(Path::to_path_buf);
            if root != self.root {
                self.root = root;
                self.tree_generation += 1;
                self.directory_jobs.clear();
                tasks::release_in_background(std::mem::take(&mut self.directories), cx);
                self.expanded.clear();
                if let Some(root) = &self.root {
                    self.expanded.insert(root.clone());
                }
                self.rebuild_tree(cx);
            } else {
                if changed_file && let Some(root) = &self.root {
                    self.expanded.insert(root.clone());
                }
                self.rebuild_tree(cx);
            }
            // Keep an old summary only for a verified append; never display it for a new file.
            let reusable = self.append_pair.as_ref().is_some_and(|(old, new)| {
                self.summary_source
                    .as_ref()
                    .is_some_and(|source| Arc::ptr_eq(source, old))
                    && self
                        .document
                        .as_ref()
                        .is_some_and(|(_, source)| Arc::ptr_eq(source, new))
            });
            if !reusable {
                tasks::release_in_background(self.summary.take(), cx);
                self.summary_source = None;
                self.overview_marks.clear();
                self.append_pair = None;
            }
        }
        if colors_changed {
            self.colors_loaded = false;
            self.color_revision += 1;
            self.color_job.take();
            tasks::release_in_background(std::mem::take(&mut self.colors), cx);
            self.overview_job.take();
            self.overview_dirty = true;
            self.overview_marks.clear();
            self.previews.clear();
            self.preview_range = None;
            self.preview_job.take();
            self.color_error = None;
        }
        if self.store.is_none()
            && let Some(store) = store
        {
            self.store = Some(store);
            self.load_layout(cx);
        }
        if history_changed && self.is_showing(SidebarPanelId::History) {
            self.refresh_history(cx);
        }
        self.ensure_visible_data(cx);
        cx.notify();
    }

    fn ensure_visible_data(&mut self, cx: &mut Context<Self>) {
        let need_summary =
            self.is_showing(SidebarPanelId::Minutes) || self.is_showing(SidebarPanelId::Minimap);
        let need_colors =
            self.is_showing(SidebarPanelId::Colors) || self.is_showing(SidebarPanelId::Minimap);
        if !need_summary {
            self.summary_job.take();
            self.progress_task.take();
        }
        if !self.is_showing(SidebarPanelId::Minimap) {
            self.overview_job.take();
        }
        if !need_colors {
            self.color_job.take();
            self.preview_job.take();
            self.preview_range = None;
        }
        if !self.is_showing(SidebarPanelId::Files) {
            self.tree_job.take();
            self.directory_jobs.clear();
            self.directories
                .retain(|_, load| !matches!(load, DirectoryLoad::Loading));
        }
        if !self.is_showing(SidebarPanelId::History) {
            self.history_task.take();
        }
        if self.is_showing(SidebarPanelId::Files)
            && let Some(root) = self.root.clone()
        {
            self.load_directory(root, cx);
            for path in self.expanded.iter().cloned().collect::<Vec<_>>() {
                self.load_directory(path, cx);
            }
            if self.tree_dirty && self.tree_job.is_none() {
                self.rebuild_tree(cx);
            }
        }
        if (self.is_showing(SidebarPanelId::Minutes) || self.is_showing(SidebarPanelId::Minimap))
            && self.summary_job.is_none()
            && self.summary_error.is_none()
            && self
                .document
                .as_ref()
                .is_some_and(|(_, document)| document.has_complete_line_index())
            && !self
                .summary_source
                .as_ref()
                .zip(self.document.as_ref())
                .is_some_and(|(old, (_, new))| Arc::ptr_eq(old, new))
        {
            self.start_summary(cx);
        }
        if (self.is_showing(SidebarPanelId::Colors) || self.is_showing(SidebarPanelId::Minimap))
            && self.color_job.is_none()
            && self.color_error.is_none()
            && !self.colors_loaded
            && !self.rules.is_empty()
        {
            self.start_colors(cx);
        }
        if self.overview_dirty && self.overview_job.is_none() {
            self.prepare_overview_marks(cx);
        }
    }

    fn changed_layout(&mut self, cx: &mut Context<Self>) {
        self.layout_modified = true;
        self.layout_revision = tasks::record_layout(&self.layout);
        self.save_layout(cx);
        self.ensure_visible_data(cx);
        cx.emit(SidebarChanged);
        cx.notify();
    }

    fn toggle(&mut self, side: SidebarSide, window: &mut Window, cx: &mut Context<Self>) {
        let placement = &mut self.layout.sides[side.ix()];
        placement.visible = !placement.visible;
        if self.is_showing(SidebarPanelId::History) {
            self.refresh_history(cx);
        }
        self.minimap_drag = None;
        self.changed_layout(cx);
        if self.layout.sides[side.ix()].visible {
            if let Some(panel) = self.layout.sides[side.ix()].active {
                self.focus_panel(panel, window, cx);
            }
        } else {
            _ = self.workspace.update(cx, |workspace, cx| {
                if workspace.active_document().is_some() {
                    workspace.log_viewer.focus_handle.focus(window, cx);
                } else {
                    workspace.focus_handle.focus(window, cx);
                }
            });
        }
    }

    fn focus_panel(&self, panel: SidebarPanelId, window: &mut Window, cx: &mut Context<Self>) {
        if panel == SidebarPanelId::Files
            && self.root.as_ref().is_some_and(|root| {
                !matches!(self.directories.get(root), Some(DirectoryLoad::Failed(_)))
            })
        {
            self.tree.update(cx, |tree, cx| tree.focus(window, cx));
        } else {
            self.focus[&panel].focus(window, cx);
        }
    }

    fn activate(
        &mut self,
        panel: SidebarPanelId,
        side: SidebarSide,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.layout.sides[side.ix()].active = Some(panel);
        self.layout.sides[side.ix()].visible = true;
        if panel == SidebarPanelId::History {
            self.refresh_history(cx);
        }
        self.changed_layout(cx);
        self.focus_panel(panel, window, cx);
    }

    fn drop_panel(
        &mut self,
        dragged: &DraggedSidebarPanel,
        side: SidebarSide,
        before: Option<SidebarPanelId>,
        cx: &mut Context<Self>,
    ) {
        if dragged.owner != cx.entity_id() {
            return;
        }
        self.layout.move_panel(dragged.panel, side, before);
        if dragged.panel == SidebarPanelId::History {
            self.refresh_history(cx);
        }
        self.changed_layout(cx);
    }

    fn jump(&self, row: usize, select: bool, window: &mut Window, cx: &mut App) {
        self.jump_fraction(row, 0., select, window, cx);
    }

    fn jump_fraction(
        &self,
        row: usize,
        fraction: f32,
        select: bool,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some((id, source)) = &self.document else {
            return;
        };
        let id = *id;
        let source = source.clone();
        _ = self.workspace.update(cx, |workspace, cx| {
            let Some(tab) = workspace.active_document() else {
                return;
            };
            if tab.id != id
                || !Arc::ptr_eq(&tab.document, &source)
                || !source.has_complete_line_index()
            {
                return;
            }
            let Some(local) = source.local_row(row) else {
                return;
            };
            let table = tab.log_table.clone();
            if let Some(tab) = workspace.documents.iter_mut().find(|tab| tab.id == id) {
                tab.view.auto_follow = false;
            }
            workspace
                .pending_log_scroll_frames
                .clear((id, WrappedRegion::Log));
            table.update(cx, |table, cx| {
                if select {
                    table.delegate().settle_table_selection(local);
                    table.set_active_log_row(local, cx);
                }
                let offset = table.viewport().row_height(local) * fraction.clamp(0., 0.9999);
                table.viewport().scroll_row_to_viewport_y(local, -offset);
                cx.notify();
            });
            if select {
                workspace.selected_source_row = Some(row);
                workspace.log_viewer.focus_handle.focus(window, cx);
            }
            workspace.remember_user_log_region(LogRegion::Body);
            cx.notify();
        });
    }
}

pub(super) struct SidebarSurface {
    state: Entity<SidebarState>,
    side: SidebarSide,
    _subscription: Subscription,
}
impl SidebarSurface {
    fn new(state: Entity<SidebarState>, side: SidebarSide, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&state, |_, _, cx| cx.notify());
        Self {
            state,
            side,
            _subscription: subscription,
        }
    }
}
impl Render for SidebarSurface {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Render Workspace-owned tabs before borrowing SidebarState for the shell.
        let tabs = {
            let state = self.state.read(cx);
            (state.layout.sides[self.side.ix()].active == Some(SidebarPanelId::Tabs)).then(|| {
                (
                    state.workspace.clone(),
                    state.focus[&SidebarPanelId::Tabs].clone(),
                )
            })
        };
        let tabs = tabs.and_then(|(workspace, focus)| {
            workspace
                .update(cx, |workspace, cx| {
                    workspace.render_vertical_tabs(focus, window, cx)
                })
                .ok()
        });
        self.state.update(cx, |state, cx| {
            state.render_side(self.side, tabs, window, cx)
        })
    }
}

impl Workspace {
    pub(super) fn vertical_tabs_enabled(&self, cx: &App) -> bool {
        self.sidebar.read(cx).layout.vertical_tabs
    }

    pub(super) fn toggle_vertical_tabs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sidebar.update(cx, |state, cx| {
            let enabled = !state.layout.vertical_tabs;
            state.layout.set_vertical_tabs(enabled);
            state.changed_layout(cx);
            if enabled {
                state.focus_panel(SidebarPanelId::Tabs, window, cx);
            }
        });
        self.vertical_tab_state.revealed.set(None);
        *self.tab_drop_layout.borrow_mut() = TabDropLayout::default();
        cx.notify();
    }

    pub(super) fn create_sidebars(
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (
        Entity<SidebarState>,
        [Entity<SidebarSurface>; 2],
        Vec<Subscription>,
    ) {
        let workspace = cx.weak_entity();
        let state = cx.new(|cx| SidebarState::new(workspace, cx));
        let surfaces = [SidebarSide::Left, SidebarSide::Right]
            .map(|side| cx.new(|cx| SidebarSurface::new(state.clone(), side, cx)));
        let subscriptions = vec![
            cx.subscribe(&state, |this, _, _: &SidebarChanged, cx| {
                this.reset_sidebar_split(cx)
            }),
            cx.observe_window_bounds(window, |this, window, cx| {
                let sidebar = this.sidebar.read(cx);
                let widths = sidebar
                    .layout
                    .fit(window.viewport_size().width / window.rem_size());
                let restore_focus = sidebar.layout.sides.iter().enumerate().any(|(ix, side)| {
                    widths[ix].is_none()
                        && side
                            .active
                            .is_some_and(|panel| sidebar.focus[&panel].contains_focused(window, cx))
                });
                if restore_focus {
                    this.log_viewer.focus_handle.focus(window, cx);
                }
                this.reset_sidebar_split(cx);
            }),
            cx.observe_global::<gpui_component::Theme>(|this, cx| this.reset_sidebar_split(cx)),
        ];
        (state, surfaces, subscriptions)
    }

    fn reset_sidebar_split(&mut self, cx: &mut Context<Self>) {
        self.sidebar_split
            .update(cx, |split, _| *split = ResizableState::default());
        cx.notify();
    }

    pub(super) fn refresh_sidebar_history(&mut self, cx: &mut Context<Self>) {
        self.sidebar.update(cx, |state, cx| {
            if state.is_showing(SidebarPanelId::History) {
                state.refresh_history(cx);
            }
        });
    }

    pub(super) fn note_sidebar_append(
        &mut self,
        old: Arc<LogDocument>,
        new: Arc<LogDocument>,
        cx: &mut Context<Self>,
    ) {
        self.sidebar
            .update(cx, |state, _| state.append_pair = Some((old, new)));
    }

    pub(super) fn sidebar_toggle_button(
        &self,
        right: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let side = if right {
            SidebarSide::Right
        } else {
            SidebarSide::Left
        };
        let state = self.sidebar.clone();
        let owner = state.entity_id();
        let visible = state.read(cx).layout.sides[side.ix()].visible;
        div()
            .id(if right {
                "right-sidebar-toggle-drop"
            } else {
                "left-sidebar-toggle-drop"
            })
            .flex_shrink_0()
            .drag_over::<DraggedSidebarPanel>(move |this, drag, _, cx| {
                if drag.owner == owner {
                    this.bg(cx.theme().accent)
                } else {
                    this
                }
            })
            .on_drop({
                let state = state.clone();
                move |drag: &DraggedSidebarPanel, _, cx| {
                    state.update(cx, |state, cx| state.drop_panel(drag, side, None, cx))
                }
            })
            .child(
                Button::new(if right {
                    "toggle-right-sidebar"
                } else {
                    "toggle-left-sidebar"
                })
                .ghost()
                .icon(if right {
                    IconName::PanelRight
                } else {
                    IconName::PanelLeft
                })
                .selected(visible)
                .tooltip(if right {
                    crate::tr!("打开或关闭右侧边栏", "Toggle right sidebar")
                } else {
                    crate::tr!("打开或关闭左侧边栏", "Toggle left sidebar")
                })
                .on_click(move |_, window, cx| {
                    state.update(cx, |state, cx| state.toggle(side, window, cx))
                }),
            )
    }

    pub(super) fn render_sidebar_workspace(
        &mut self,
        has_other_window: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let rem = window.rem_size();
        let widths = self
            .sidebar
            .read(cx)
            .layout
            .fit(window.viewport_size().width / rem);
        let state = self.sidebar.clone();
        let vertical_visible = widths.iter().enumerate().any(|(ix, width)| {
            width.is_some()
                && self.sidebar.read(cx).layout.sides[ix].active == Some(SidebarPanelId::Tabs)
        });
        let center = v_flex()
            .min_w_0()
            .min_h_0()
            .size_full()
            .when(!vertical_visible, |this| {
                this.child(self.render_tabs(has_other_window, cx))
            })
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_tab_workspace(window, cx)),
            );
        h_resizable("workspace-sidebars")
            .with_state(&self.sidebar_split)
            .on_resize(move |sizes, window, cx| {
                let sizes = sizes.read(cx).sizes().clone();
                let mut ix = 0;
                let left = widths[0].and_then(|_| {
                    let width = sizes.get(ix).copied();
                    ix += 1;
                    width
                });
                ix += 1;
                let right = widths[1].and_then(|_| sizes.get(ix).copied());
                state.update(cx, |state, cx| {
                    let mut changed = false;
                    for (side, measured) in [left, right].into_iter().enumerate() {
                        if let Some(width) = measured {
                            let width = (width / window.rem_size() - SIDEBAR_RAIL_WIDTH_REM)
                                .clamp(SIDEBAR_MIN_WIDTH_REM, SIDEBAR_MAX_WIDTH_REM);
                            // Only persist a divider's explicit change, not another pane's temporary fit.
                            if widths[side].is_some_and(|shown| (shown - width).abs() > 0.05) {
                                state.layout.sides[side].width = width;
                                changed = true;
                            }
                        }
                    }
                    if changed {
                        state.changed_layout(cx);
                    }
                });
            })
            .when_some(widths[0], |split, width| {
                split.child(
                    resizable_panel()
                        .size(rem * (width + SIDEBAR_RAIL_WIDTH_REM))
                        .flex_none()
                        .size_range(
                            rem * (SIDEBAR_MIN_WIDTH_REM + SIDEBAR_RAIL_WIDTH_REM)
                                ..rem * (SIDEBAR_MAX_WIDTH_REM + SIDEBAR_RAIL_WIDTH_REM),
                        )
                        .child(self.sidebar_surfaces[0].clone()),
                )
            })
            .child(
                resizable_panel()
                    .size_range((rem * 24.).min(window.viewport_size().width)..Pixels::MAX)
                    .child(center),
            )
            .when_some(widths[1], |split, width| {
                split.child(
                    resizable_panel()
                        .size(rem * (width + SIDEBAR_RAIL_WIDTH_REM))
                        .flex_none()
                        .size_range(
                            rem * (SIDEBAR_MIN_WIDTH_REM + SIDEBAR_RAIL_WIDTH_REM)
                                ..rem * (SIDEBAR_MAX_WIDTH_REM + SIDEBAR_RAIL_WIDTH_REM),
                        )
                        .child(self.sidebar_surfaces[1].clone()),
                )
            })
    }
}
