use super::*;
use gpui_kit::component::{
    list::ListItem,
    tree::{Tree, TreeEvent, TreeItem, TreeState},
};
use vclogg_core::{CancellationToken, DirectoryEntry, navigation_directory, navigation_roots};

pub(super) struct FileTree {
    workspace: WeakEntity<Workspace>,
    tree: Entity<TreeState>,
    root: Option<PathBuf>,
    roots: Vec<PathBuf>,
    _roots_job: Task<()>,
    selected: Option<PathBuf>,
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
            }
            TreeEvent::Collapsed(id) => {
                this.expanded.remove(&decode_persisted_path(id));
            }
        });
        let roots_job = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async { navigation_roots(&CancellationToken::default()) })
                .await;
            _ = this.update(cx, |this, cx| {
                match result {
                    Ok(Some(roots)) => this.roots = roots,
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
            root: None,
            roots: Vec::new(),
            _roots_job: roots_job,
            selected: None,
            expanded: BTreeSet::new(),
            directories: BTreeMap::new(),
            jobs: BTreeMap::new(),
            errors: BTreeMap::new(),
            generation: 0,
            _subscription: subscription,
        }
    }

    pub(super) fn reveal(&mut self, path: PathBuf, directory: bool, cx: &mut Context<Self>) {
        let path = std::path::absolute(&path).unwrap_or(path);
        let root = if directory {
            path.clone()
        } else {
            path.parent().unwrap_or(Path::new(".")).to_path_buf()
        };
        if self
            .root
            .as_ref()
            .is_none_or(|existing| !path.starts_with(existing))
        {
            for (cancel, _) in self.jobs.values() {
                cancel.cancel();
            }
            self.jobs.clear();
            self.generation += 1;
            self.directories.clear();
            self.expanded.clear();
            self.errors.clear();
            self.root = Some(root.clone());
        }
        self.selected = Some(path.clone());
        let root = self.root.clone().unwrap_or(root);
        let ancestors = path
            .ancestors()
            .skip(usize::from(!directory))
            .take_while(|ancestor| ancestor.starts_with(&root))
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
            let children = match self.directories.get(path) {
                Some(entries) if !entries.is_empty() => entries
                    .iter()
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
            item = item.children(children);
        }
        item
    }

    fn rebuild(&mut self, cx: &mut Context<Self>) {
        let items = self
            .root
            .as_ref()
            .map(|root| vec![self.item(root, true)])
            .unwrap_or_else(|| {
                self.roots
                    .iter()
                    .map(|root| self.item(root, true))
                    .collect()
            });
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
        self.tree.update(cx, |tree, cx| {
            tree.set_items(items, cx);
            if let Some(ix) = selected.as_ref().and_then(|id| tree.index_of(id)) {
                tree.set_selected_index(Some(ix), cx);
                if pending_reveal {
                    tree.scroll_to_item(ix, ScrollStrategy::Nearest);
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let workspace_current = self.workspace.clone();
        let workspace = self.workspace.clone();
        let workspace_keyboard = workspace.clone();
        v_flex()
            .size_full()
            .min_h_0()
            .child(
                h_flex()
                    .gap_1()
                    .p_1()
                    .child(
                        Button::new("tree-devices")
                            .small()
                            .ghost()
                            .icon(IconName::Folder)
                            .tooltip(crate::tr!("设备与根目录", "Devices and roots"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.root = None;
                                this.selected = None;
                                this.rebuild(cx);
                            })),
                    )
                    .child(
                        Button::new("tree-parent")
                            .small()
                            .ghost()
                            .icon(IconName::ChevronUp)
                            .tooltip(crate::tr!("上级文件夹", "Parent folder"))
                            .disabled(self.root.as_ref().and_then(|root| root.parent()).is_none())
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(parent) = this
                                    .root
                                    .as_ref()
                                    .and_then(|root| root.parent())
                                    .map(Path::to_path_buf)
                                {
                                    this.reveal(parent, true, cx);
                                }
                            })),
                    )
                    .child(
                        Button::new("tree-refresh")
                            .small()
                            .ghost()
                            .icon(crate::app_assets::AppIcon::Refresh)
                            .tooltip(crate::tr!("刷新文件树", "Refresh file tree"))
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
                        Button::new("tree-current")
                            .small()
                            .ghost()
                            .label(crate::tr!("当前文件", "Current file"))
                            .on_click(move |_, _, cx| {
                                _ = workspace_current
                                    .update(cx, |this, cx| this.reveal_active_navigation_file(cx));
                            }),
                    ),
            )
            .when(self.root.is_none() && self.roots.is_empty(), |view| {
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
                div()
                    .flex_1()
                    .min_h_0()
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
                    .child(Tree::new(&self.tree, move |ix, entry, _, _, _| {
                        let path = decode_persisted_path(&entry.item().id);
                        let directory = entry.is_folder();
                        let workspace = workspace.clone();
                        ListItem::new(ix)
                            .child(
                                h_flex()
                                    .gap_1()
                                    .pl(rems(entry.depth() as f32))
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
                                        .xsmall(),
                                    )
                                    .child(entry.item().label.clone()),
                            )
                            .on_click(move |_, window, cx| {
                                if !directory {
                                    _ = workspace.update(cx, |workspace, cx| {
                                        workspace.begin_open_paths(vec![path.clone()], window, cx)
                                    });
                                }
                            })
                    })),
            )
    }
}
