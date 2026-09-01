use super::*;

impl Workspace {
    pub(super) fn copy_current_line(
        &mut self,
        _: &CopyCurrentLine,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.copy_selected_line(false, window, cx);
    }

    pub(super) fn copy_current_line_with_number(
        &mut self,
        _: &CopyCurrentLineWithNumber,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.copy_selected_line(true, window, cx);
    }

    pub(super) fn copy_selected_line(
        &mut self,
        include_line_number: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.line_copy_revision = self.line_copy_revision.saturating_add(1);
        if let Some(cancellation) = self.line_copy_cancellation.take() {
            cancellation.cancel();
        }
        self.line_copy_task = None;
        let revision = self.line_copy_revision;
        if !include_line_number {
            let selected_text = TextSelection::selected_text(window, cx);
            if !selected_text.trim().is_empty() {
                cx.write_to_clipboard(ClipboardItem::new_string(selected_text));
                window.push_notification(crate::tr!("已复制所选文字", "Selected text copied"), cx);
                return;
            }
        }
        if self.active_log_region == LogRegion::GlobalResults && self.global_search.results_visible
        {
            let selected_documents = self
                .global_table
                .read(cx)
                .delegate()
                .selected_match_documents();
            if selected_documents.is_empty() {
                window.push_notification(
                    crate::tr!(
                        "请先选择要复制的全局结果行",
                        "Select global result lines to copy first"
                    ),
                    cx,
                );
                return;
            }
            self.start_line_copy(
                selected_documents,
                include_line_number,
                LineCopyScope::Global,
                revision,
                window,
                cx,
            );
            return;
        }
        let Some(tab) = self.active_document() else {
            return;
        };
        let selected_rows = tab.selected_source_rows_compressed(cx);
        if selected_rows.is_empty() {
            window.push_notification(
                crate::tr!("请先选择要复制的日志行", "Select log lines to copy first"),
                cx,
            );
            return;
        }
        self.start_line_copy(
            vec![(tab.document.clone(), selected_rows)],
            include_line_number,
            LineCopyScope::Local,
            revision,
            window,
            cx,
        );
    }

    pub(super) fn start_line_copy(
        &mut self,
        documents: Vec<(Arc<LogDocument>, CompressedRows)>,
        include_line_number: bool,
        scope: LineCopyScope,
        revision: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let cancellation = SearchCancellation::default();
        self.line_copy_cancellation = Some(cancellation.clone());
        self.line_copy_task = Some(cx.spawn_in(window, async move |this, cx| {
            let copied = cx
                .background_spawn(async move {
                    collect_log_lines_for_clipboard(
                        documents,
                        include_line_number,
                        &cancellation,
                    )
                })
                .await;
            _ = this.update_in(cx, |this, window, cx| {
                if this.line_copy_revision != revision {
                    return;
                }
                this.line_copy_task = None;
                this.line_copy_cancellation = None;
                let copied = match copied {
                    DocumentLineTask::Completed(copied) => copied,
                    DocumentLineTask::Cancelled => return,
                    DocumentLineTask::SourceUnavailable => {
                        window.push_notification(
                            crate::tr!(
                                "所选日志的文件内容已改变，请重新加载后再复制",
                                "The selected log file changed. Reload it before copying."
                            ),
                            cx,
                        );
                        return;
                    }
                };
                if copied.text.is_empty() {
                    window.push_notification(
                        match scope {
                            LineCopyScope::Local => crate::tr!(
                                "所选日志行已不可用，请重新选择",
                                "The selected log lines are no longer available. Select them again."
                            ),
                            LineCopyScope::Global => crate::tr!(
                                "所选全局结果已不可用，请重新选择",
                                "The selected global results are no longer available. Select them again."
                            ),
                        },
                        cx,
                    );
                    return;
                }
                cx.write_to_clipboard(ClipboardItem::new_string(copied.text));
                let notification = match scope {
                    LineCopyScope::Global => crate::tr_args!(
                        "已复制 {} 条全局结果",
                        "Copied {} global results",
                        copied.count
                    ),
                    LineCopyScope::Local if copied.count == 1 => crate::tr_args!(
                        "已复制第 {} 行",
                        "Copied line {}",
                        copied.first_source_row.unwrap_or_default() + 1
                    ),
                    LineCopyScope::Local => {
                        crate::tr_args!("已复制 {} 行", "Copied {} lines", copied.count)
                    }
                };
                window.push_notification(notification, cx);
            });
        }));
    }

    pub(super) fn select_all_rows(
        &mut self,
        _: &SelectAllRows,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        TextSelection::clear(window, cx);
        if self.active_log_region == LogRegion::GlobalResults && self.global_search.results_visible
        {
            self.global_table.update(cx, |table, cx| {
                table.delegate().select_all_rows();
                cx.notify();
            });
            self.status_surface.update(cx, |_, cx| cx.notify());
            self.schedule_workspace_search_state_save(window, cx);
            return;
        }
        let Some(tab) = self.active_document() else {
            return;
        };
        let table = match tab.selection_table {
            SelectionTable::Log => tab.log_table.clone(),
            SelectionTable::Results => tab.result_table.clone(),
        };
        table.update(cx, |table, cx| {
            if table.delegate().selected_rows_count() == 0
                && table.delegate().source_row(0).is_some()
            {
                table.set_active_log_row(0, cx);
            }
            table.delegate().select_all_rows();
            cx.notify();
        });
        self.status_surface.update(cx, |_, cx| cx.notify());
        self.schedule_checkpoint(tab.id, window, cx);
    }

    pub(super) fn copy_file_path(
        &mut self,
        _: &CopyFilePath,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.active_document() else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(
            tab.document.path().display().to_string(),
        ));
        window.push_notification(crate::tr!("已复制文件路径", "File path copied"), cx);
    }

    pub(super) fn copy_document_encoding(
        &mut self,
        document_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.documents.iter().find(|tab| tab.id == document_id) else {
            return;
        };
        let encoding = tab.document.metadata().encoding_name.clone();
        cx.write_to_clipboard(ClipboardItem::new_string(encoding.clone()));
        window.push_notification(
            crate::tr_args!(
                "已复制编码名称：{encoding}",
                "Encoding name copied: {encoding}"
            ),
            cx,
        );
    }

    pub(super) fn reload_document_encoding(
        &mut self,
        document_id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.reload_document(document_id, false, ReloadStrategy::Full, window, cx);
    }

    pub(super) fn build_encoding_menu(
        menu: PopupMenu,
        document_id: u64,
        encoding_name: SharedString,
        workspace: Entity<Self>,
        window: &mut Window,
    ) -> PopupMenu {
        let reload = {
            let workspace = workspace.clone();
            window.listener_for(&workspace, move |this, _, window, cx| {
                this.reload_document_encoding(document_id, window, cx)
            })
        };
        let copy = window.listener_for(&workspace, move |this, _, window, cx| {
            this.copy_document_encoding(document_id, window, cx)
        });
        menu.item(
            PopupMenuItem::new(crate::tr_args!(
                "当前编码：{encoding_name}",
                "Current encoding: {encoding_name}"
            ))
            .disabled(true),
        )
        .item(
            PopupMenuItem::new(crate::tr!("重新检测并加载", "Detect and reload")).on_click(reload),
        )
        .item(PopupMenuItem::new(crate::tr!("复制编码名称", "Copy encoding name")).on_click(copy))
    }

    pub(super) fn open_go_to_line(
        &mut self,
        _: &GoToLine,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.active_document() else {
            return;
        };
        if tab.load_state != DocumentLoadState::Ready {
            window.push_notification(
                crate::tr!(
                    "完整索引建立后即可按行号定位",
                    "Go to line will be available after the full index is built"
                ),
                cx,
            );
            return;
        }
        let line_count = tab.document.line_count();
        if line_count == 0 {
            window.push_notification(
                crate::tr!(
                    "当前文件没有可定位的日志行",
                    "The current file has no log line to locate"
                ),
                cx,
            );
            return;
        }

        let workspace = cx.entity();
        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(crate::tr_args!(
                "输入 1 到 {line_count} 之间的行号",
                "Enter a line number from 1 to {line_count}"
            ))
        });
        let focus = input.focus_handle(cx);
        window.defer(cx, move |window, cx| focus.focus(window, cx));

        window.open_dialog(cx, move |dialog, _, _| {
            let input_for_confirm = input.clone();
            let workspace_for_confirm = workspace.clone();
            dialog
                .title(crate::tr!("转到行", "Go to line"))
                .child(
                    v_flex()
                        .gap_3()
                        .child(crate::tr!("输入源日志中的行号。确认后会选择该行并滚动到可见位置。", "Enter a source log line number. The line will be selected and scrolled into view."))
                        .child(Input::new(&input)),
                )
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(crate::tr!("转到", "Go"))
                        .cancel_text(crate::tr!("取消", "Cancel"))
                        .show_cancel(true),
                )
                .on_ok(move |_, window, cx| {
                    let value = input_for_confirm.read(cx).value().trim().to_string();
                    let Ok(line_number) = value.parse::<usize>() else {
                        window.push_notification(crate::tr!("请输入有效的正整数行号", "Enter a valid positive line number"), cx);
                        return false;
                    };
                    let outcome = workspace_for_confirm.update(cx, |workspace, cx| {
                        let Some(tab) = workspace.active_document() else {
                            return Err(crate::tr!("当前没有活动日志文件", "There is no active log file").to_string());
                        };
                        let current_line_count = tab.document.line_count();
                        if !(1..=current_line_count).contains(&line_number) {
                            return Err(crate::tr_args!("行号应在 1 到 {current_line_count} 之间", "Line number must be from 1 to {current_line_count}"));
                        }

                        let source_row = line_number - 1;
                        let table = tab.log_table.clone();
                        table.update(cx, |table, cx| {
                            table.set_active_log_row(source_row, cx);
                        });
                        workspace.selected_source_row = Some(source_row);
                        cx.notify();
                        Ok(())
                    });
                    match outcome {
                        Ok(()) => true,
                        Err(message) => {
                            window.push_notification(message, cx);
                            false
                        }
                    }
                })
        });
    }

    pub(super) fn cycle_color_label(
        &mut self,
        _: &CycleColorLabel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selected_text = TextSelection::selected_text(window, cx);
        let (_, target) = match self.context_color_target(Some(selected_text.as_str()), cx) {
            Ok(target) => target,
            Err(message) => {
                window.push_notification(message, cx);
                return;
            }
        };
        self.start_color_rule_action(target, ColorRuleAction::Cycle, window, cx);
    }

    pub(super) fn toggle_marked_row(
        &mut self,
        _: &ToggleMarkedRow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active_log_region == LogRegion::GlobalResults && self.global_search.results_visible
        {
            let selected_matches = self.global_table.read(cx).delegate().selection_snapshot();
            if selected_matches.is_empty() {
                window.push_notification(
                    crate::tr!(
                        "请先选择要标记的全局结果行",
                        "Select global result lines to mark first"
                    ),
                    cx,
                );
                return;
            }
            let Some(selected_by_document) = self.resolve_global_mark_targets(&selected_matches)
            else {
                window.push_notification(
                    if self.global_search.scope == SearchScope::Directory {
                        crate::tr!(
                            "请打开所有选中结果对应的文件；若文件内容已改变，请重新搜索",
                            "Open every file for the selected results. If a file changed, search again."
                        )
                    } else {
                        crate::tr!(
                            "所选结果所属文件已不可用，请重新搜索",
                            "A file for the selected results is unavailable. Search again."
                        )
                    },
                    cx,
                );
                return;
            };
            let is_marking = selected_by_document.iter().any(|(document_id, rows)| {
                self.documents
                    .iter()
                    .find(|tab| tab.id == *document_id)
                    .is_some_and(|tab| !tab.marked_rows.contains_all(rows))
            });
            let mut changed_documents = Vec::new();
            let mut changed_rows = 0_usize;
            let row_height = self.log_row_height();
            for (document_id, rows) in selected_by_document {
                let Some(tab_ix) = self.documents.iter().position(|tab| tab.id == document_id)
                else {
                    continue;
                };
                let tab = &mut self.documents[tab_ix];
                if rows.is_empty() {
                    continue;
                }
                if is_marking {
                    tab.marked_rows.insert_rows(&rows);
                    tab.pending_restore_marked_rows.insert_rows(&rows);
                } else {
                    tab.marked_rows.remove_rows(&rows);
                    tab.pending_restore_marked_rows.remove_rows(&rows);
                }
                changed_rows = changed_rows.saturating_add(rows.len());
                let marked_rows = tab.marked_rows.clone();
                tab.log_table.update(cx, |table, cx| {
                    table.delegate_mut().set_marked_rows(marked_rows);
                    table.refresh(cx);
                });
                tab.refresh_result_rows(row_height, cx);
                if is_marking && tab.result_mode.includes_marks() {
                    tab.results_visible = true;
                }
                changed_documents.push(document_id);
            }
            self.refresh_global_result_rows(window, cx);
            if self
                .active_document()
                .is_some_and(|tab| changed_documents.contains(&tab.id))
            {
                self.refresh_active_document_surfaces_atomically(window, cx);
            }
            self.schedule_workspace_search_state_save(window, cx);
            for document_id in changed_documents {
                self.schedule_checkpoint(document_id, window, cx);
            }
            window.push_notification(
                crate::tr_args!(
                    "{} {changed_rows} 条全局结果",
                    "{} {changed_rows} global results",
                    if is_marking {
                        crate::tr!("已标记", "Marked")
                    } else {
                        crate::tr!("已取消标记", "Unmarked")
                    },
                ),
                cx,
            );
            cx.notify();
            return;
        }
        let Some(active_ix) = self.active_ix else {
            return;
        };
        let selected_rows = self.documents[active_ix].selected_source_rows_compressed(cx);
        if selected_rows.is_empty() {
            window.push_notification(
                crate::tr!("请先选择要标记的日志行", "Select log lines to mark first"),
                cx,
            );
            return;
        }
        let row_height = self.log_row_height();
        let (document_id, is_marking) = {
            let tab = &mut self.documents[active_ix];
            let selection_is_valid = selected_rows.first().is_some_and(|row| {
                tab.document.contains_source_row(row)
                    && selected_rows
                        .get(selected_rows.len().saturating_sub(1))
                        .is_some_and(|row| tab.document.contains_source_row(row))
            });
            if !selection_is_valid {
                window.push_notification(
                    crate::tr!(
                        "所选日志行已不可用，请重新选择",
                        "The selected log lines are no longer available. Select them again."
                    ),
                    cx,
                );
                return;
            }

            let is_marking = !tab.marked_rows.contains_all(&selected_rows);
            if is_marking {
                tab.marked_rows.insert_rows(&selected_rows);
                tab.pending_restore_marked_rows.insert_rows(&selected_rows);
            } else {
                tab.marked_rows.remove_rows(&selected_rows);
                tab.pending_restore_marked_rows.remove_rows(&selected_rows);
            }
            let marked_rows = tab.marked_rows.clone();
            tab.log_table.update(cx, |table, cx| {
                table.delegate_mut().set_marked_rows(marked_rows);
                table.refresh(cx);
            });
            tab.refresh_result_rows(row_height, cx);
            if is_marking && tab.result_mode.includes_marks() {
                tab.results_visible = true;
            }
            (tab.id, is_marking)
        };
        if is_marking
            && self.global_search.result_mode.includes_marks()
            && self.global_search.selected_documents.contains(&document_id)
        {
            self.global_search.results_visible = true;
        }
        self.refresh_global_result_rows(window, cx);
        self.refresh_active_document_surfaces_atomically(window, cx);
        self.schedule_workspace_search_state_save(window, cx);
        let action = if is_marking {
            crate::tr!("已标记", "Marked")
        } else {
            crate::tr!("已取消标记", "Unmarked")
        };
        if selected_rows.len() == 1 {
            let source_row = selected_rows
                .first()
                .expect("a one-row selection has a first row");
            window.push_notification(
                crate::tr_args!("{action}第 {} 行", "{action} line {}", source_row + 1),
                cx,
            );
        } else {
            window.push_notification(
                crate::tr_args!("{action} {} 行", "{action} {} lines", selected_rows.len()),
                cx,
            );
        }
        self.schedule_checkpoint(document_id, window, cx);
        cx.notify();
    }

    pub(super) fn focus_search(
        &mut self,
        _: &FocusSearch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let focus_handle = self.query.focus_handle(cx);
        focus_handle.focus(window, cx);
        self.query
            .update(cx, |state, cx| state.select_all(window, cx));
    }

    pub(super) fn remember_user_log_region(&mut self, region: LogRegion) {
        self.last_user_log_region = region;
        self.active_log_region = region;
    }

    pub(super) fn toggle_case_sensitive(
        &mut self,
        _: &ToggleCaseSensitive,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_search_defaults(!self.case_sensitive, self.regex, window, cx);
    }

    pub(super) fn toggle_regex(
        &mut self,
        _: &ToggleRegex,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_search_defaults(self.case_sensitive, !self.regex, window, cx);
    }

    pub(super) fn jump_to_start(
        &mut self,
        _: &JumpToStart,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.active_document() else {
            return;
        };
        if tab.document.line_count() == 0 {
            window.push_notification(
                crate::tr!("当前文件没有日志行", "The current file has no log lines"),
                cx,
            );
            return;
        }
        tab.log_table
            .update(cx, |table, cx| table.set_active_log_row(0, cx));
        self.selected_source_row = Some(0);
        cx.notify();
    }

    pub(super) fn toggle_fullscreen(
        &mut self,
        _: &ToggleFullscreen,
        window: &mut Window,
        _: &mut Context<Self>,
    ) {
        window.toggle_fullscreen();
    }

    pub(super) fn new_window(
        &mut self,
        _: &NewWindow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Err(error) = crate::open_workspace_window(cx, false, Vec::new()) {
            window.push_notification(
                crate::tr_args!(
                    "无法打开新窗口：{error}",
                    "Couldn’t open a new window: {error}"
                ),
                cx,
            );
        }
    }

    pub(super) fn jump_to_end(
        &mut self,
        _: &JumpToEnd,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.active_document() else {
            return;
        };
        if tab.load_state != DocumentLoadState::Ready {
            window.push_notification(
                crate::tr!(
                    "完整索引建立后即可跳到文件末尾",
                    "Jump to the end will be available after the full index is built"
                ),
                cx,
            );
            return;
        }
        let line_count = tab.document.line_count();
        if line_count == 0 {
            window.push_notification(
                crate::tr!("当前文件没有日志行", "The current file has no log lines"),
                cx,
            );
            return;
        }
        let last_row = line_count - 1;
        tab.log_table
            .update(cx, |table, cx| table.set_active_log_row(last_row, cx));
        self.selected_source_row = Some(last_row);
        cx.notify();
    }

    pub(super) fn toggle_auto_follow(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(active_ix) = self.active_ix else {
            return;
        };
        let tab = &mut self.documents[active_ix];
        if tab.load_state != DocumentLoadState::Ready {
            window.push_notification(
                crate::tr!(
                    "完整索引建立后即可开启末尾跟随",
                    "Follow end will be available after the full index is built"
                ),
                cx,
            );
            return;
        }
        tab.auto_follow = !tab.auto_follow;
        if tab.auto_follow && tab.document.line_count() > 0 {
            let last_row = tab.document.line_count() - 1;
            tab.log_table
                .update(cx, |table, cx| table.set_active_log_row(last_row, cx));
            self.selected_source_row = Some(last_row);
            window.push_notification(crate::tr!("已开启末尾跟随", "Follow end enabled"), cx);
        } else if !tab.auto_follow {
            window.push_notification(crate::tr!("已关闭末尾跟随", "Follow end disabled"), cx);
        }
        let document_id = tab.id;
        self.schedule_checkpoint(document_id, window, cx);
        cx.notify();
    }

    pub(super) fn toggle_line_numbers(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(active_ix) = self.active_ix else {
            return;
        };
        let document_id = {
            let tab = &mut self.documents[active_ix];
            tab.show_line_numbers = !tab.show_line_numbers;
            tab.uses_default_view_options = false;
            tab.refresh_view_options(cx);
            tab.id
        };
        self.schedule_checkpoint(document_id, window, cx);
        cx.notify();
    }

    pub(super) fn toggle_row_separators(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(active_ix) = self.active_ix else {
            return;
        };
        let document_id = {
            let tab = &mut self.documents[active_ix];
            tab.show_row_separators = !tab.show_row_separators;
            tab.uses_default_view_options = false;
            tab.refresh_view_options(cx);
            tab.id
        };
        self.schedule_checkpoint(document_id, window, cx);
        cx.notify();
    }
}
