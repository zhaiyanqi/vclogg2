use super::*;

struct SearchColorClearTarget {
    scope: SearchScope,
    revision: u64,
    results: GlobalSearchResults,
    paths: Vec<PathBuf>,
    all_results: bool,
}

impl Workspace {
    pub(super) fn append_search_color_clear_menu(
        mut menu: PopupMenu,
        workspace: Entity<Self>,
        window: &mut Window,
        cx: &mut Context<PopupMenu>,
    ) -> PopupMenu {
        let this = workspace.read(cx);
        let scope = this.global_search.scope;
        let revision = this.global_search.revision;
        let results = this.global_search.results.clone();
        let table = this.global_table.read(cx);
        let current_path = table
            .active_log_row()
            .and_then(|row| table.delegate().row_key(row))
            .and_then(|key| {
                let document_id = match key {
                    LogRowKey::Row { document_id, .. } | LogRowKey::FileGroup { document_id } => {
                        document_id
                    }
                };
                this.global_search
                    .results
                    .get(&document_id)
                    .map(|result| result.path.clone())
            });
        let result_paths = table
            .delegate()
            .projected_result_groups()
            .map(|(path, _, _)| path.to_path_buf())
            .collect::<Vec<_>>();
        let available = this.global_search.result_scope == Some(scope);
        for (label, paths, all_results) in [
            (
                crate::tr!("清除当前文件颜色", "Clear current file colors"),
                current_path.into_iter().collect(),
                false,
            ),
            (
                crate::tr!("清除所有颜色", "Clear all colors"),
                result_paths,
                true,
            ),
        ] {
            let disabled = !available || paths.is_empty();
            let target = SearchColorClearTarget {
                scope,
                revision,
                results: results.clone(),
                paths,
                all_results,
            };
            menu = menu.item(PopupMenuItem::new(label).disabled(disabled).on_click(
                window.listener_for(&workspace, move |this, _, window, cx| {
                    this.clear_search_colors(&target, window, cx);
                }),
            ));
        }
        menu
    }

    fn clear_search_colors(
        &mut self,
        target: &SearchColorClearTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.global_search.scope != target.scope
            || self.global_search.result_scope != Some(target.scope)
            || self.global_search.revision != target.revision
            || !self.global_search.results.is_same_snapshot(&target.results)
        {
            window.notify_message(
                crate::tr!(
                    "搜索结果已发生变化，请重新打开菜单",
                    "Search results changed. Open the menu again."
                ),
                cx,
            );
            return;
        }
        self.cancel_color_rule_action();
        let context = match target.scope {
            SearchScope::AllOpenFiles => &mut self.global_search.all_open_context,
            SearchScope::Directory => &mut self.global_search.directory_context,
            SearchScope::CurrentFile => return,
        };
        if target.all_results {
            context.keyword_color_rules.clear();
            context.resolved_color_rules = Arc::default();
            context.color_exclusions = Default::default();
        } else {
            for path in &target.paths {
                context
                    .color_exclusions
                    .clear_file(path, &context.keyword_color_rules);
            }
        }
        let paths = target
            .paths
            .iter()
            .map(|path| path_match_key(path))
            .collect::<BTreeSet<_>>();
        let mut changed_documents = Vec::new();
        for tab in &mut self.documents {
            if !paths.contains(&path_match_key(tab.document.path())) {
                continue;
            }
            tab.file.keyword_color_rules.clear();
            tab.file.resolved_color_rules = Arc::default();
            for table in [&tab.log_table, &tab.result_table] {
                table.update(cx, |table, cx| {
                    table.delegate_mut().set_color_rules(Arc::default());
                    table.refresh(cx);
                });
            }
            changed_documents.push(tab.id);
        }
        self.refresh_global_result_rows(window, cx);
        self.refresh_active_log_search_presentation(cx);
        for document_id in changed_documents {
            self.schedule_checkpoint(document_id, window, cx);
        }
        self.schedule_workspace_search_state_save(window, cx);
        window.notify_message(
            if target.all_results {
                crate::tr!(
                    "已清除搜索结果中的所有颜色",
                    "Cleared all colors from search results"
                )
            } else {
                crate::tr!(
                    "已清除当前文件的所有颜色",
                    "Cleared all colors from the current file"
                )
            },
            cx,
        );
        cx.notify();
    }
}
