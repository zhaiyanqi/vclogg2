use super::*;
use gpui_kit::component::{
    list::ListItem,
    tree::{Tree, TreeEvent, TreeItem, TreeState},
};
use vclogg_core::{CancellationToken, DirectoryEntry, navigation_directory, navigation_roots};

pub(super) struct FileTree {
    workspace: WeakEntity<Workspace>,
    tree: Entity<TreeState>,
    roots: Vec<PathBuf>,
    horizontal_scroll: ScrollHandle,
    width_rem: f32,
    width_job: Option<Task<()>>,
    width_revision: u64,
    _roots_job: Task<()>,
    selected: Option<PathBuf>,
    reveal_strategy: ScrollStrategy,
    revealed_path: Option<PathBuf>,
    expanded: BTreeSet<PathBuf>,
    directories: BTreeMap<PathBuf, Vec<DirectoryEntry>>,
    jobs: BTreeMap<PathBuf, (CancellationToken, Task<()>)>,
    errors: BTreeMap<PathBuf, String>,
    generation: u64,
    _subscription: Subscription,
}

impl Drop for FileTree {
    fn drop(&mut self) {
        for (cancel, _) in self.jobs.values() {
            cancel.cancel();
        }
    }
}

impl FileTree {
    pub(super) fn new(workspace: WeakEntity<Workspace>, cx: &mut Context<Self>) -> Self {
        let tree = cx.new(|cx| TreeState::new(cx));
        let subscription = cx.subscribe(&tree, |this, _, event: &TreeEvent, cx| match event {
            TreeEvent::Expanded(id) => {
                let path = decode_persisted_path(id);
                this.expanded.insert(path.clone());
                this.load(path, cx);
                this.rebuild(cx);
            }
            TreeEvent::Collapsed(id) => {
                this.expanded.remove(&decode_persisted_path(id));
                this.rebuild(cx);
            }
        });
        let roots_job = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async { navigation_roots(&CancellationToken::default()) })
                .await;
            _ = this.update(cx, |this, cx| {
                match result {
                    Ok(Some(roots)) => {
                        for root in roots {
                            if !this.roots.contains(&root) {
                                this.roots.push(root);
                            }
                        }
                        this.roots.sort();
                    }
                    Ok(None) => {}
                    Err(error) => {
                        this.errors.insert(PathBuf::new(), error.to_string());
                    }
                }
                this.rebuild(cx);
            });
        });
        Self {
            workspace,
            tree,
            roots: Vec::new(),
            horizontal_scroll: ScrollHandle::new(),
            width_rem: 0.,
            width_job: None,
            width_revision: 0,
            _roots_job: roots_job,
            selected: None,
            reveal_strategy: ScrollStrategy::Nearest,
            revealed_path: None,
            expanded: BTreeSet::new(),
            directories: BTreeMap::new(),
            jobs: BTreeMap::new(),
            errors: BTreeMap::new(),
            generation: 0,
            _subscription: subscription,
        }
    }

    pub(super) fn reveal(
        &mut self,
        path: PathBuf,
        directory: bool,
        strategy: ScrollStrategy,
        cx: &mut Context<Self>,
    ) {
        let path = std::path::absolute(&path).unwrap_or(path);
        if let Some(root) = path.ancestors().find(|path| path.parent().is_none()) {
            let root = root.to_path_buf();
            if !self.roots.contains(&root) {
                self.roots.push(root);
                self.roots.sort();
            }
        }
        self.selected = Some(path.clone());
        self.reveal_strategy = strategy;
        self.revealed_path = Some(path.clone());
        let ancestors = path
            .ancestors()
            .skip(usize::from(!directory))
            .filter(|ancestor| !ancestor.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .collect::<Vec<_>>();
        for ancestor in ancestors {
            self.expanded.insert(ancestor.clone());
            self.load(ancestor, cx);
        }
        self.rebuild(cx);
    }

    fn load(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if self.jobs.contains_key(&path) || self.directories.contains_key(&path) {
            return;
        }
        self.errors.remove(&path);
        let generation = self.generation;
        let cancellation = CancellationToken::default();
        let worker = cancellation.clone();
        let loading_path = path.clone();
        let task = cx.spawn(async move |this, cx| {
            let read_path = loading_path.clone();
            let result = cx
                .background_spawn(async move { navigation_directory(&read_path, &worker) })
                .await;
            _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.jobs.remove(&loading_path);
                match result {
                    Ok(Some(entries)) => {
                        this.directories.insert(loading_path, entries);
                    }
                    Ok(None) => return,
                    Err(error) => {
                        this.errors.insert(loading_path, error.to_string());
                    }
                }
                this.rebuild(cx);
            });
        });
        self.jobs.insert(path, (cancellation, task));
    }

    fn item(&self, path: &Path, directory: bool) -> TreeItem {
        let id = encode_persisted_path(path);
        let label = path.file_name().map_or_else(
            || path.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        let mut item = TreeItem::new(id.clone(), label).expanded(self.expanded.contains(path));
        if directory {
            let mut children: Vec<TreeItem> = match self.directories.get(path) {
                Some(entries) if !entries.is_empty() => entries
                    .iter()
                    .filter(|entry| {
                        !entry.is_hidden()
                            || self
                                .revealed_path
                                .as_ref()
                                .is_some_and(|path| path.starts_with(entry.path()))
                    })
                    .map(|entry| self.item(entry.path(), entry.is_directory()))
                    .collect(),
                _ => vec![
                    TreeItem::new(
                        format!("placeholder:{id}"),
                        self.errors.get(path).cloned().unwrap_or_else(|| {
                            if self.directories.contains_key(path) {
                                crate::tr!("空文件夹", "Empty folder").into()
                            } else {
                                crate::tr!("正在读取…", "Loading…").into()
                            }
                        }),
                    )
                    .disabled(true),
                ],
            };
            if children.is_empty() {
                children.push(
                    TreeItem::new(
                        format!("placeholder:{id}"),
                        crate::tr!("空文件夹", "Empty folder"),
                    )
                    .disabled(true),
                );
            }
            item = item.children(children);
        }
        item
    }

    fn measure_width(&mut self, items: &[TreeItem], cx: &mut Context<Self>) {
        let mut pending = items.iter().map(|item| (item, 0usize)).collect::<Vec<_>>();
        let mut labels = Vec::new();
        while let Some((item, depth)) = pending.pop() {
            labels.push((
                item.label
                    .replace('\n', "\\n")
                    .replace('\r', "\\r")
                    .replace('\t', "\\t"),
                depth,
            ));
            if item.is_expanded() {
                pending.extend(item.children.iter().map(|item| (item, depth + 1)));
            }
        }
        self.width_revision += 1;
        let revision = self.width_revision;
        let text_system = cx.text_system().clone();
        let font = gpui_kit::font(cx.theme().font_family.clone());
        let rem = cx.theme().font_size;
        self.width_job = Some(cx.spawn(async move |this, cx| {
            let width = cx
                .background_spawn(async move {
                    let text_system = gpui_kit::WindowTextSystem::new(text_system);
                    labels.into_iter().fold(0f32, |width, (label, depth)| {
                        let run = gpui_kit::TextRun {
                            len: label.len(),
                            font: font.clone(),
                            ..Default::default()
                        };
                        let text_width = text_system
                            .layout_line(&label, rem * 0.875, &[run], None)
                            .width;
                        width.max(text_width / rem + depth as f32 + 1. + 0.875 + 0.25)
                    })
                })
                .await;
            _ = this.update(cx, |this, cx| {
                if this.width_revision == revision {
                    this.width_rem = width;
                    this.width_job = None;
                    cx.notify();
                }
            });
        }));
    }

    fn rebuild(&mut self, cx: &mut Context<Self>) {
        let items = self
            .roots
            .iter()
            .map(|root| self.item(root, true))
            .collect::<Vec<_>>();
        self.measure_width(&items, cx);
        let pending_reveal = self.selected.is_some();
        let selected = self
            .selected
            .as_ref()
            .map(|path| SharedString::from(encode_persisted_path(path)))
            .or_else(|| {
                self.tree
                    .read(cx)
                    .selected_item()
                    .map(|item| item.id.clone())
            });
        let mut revealed = false;
        let strategy = self.reveal_strategy;
        self.tree.update(cx, |tree, cx| {
            tree.set_items(items, cx);
            if let Some(ix) = selected.as_ref().and_then(|id| tree.index_of(id)) {
                tree.set_selected_index(Some(ix), cx);
                if pending_reveal {
                    if strategy == ScrollStrategy::Center {
                        // Explicit navigation centers even an already visible row.
                        tree.scroll_handle().scroll_to_item_strict(ix, strategy);
                    } else {
                        // Opening a visible tree row must preserve its position.
                        tree.scroll_to_item(ix, strategy);
                    }
                    revealed = true;
                }
            }
        });
        if revealed {
            self.selected = None;
        }
        cx.notify();
    }
}

impl Render for FileTree {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = cx.entity();
        let workspace_current = self.workspace.clone();
        let workspace = self.workspace.clone();
        let workspace_keyboard = workspace.clone();
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
                            .child(crate::tr!("文件", "Files")),
                    )
                    .child(
                        Button::new("sidebar-files-refresh")
                            .xsmall()
                            .ghost()
                            .icon(crate::app_assets::AppIcon::Refresh)
                            .tooltip(crate::tr!("刷新文件夹", "Refresh folders"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                for (cancel, _) in this.jobs.values() {
                                    cancel.cancel();
                                }
                                this.jobs.clear();
                                this.generation += 1;
                                this.directories.clear();
                                for path in this.expanded.clone() {
                                    this.load(path, cx);
                                }
                                this.rebuild(cx);
                            })),
                    )
                    .child(
                        Button::new("sidebar-files-collapse")
                            .xsmall()
                            .ghost()
                            .icon(IconName::ChevronUp)
                            .tooltip(crate::tr!("折叠文件夹", "Collapse folders"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.selected = None;
                                this.expanded.clear();
                                this.rebuild(cx);
                            })),
                    )
                    .child(
                        Button::new("sidebar-panel-menu")
                            .xsmall()
                            .ghost()
                            .icon(IconName::Ellipsis)
                            .tooltip(crate::tr!("侧边栏操作", "Sidebar actions"))
                            .dropdown_menu(move |menu, window, cx| {
                                let Some(workspace) = workspace_current.upgrade() else {
                                    return menu;
                                };
                                Workspace::navigation_panel_menu(menu, &workspace, window, cx)
                            }),
                    ),
            )
            .when(self.roots.is_empty(), |view| {
                view.child(
                    div()
                        .p_2()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(
                            self.errors
                                .get(&PathBuf::new())
                                .cloned()
                                .unwrap_or_else(|| {
                                    crate::tr!("正在读取设备…", "Loading devices…").into()
                                }),
                        ),
                )
            })
            .child(
                // The scroll viewport grows inside this column. A block parent
                // leaves its flex-only height unresolved and the tree paints no rows.
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .capture_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                        if event.keystroke.key != "enter" {
                            return;
                        }
                        if let Some(entry) = this
                            .tree
                            .read(cx)
                            .selected_entry()
                            .filter(|entry| !entry.is_folder() && !entry.is_disabled())
                        {
                            let path = decode_persisted_path(&entry.item().id);
                            cx.stop_propagation();
                            _ = workspace_keyboard.update(cx, |workspace, cx| {
                                workspace.begin_open_paths(vec![path], window, cx)
                            });
                        }
                    }))
                    .child(file_tree_scroll_view(
                        Tree::new(&self.tree, move |_, entry, selected, _, _| {
                            let path = decode_persisted_path(&entry.item().id);
                            let directory = entry.is_folder();
                            let disabled = entry.is_disabled();
                            let state = state.clone();
                            let workspace = workspace.clone();
                            ListItem::new(entry.item().id.clone())
                                .selected(selected)
                                .w_full()
                                .h(rems(2.))
                                .px(rems(0.5))
                                .pl(rems(0.5 + entry.depth() as f32))
                                .child(
                                    h_flex()
                                        .flex_shrink_0()
                                        .gap(rems(0.25))
                                        .child(
                                            Icon::new(if directory {
                                                if entry.is_expanded() {
                                                    IconName::ChevronDown
                                                } else {
                                                    IconName::ChevronRight
                                                }
                                            } else {
                                                IconName::File
                                            })
                                            .size(rems(0.875)),
                                        )
                                        .child(
                                            div()
                                                .flex_shrink_0()
                                                .whitespace_nowrap()
                                                .text_size(rems(0.875))
                                                .child(
                                                    entry
                                                        .item()
                                                        .label
                                                        .replace('\n', "\\n")
                                                        .replace('\r', "\\r")
                                                        .replace('\t', "\\t"),
                                                ),
                                        ),
                                )
                                .on_click(move |event, window, cx| {
                                    if disabled {
                                        return;
                                    }
                                    state.update(cx, |this, _| this.selected = None);
                                    if !directory && event.click_count() >= 2 {
                                        _ = workspace.update(cx, |workspace, cx| {
                                            workspace.begin_open_paths(
                                                vec![path.clone()],
                                                window,
                                                cx,
                                            )
                                        });
                                    }
                                })
                        }),
                        window.rem_size() * self.width_rem,
                        &self.horizontal_scroll,
                    )),
            )
    }
}

fn file_tree_scroll_view(
    tree: Tree,
    tree_width: Pixels,
    horizontal_scroll: &ScrollHandle,
) -> impl IntoElement {
    use gpui_kit::component::scroll::ScrollbarMode;

    let wheel_scroll = horizontal_scroll.clone();
    div()
        .id("sidebar-file-tree-horizontal")
        .relative()
        .flex_1()
        .min_h_0()
        .min_w_0()
        .child(
            canvas(
                |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
                move |_, hitbox, window, _| {
                    // Capture before the nested tree handles the same wheel vertically.
                    window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
                        if !phase.capture()
                            || !event.modifiers.shift
                            || !hitbox.should_handle_scroll(window)
                        {
                            return;
                        }
                        let delta = event.delta.pixel_delta(window.line_height());
                        // Some platforms already translate Shift+wheel onto the x axis.
                        let horizontal_delta = if delta.x != px(0.) { delta.x } else { delta.y };
                        let mut offset = wheel_scroll.offset();
                        offset.x = (offset.x + horizontal_delta)
                            .clamp(-wheel_scroll.max_offset().x, px(0.));
                        wheel_scroll.set_offset(offset);
                        window.refresh();
                        // Consume even at an edge: Shift must never scroll the tree vertically.
                        cx.stop_propagation();
                    });
                },
            )
            .absolute()
            .size_full(),
        )
        .child(
            div()
                .id("sidebar-file-tree-scroll-area")
                .absolute()
                .inset_0()
                .overflow_x_scroll()
                .restrict_scroll_to_axis()
                .track_scroll(horizontal_scroll)
                .child(tree.w(tree_width).min_w_full()),
        )
        .child(
            div().absolute().inset_0().child(
                Scrollbar::horizontal(horizontal_scroll)
                    .id("sidebar-file-tree-horizontal-scrollbar")
                    .mode(ScrollbarMode::Always)
                    .viewport_from_layout(),
            ),
        )
}
