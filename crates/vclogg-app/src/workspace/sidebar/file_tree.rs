//! Window-lived directory snapshots and explicit file-tree navigation.
use super::*;
use tasks::release_in_background;

pub(super) const TREE_TEXT_REM: f32 = 0.875;
pub(super) const TREE_ICON_REM: f32 = 0.875;
pub(super) const TREE_GAP_REM: f32 = 0.25;
pub(super) const TREE_PADDING_REM: f32 = 0.5;

pub(super) fn tree_label(label: &str) -> String {
    label
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

// Keep directory snapshots Send so enumeration and text measurement can run
// off the UI thread. GPUI's public TreeItem holds UI-thread-only state.
struct FileTreeNode {
    id: String,
    label: String,
    children: Vec<Self>,
    expanded: bool,
    disabled: bool,
}

impl FileTreeNode {
    fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            children: Vec::new(),
            expanded: false,
            disabled: false,
        }
    }

    fn children(mut self, children: Vec<Self>) -> Self {
        self.children = children;
        self
    }

    fn expanded(mut self, expanded: bool) -> Self {
        self.expanded = expanded;
        self
    }

    fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    fn into_tree_item(self) -> TreeItem {
        TreeItem::new(self.id, self.label)
            .children(self.children.into_iter().map(Self::into_tree_item))
            .expanded(self.expanded)
            .disabled(self.disabled)
    }
}

#[derive(Clone)]
pub(super) struct FileTreeTarget {
    path: PathBuf,
}

impl SidebarState {
    pub(super) fn reveal_active_file(&mut self, cx: &mut Context<Self>) {
        if let Some(path) = self
            .document
            .as_ref()
            .map(|(_, doc)| doc.path().to_path_buf())
        {
            self.reveal_tree_path(path, false, cx);
        } else {
            self.tree_target = None;
            self.tree_reveal_active = false;
            self.tree_error = None;
            self.rebuild_tree(cx);
        }
    }

    fn reveal_tree_path(&mut self, path: PathBuf, directory: bool, cx: &mut Context<Self>) {
        let path = std::path::absolute(&path).unwrap_or(path);
        let id = SharedString::from(encode_persisted_path(&path));
        self.tree_target = Some(FileTreeTarget { path: path.clone() });
        self.tree_reveal_active = true;
        self.tree_retried.clear();
        self.tree_error = None;
        // Hidden tab switches only replace the intent. Do not accumulate paths
        // for tabs that have never actually been shown in the file tree.
        if !self.is_showing(SidebarPanelId::Files) {
            return;
        }
        let mut structure_changed = false;
        if let Some(root) = path.ancestors().find(|path| path.parent().is_none())
            && !self.roots.iter().any(|known| known == root)
        {
            self.roots.push(root.to_path_buf());
            structure_changed = true;
        }
        for ancestor in path.ancestors().skip(usize::from(!directory)) {
            if !ancestor.as_os_str().is_empty() {
                structure_changed |= self.expanded.insert(ancestor.to_path_buf());
            }
        }
        // The common tab-switch path does not allocate a prepared tree or read a directory.
        if !structure_changed
            && !self.tree_dirty
            && self.hidden_tree_paths() == self.tree_exceptions
            && self.is_showing(SidebarPanelId::Files)
            && let Some(ix) = self.tree.read(cx).index_of(&id)
        {
            self.tree.update(cx, |tree, cx| {
                tree.set_selected_index(Some(ix), cx);
                tree.scroll_to_item(ix, ScrollStrategy::Nearest);
            });
            self.tree_reveal_active = false;
            cx.notify();
            return;
        }
        // A new target can expose a previously filtered hidden path.
        self.rebuild_tree(cx);
        if self.is_showing(SidebarPanelId::Files) {
            self.load_expanded_directories(cx);
        }
    }

    pub(super) fn ensure_file_tree(&mut self, cx: &mut Context<Self>) {
        let visible = self.is_showing(SidebarPanelId::Files);
        let became_visible = visible && !self.tree_visible;
        self.tree_visible = visible;
        if !visible {
            if self.tree_job.take().is_some() {
                self.tree_request += 1;
                self.tree_dirty = true;
            }
            return;
        }
        if became_visible {
            self.reveal_active_file(cx);
        }
        if !self.roots_loaded && self.roots_job.is_none() && self.roots_error.is_none() {
            self.load_roots(cx);
        }
        if became_visible {
            self.load_expanded_directories(cx);
        }
        if self.tree_dirty && self.tree_job.is_none() {
            self.rebuild_tree(cx);
        }
    }

    fn load_roots(&mut self, cx: &mut Context<Self>) {
        self.roots_job.take();
        self.roots_error = None;
        let generation = self.tree_generation;
        let cancellation = CancellationToken::default();
        let worker = cancellation.clone();
        let task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { vclogg_core::navigation_roots(&worker) })
                .await;
            _ = this.update(cx, |this, cx| {
                if this.tree_generation != generation {
                    return;
                }
                this.roots_job = None;
                match result {
                    Ok(Some(mut roots)) => {
                        // Keep explicit roots (notably UNC shares) without enumerating the network.
                        if let Some(target) = &this.tree_target
                            && let Some(root) =
                                target.path.ancestors().find(|path| path.parent().is_none())
                            && !roots.iter().any(|known| known == root)
                        {
                            roots.push(root.to_path_buf());
                        }
                        for root in &this.roots {
                            if this.expanded.contains(root) && !roots.contains(root) {
                                roots.push(root.clone());
                            }
                        }
                        this.roots = roots;
                        this.roots_loaded = true;
                    }
                    Ok(None) => return,
                    Err(error) => this.roots_error = Some(error.to_string()),
                }
                this.rebuild_tree(cx);
                cx.notify();
            });
        });
        self.roots_job = Some(SidebarJob {
            cancellation,
            _task: task,
        });
    }

    pub(super) fn load_expanded_directories(&mut self, cx: &mut Context<Self>) {
        for path in self.visible_tree_directories() {
            self.load_directory(path, cx);
        }
    }

    fn visible_tree_directories(&self) -> Vec<PathBuf> {
        self.expanded
            .iter()
            .filter(|path| {
                path.ancestors().all(|ancestor| {
                    ancestor.as_os_str().is_empty() || self.expanded.contains(ancestor)
                })
            })
            .cloned()
            .collect()
    }

    pub(super) fn load_directory(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if self.directories.contains_key(&path) || self.directory_jobs.contains_key(&path) {
            return;
        }
        self.reload_directory(path, cx);
    }

    fn reload_directory(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if self.directory_jobs.contains_key(&path) {
            return;
        }
        let generation = self.tree_generation;
        let cancellation = CancellationToken::default();
        let worker = cancellation.clone();
        let worker_path = path.clone();
        let job_path = path.clone();
        self.directories
            .entry(path.clone())
            .or_insert(DirectoryLoad::Loading);
        let task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    vclogg_core::navigation_directory(&worker_path, &worker)
                })
                .await;
            _ = this.update(cx, |this, cx| {
                if this.tree_generation != generation {
                    return;
                }
                this.directory_jobs.remove(&path);
                let retired = match result {
                    Ok(Some(entries)) => this
                        .directories
                        .insert(path, DirectoryLoad::Ready(Arc::new(entries))),
                    Ok(None) => this.directories.remove(&path),
                    Err(error) => this
                        .directories
                        .insert(path, DirectoryLoad::Failed(error.to_string())),
                };
                release_in_background(retired, cx);
                this.rebuild_tree(cx);
                cx.notify();
            });
        });
        self.directory_jobs.insert(
            job_path,
            SidebarJob {
                cancellation,
                _task: task,
            },
        );
    }

    pub(super) fn retry_tree_directory(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.tree_error = None;
        if let Some(retired) = self.directories.remove(&path) {
            release_in_background(retired, cx);
        }
        self.reload_directory(path, cx);
        self.rebuild_tree(cx);
    }

    pub(super) fn refresh_tree(&mut self, cx: &mut Context<Self>) {
        self.tree_generation += 1;
        self.directory_jobs.clear();
        self.tree_retried.clear();
        self.tree_error = None;
        self.roots_loaded = false;
        self.load_roots(cx);
        // Retain the visible snapshot while replacing expanded directory listings.
        let visible = self
            .visible_tree_directories()
            .into_iter()
            .collect::<BTreeSet<_>>();
        let old = std::mem::take(&mut self.directories);
        for (path, load) in &old {
            if visible.contains(path) && matches!(load, DirectoryLoad::Ready(_)) {
                self.directories.insert(path.clone(), load.clone());
            }
        }
        release_in_background(old, cx);
        for path in visible {
            self.reload_directory(path, cx);
        }
        self.rebuild_tree(cx);
        cx.notify();
    }

    /// Resolve only the latest navigation intent, never the document generation.
    fn settle_tree_target(&mut self, cx: &mut Context<Self>) {
        if !self.tree_reveal_active {
            return;
        }
        let Some(target) = self.tree_target.clone() else {
            return;
        };
        let id = SharedString::from(encode_persisted_path(&target.path));
        if let Some(ix) = self.tree.read(cx).index_of(&id) {
            self.tree.update(cx, |tree, cx| {
                tree.set_selected_index(Some(ix), cx);
                tree.scroll_to_item(ix, ScrollStrategy::Nearest);
            });
            self.tree_reveal_active = false;
            return;
        }
        let mut chain = target.path.ancestors().collect::<Vec<_>>();
        chain.reverse();
        for pair in chain.windows(2) {
            let (parent, child) = (pair[0], pair[1]);
            if self.directory_jobs.contains_key(parent) {
                return;
            }
            match self.directories.get(parent) {
                Some(DirectoryLoad::Ready(entries))
                    if !entries.iter().any(|entry| entry.path() == child) =>
                {
                    if self.tree_retried.insert(parent.to_path_buf()) {
                        self.reload_directory(parent.to_path_buf(), cx);
                        return;
                    }
                    self.tree_error = Some(crate::tr_args!(
                        "无法定位：{}",
                        "Couldn’t locate: {}",
                        target.path.display()
                    ));
                    self.tree_reveal_active = false;
                    return;
                }
                Some(DirectoryLoad::Failed(error)) => {
                    self.tree_error = Some(crate::tr_args!(
                        "无法读取目录 {}：{}",
                        "Couldn’t read folder {}: {}",
                        parent.display(),
                        error
                    ));
                    self.tree_reveal_active = false;
                    return;
                }
                None | Some(DirectoryLoad::Loading) => return,
                _ => {}
            }
        }
    }

    fn hidden_tree_paths(&self) -> BTreeSet<PathBuf> {
        self.tree_target.as_ref().map(|target| target.path.as_path()).into_iter()
            .chain(self.document.as_ref().map(|(_, doc)| doc.path()))
            .flat_map(Path::ancestors)
            .filter(|path| {
                path.file_name().is_some_and(|name| name.as_encoded_bytes().starts_with(b"."))
                    || path.parent().and_then(|parent| self.directories.get(parent)).is_some_and(|load| {
                        matches!(load, DirectoryLoad::Ready(entries) if entries.iter().any(|entry| entry.path() == *path && entry.is_hidden()))
                    })
            })
            .map(Path::to_path_buf)
            .collect()
    }

    pub(super) fn rebuild_tree(&mut self, cx: &mut Context<Self>) {
        self.tree_dirty = true;
        self.tree_job.take();
        self.tree_request += 1;
        if !self.is_showing(SidebarPanelId::Files) {
            return;
        }
        let roots = self.roots.clone();
        let directories = self.directories.clone();
        let expanded = self.expanded.clone();
        let exceptions = self.hidden_tree_paths();
        let installed_exceptions = exceptions.clone();
        let text_system = cx.text_system().clone();
        let font = gpui_kit::font(cx.theme().font_family.clone());
        let rem_size = cx.theme().font_size;
        let generation = self.tree_generation;
        let request = self.tree_request;
        let cancellation = CancellationToken::default();
        let worker = cancellation.clone();
        let task = cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(16))
                .await;
            let prepared = cx
                .background_spawn(async move {
                    fn folder(
                        path: &Path,
                        directories: &BTreeMap<PathBuf, DirectoryLoad>,
                        expanded: &BTreeSet<PathBuf>,
                        exceptions: &BTreeSet<PathBuf>,
                        worker: &CancellationToken,
                    ) -> Option<FileTreeNode> {
                        if worker.is_cancelled() {
                            return None;
                        }
                        let id = encode_persisted_path(path);
                        let mut children = Vec::new();
                        if expanded.contains(path) {
                            match directories.get(path) {
                                Some(DirectoryLoad::Ready(entries)) => {
                                    for entry in entries.iter() {
                                        if worker.is_cancelled() {
                                            return None;
                                        }
                                        if entry.is_hidden() && !exceptions.contains(entry.path()) {
                                            continue;
                                        }
                                        children.push(if entry.is_directory() {
                                            folder(
                                                entry.path(),
                                                directories,
                                                expanded,
                                                exceptions,
                                                worker,
                                            )?
                                        } else {
                                            FileTreeNode::new(
                                                encode_persisted_path(entry.path()),
                                                entry
                                                    .path()
                                                    .file_name()
                                                    .unwrap_or_default()
                                                    .to_string_lossy()
                                                    .into_owned(),
                                            )
                                        });
                                    }
                                    if children.is_empty() {
                                        children.push(
                                            FileTreeNode::new(
                                                format!("empty:{id}"),
                                                crate::tr!("空文件夹", "Empty folder"),
                                            )
                                            .disabled(true),
                                        );
                                    }
                                }
                                Some(DirectoryLoad::Failed(error)) => children.push(
                                    FileTreeNode::new(format!("error:{id}"), error.clone())
                                        .disabled(true),
                                ),
                                _ => children.push(
                                    FileTreeNode::new(
                                        format!("pending:{id}"),
                                        crate::tr!("加载中…", "Loading…"),
                                    )
                                    .disabled(true),
                                ),
                            }
                        } else {
                            // Tree's folder semantics require a child; never traverse cached collapsed descendants.
                            children.push(
                                FileTreeNode::new(format!("pending:{id}"), "").disabled(true),
                            );
                        }
                        Some(
                            FileTreeNode::new(
                                id,
                                path.file_name()
                                    .unwrap_or(path.as_os_str())
                                    .to_string_lossy()
                                    .into_owned(),
                            )
                            .children(children)
                            .expanded(expanded.contains(path)),
                        )
                    }
                    let items = roots
                        .iter()
                        .map(|root| folder(root, &directories, &expanded, &exceptions, &worker))
                        .collect::<Option<Vec<_>>>()?;
                    // Measure expanded rows off the UI thread. The public Tree API
                    // installs the resulting roots on the UI thread.
                    let text_system = gpui_kit::WindowTextSystem::new(text_system);
                    let mut width = Pixels::ZERO;
                    let mut pending = items
                        .iter()
                        .rev()
                        .map(|item| (item, 0usize))
                        .collect::<Vec<_>>();
                    while let Some((item, depth)) = pending.pop() {
                        if worker.is_cancelled() {
                            return None;
                        }
                        let label = tree_label(&item.label);
                        let run = gpui_kit::TextRun {
                            len: label.len(),
                            font: font.clone(),
                            ..Default::default()
                        };
                        let text_width = text_system
                            .layout_line(&label, rem_size * TREE_TEXT_REM, &[run], None)
                            .width;
                        let chrome = rem_size
                            * (depth as f32 + TREE_PADDING_REM * 2. + TREE_ICON_REM + TREE_GAP_REM);
                        width = width.max((text_width + chrome).ceil());
                        if item.expanded {
                            pending
                                .extend(item.children.iter().rev().map(|child| (child, depth + 1)));
                        }
                    }
                    Some((items, width))
                })
                .await;
            _ = this.update(cx, |this, cx| {
                if this.tree_generation != generation || this.tree_request != request {
                    release_in_background(prepared, cx);
                    return;
                }
                this.tree_job = None;
                if let Some((items, width)) = prepared {
                    // Read selection at installation so user clicks during preparation survive.
                    let selected = this
                        .tree
                        .read(cx)
                        .selected_entry()
                        .map(|entry| entry.item().id.clone());
                    this.tree.update(cx, |tree, cx| {
                        tree.set_items(
                            items
                                .into_iter()
                                .map(FileTreeNode::into_tree_item)
                                .collect::<Vec<_>>(),
                            cx,
                        );
                        let selected_ix = selected.as_ref().and_then(|id| tree.index_of(id));
                        tree.set_selected_index(selected_ix, cx);
                    });
                    this.tree_width = width;
                    this.tree_dirty = false;
                    this.tree_exceptions = installed_exceptions;
                    this.settle_tree_target(cx);
                }
                cx.notify();
            });
        });
        self.tree_job = Some(SidebarJob {
            cancellation,
            _task: task,
        });
    }
}

impl Workspace {
    pub(in super::super) fn reveal_in_sidebar(
        &mut self,
        path: PathBuf,
        directory: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sidebar.update(cx, |state, cx| {
            let side = if state.layout.sides[1]
                .panels
                .contains(&SidebarPanelId::Files)
            {
                SidebarSide::Right
            } else {
                SidebarSide::Left
            };
            state.activate(SidebarPanelId::Files, side, window, cx);
            state.reveal_tree_path(path, directory, cx);
        });
    }
}
