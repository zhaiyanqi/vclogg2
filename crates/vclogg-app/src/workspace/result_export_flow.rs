use super::*;

impl Workspace {
    fn result_export_snapshot(&self, cx: &App) -> Option<ResultExport> {
        match self.global_search.scope {
            SearchScope::CurrentFile => {
                let tab = self.active_document()?;
                let rows = tab.result_rows(cx);
                (tab.load_state == DocumentLoadState::Ready && !rows.is_empty()).then(|| {
                    ResultExport::Single {
                        document: tab.document.clone(),
                        rows,
                    }
                })
            }
            scope @ (SearchScope::AllOpenFiles | SearchScope::Directory) => {
                if scope == SearchScope::Directory
                    && self.global_search.result_scope != Some(SearchScope::Directory)
                {
                    return None;
                }
                let table = self.global_table.read(cx);
                let groups = table
                    .delegate()
                    .projected_result_groups()
                    .filter(|(_, _, rows)| !rows.is_empty())
                    .map(|(path, document, rows)| ExportGroup {
                        path: path.to_path_buf(),
                        document: document.clone(),
                        rows: rows.clone(),
                    })
                    .collect::<Vec<_>>();
                (!groups.is_empty()).then(|| ResultExport::Global {
                    groups: groups.into(),
                })
            }
        }
    }

    fn result_export_suggested_name(&self) -> String {
        match self.global_search.scope {
            SearchScope::CurrentFile => self
                .active_document()
                .and_then(|tab| tab.document.path().file_stem())
                .map(|stem| format!("{}-results.log", stem.to_string_lossy()))
                .unwrap_or_else(|| "search-results.log".to_string()),
            SearchScope::AllOpenFiles => "global-search-results.log".to_string(),
            SearchScope::Directory => "directory-search-results.log".to_string(),
        }
    }

    pub(super) fn open_search_results_in_new_tab_action(
        &mut self,
        _: &OpenSearchResultsInNewTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_results_in_new_tab(window, cx);
    }

    pub(super) fn merge_search_results_in_new_tab_action(
        &mut self,
        _: &MergeSearchResultsInNewTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_timestamp_merged_results(window, cx);
    }

    pub(super) fn save_search_results_to_file_action(
        &mut self,
        _: &SaveSearchResultsToFile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.save_results_as(window, cx);
    }

    fn open_results_in_new_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.result_export_task.is_some() || self.open_task.is_some() {
            return;
        }
        let Some(export) = self.result_export_snapshot(cx) else {
            return;
        };
        self.result_export_operation = Some(ResultExportOperation::OpenInNewTab);
        cx.notify();
        self.result_export_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let row_count = export.row_count();
                    let path = result_export::save_to_unique_temp(&export)?;
                    Ok::<_, anyhow::Error>((path, row_count))
                })
                .await;
            _ = this.update_in(cx, |this, window, cx| {
                this.result_export_task = None;
                this.result_export_operation = None;
                match result {
                    Ok((path, row_count)) if this.open_task.is_none() => {
                        this.transient_paths.insert(path_match_key(&path));
                        this.begin_open_paths(vec![path], window, cx);
                        window.push_notification(
                            crate::tr_args!(
                                "已将 {row_count} 行结果写入新标签",
                                "Wrote {row_count} result lines to a new tab",
                            ),
                            cx,
                        );
                    }
                    Ok((path, _)) => window.push_notification(
                        crate::tr_args!(
                            "结果已写入 {}，但当前正在打开其他文件，请稍后重试",
                            "Results were written to {}, but another file is being opened. Try again shortly.",
                            path.display(),
                        ),
                        cx,
                    ),
                    Err(error) => {
                        window.push_notification(
                            crate::tr_args!(
                                "结果未能写入新标签：{error}",
                                "Couldn’t write results to a new tab: {error}",
                            ),
                            cx,
                        )
                    }
                }
                cx.notify();
            });
        }));
    }

    fn open_timestamp_merged_results(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.global_search.scope, SearchScope::CurrentFile)
            || self.result_export_task.is_some()
            || self.open_task.is_some()
        {
            return;
        }
        let Some(export) = self.result_export_snapshot(cx) else {
            return;
        };
        self.result_export_operation = Some(ResultExportOperation::MergeByTimestamp);
        cx.notify();
        self.result_export_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let row_count = export.row_count();
                    let path = result_export::save_timestamp_merged_to_unique_temp(&export)?;
                    Ok::<_, anyhow::Error>((path, row_count))
                })
                .await;
            _ = this.update_in(cx, |this, window, cx| {
                this.result_export_task = None;
                this.result_export_operation = None;
                match result {
                    Ok((path, row_count)) if this.open_task.is_none() => {
                        this.transient_paths.insert(path_match_key(&path));
                        this.begin_open_paths(vec![path], window, cx);
                        window.push_notification(
                            crate::tr_args!(
                                "已按时间戳合并 {row_count} 行结果到新标签",
                                "Merged {row_count} result lines by timestamp into a new tab",
                            ),
                            cx,
                        );
                    }
                    Ok((path, _)) => window.push_notification(
                        crate::tr_args!(
                            "结果已生成到 {}，但当前正在打开其他文件",
                            "Results were generated at {}, but another file is being opened",
                            path.display(),
                        ),
                        cx,
                    ),
                    Err(error) => window.push_notification(
                        crate::tr_args!(
                            "按时间戳合并失败：{error}",
                            "Couldn’t merge by timestamp: {error}",
                        ),
                        cx,
                    ),
                }
                cx.notify();
            });
        }));
    }

    fn save_results_as(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.result_export_task.is_some() {
            return;
        }
        let Some(export) = self.result_export_snapshot(cx) else {
            return;
        };
        let suggested_name = self.result_export_suggested_name();
        let directory = self
            .active_document()
            .and_then(|tab| tab.document.path().parent())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let prompt = cx.prompt_for_new_path(&directory, Some(&suggested_name));
        self.result_export_operation = Some(ResultExportOperation::SaveAs);
        cx.notify();
        self.result_export_task = Some(cx.spawn_in(window, async move |this, cx| {
            let selected_path = prompt.await;
            let result = match selected_path {
                Ok(Ok(Some(target))) => {
                    let saved = cx
                        .background_spawn(async move {
                            let row_count = result_export::save(&export, &target)?;
                            Ok::<_, anyhow::Error>((target, row_count))
                        })
                        .await;
                    Some(saved)
                }
                Ok(Ok(None)) => None,
                Ok(Err(error)) => Some(Err(error)),
                Err(error) => Some(Err(anyhow::anyhow!(error))),
            };
            _ = this.update_in(cx, |this, window, cx| {
                this.result_export_task = None;
                this.result_export_operation = None;
                match result {
                    Some(Ok((path, row_count))) => window.push_notification(
                        crate::tr_args!(
                            "已保存 {row_count} 行结果到 {}",
                            "Saved {row_count} result lines to {}",
                            path.display(),
                        ),
                        cx,
                    ),
                    Some(Err(error)) => window.push_notification(
                        crate::tr_args!("结果未能保存：{error}", "Couldn’t save results: {error}",),
                        cx,
                    ),
                    None => {}
                }
                cx.notify();
            });
        }));
    }
}
