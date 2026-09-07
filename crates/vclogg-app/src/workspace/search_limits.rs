use super::*;

/// One range per file, shared by the three search scopes in this workspace.
/// Completed ranges only track whether an append may reuse the installed results.
#[derive(Clone, Default)]
pub(super) struct FileSearchRanges {
    limits: BTreeMap<PathMatchKey, SearchRange>,
    local_results: BTreeMap<PathMatchKey, SearchRange>,
    global_results: BTreeMap<PathMatchKey, SearchRange>,
}

impl FileSearchRanges {
    pub(super) fn set(&mut self, path: &Path, range: SearchRange) {
        self.limits.insert(path_match_key(path), range);
    }

    pub(super) fn get(&self, path: &Path) -> SearchRange {
        self.limits
            .get(&path_match_key(path))
            .copied()
            .unwrap_or_default()
    }

    pub(super) fn completed(&mut self, path: &Path, range: SearchRange, global: bool) {
        let results = if global {
            &mut self.global_results
        } else {
            &mut self.local_results
        };
        results.insert(path_match_key(path), range);
    }

    pub(super) fn can_extend(&self, path: &Path, global: bool) -> bool {
        let results = if global {
            &self.global_results
        } else {
            &self.local_results
        };
        results
            .get(&path_match_key(path))
            .copied()
            .unwrap_or_default()
            == self.get(path)
    }
}

#[derive(Clone, Copy)]
enum SearchBoundary {
    Start,
    End,
    Clear,
}

impl Workspace {
    fn search_limit_target(&self, region: LogRegion, cx: &App) -> Option<(PathBuf, Option<usize>)> {
        match region {
            LogRegion::GlobalResults => {
                let table = self.global_table.read(cx);
                let key = table.delegate().row_key(table.active_log_row()?)?;
                let (document_id, row) = match key {
                    LogRowKey::Row {
                        document_id,
                        source_row,
                    } => (document_id, Some(source_row)),
                    LogRowKey::FileGroup { document_id } => (document_id, None),
                };
                let result = self.global_search.results.get(&document_id)?;
                Some((result.path.clone(), row))
            }
            region => {
                let tab = self.active_document()?;
                let table = if region == LogRegion::CurrentResults {
                    &tab.result_table
                } else {
                    &tab.log_table
                };
                let table = table.read(cx);
                let row = table
                    .active_log_row()
                    .and_then(|row| table.delegate().source_row(row));
                Some((tab.document.path().to_path_buf(), row))
            }
        }
    }

    pub(super) fn append_search_limit_menu(
        menu: PopupMenu,
        workspace: Entity<Self>,
        region: LogRegion,
        window: &mut Window,
        cx: &mut Context<PopupMenu>,
    ) -> PopupMenu {
        let Some((path, row)) = workspace.read(cx).search_limit_target(region, cx) else {
            return menu;
        };
        let range = workspace.read(cx).search_ranges.get(&path);
        let loading = workspace.read(cx).documents.iter().any(|tab| {
            paths_match(tab.document.path(), &path) && tab.load_state != DocumentLoadState::Ready
        });
        let summary = match range.end() {
            Some(end) => crate::tr_args!(
                "搜索范围：第 {}–{} 行",
                "Search range: lines {}–{}",
                range.start().saturating_add(1),
                end.saturating_add(1)
            ),
            None => crate::tr_args!(
                "搜索范围：第 {} 行至文件末尾",
                "Search range: line {} to end of file",
                range.start().saturating_add(1)
            ),
        };
        let mut menu = menu
            .separator()
            .item(PopupMenuItem::new(summary).disabled(true));
        for (label, boundary, disabled) in [
            (
                crate::tr!("设置搜索起点", "Set search start"),
                SearchBoundary::Start,
                row.is_none() || loading,
            ),
            (
                crate::tr!("设置搜索终点", "Set search end"),
                SearchBoundary::End,
                row.is_none() || loading,
            ),
            (
                crate::tr!("清除搜索限制", "Clear search limits"),
                SearchBoundary::Clear,
                range.is_unrestricted() || loading,
            ),
        ] {
            let path = path.clone();
            menu = menu.item(PopupMenuItem::new(label).disabled(disabled).on_click(
                window.listener_for(&workspace, move |this, _, window, cx| {
                    this.set_search_boundary(&path, row, boundary, window, cx);
                }),
            ));
        }
        menu
    }

    fn set_search_boundary(
        &mut self,
        path: &Path,
        row: Option<usize>,
        boundary: SearchBoundary,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.documents.iter().any(|tab| {
            paths_match(tab.document.path(), path) && tab.load_state != DocumentLoadState::Ready
        }) {
            return;
        }
        let previous = self.search_ranges.get(path);
        let range = match (boundary, row) {
            (SearchBoundary::Start, Some(row)) => previous.with_start(row),
            (SearchBoundary::End, Some(row)) => previous.with_end(row),
            (SearchBoundary::Clear, _) => SearchRange::default(),
            _ => return,
        };
        if range == previous {
            return;
        }
        self.cancel_search();
        self.file_refresh_task.take();
        self.global_search.revision = self.global_search.revision.saturating_add(1);
        for tab in &mut self.documents {
            if paths_match(tab.document.path(), path) {
                tab.search_revision = tab.search_revision.saturating_add(1);
            }
        }
        self.search_ranges.set(path, range);
        let message = if range.is_unrestricted() {
            crate::tr!(
                "已清除该文件的搜索限制，下次搜索将搜索整个文件",
                "Search limits cleared for this file. The next search will scan the entire file."
            )
            .to_string()
        } else {
            crate::tr!(
                "已更新该文件的搜索范围，下次搜索生效",
                "Search range updated for this file. It applies to the next search."
            )
            .to_string()
        };
        window.push_notification(message, cx);
        cx.notify();
    }
}
