use std::{collections::HashSet, path::PathBuf, sync::Arc};

use chrono::{DateTime, Local};
use gpui::{
    AppContext as _, Context, EventEmitter, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, ScrollHandle, SharedString, StatefulInteractiveElement as _,
    Styled as _, Subscription, Task, Window, div, prelude::FluentBuilder as _, rems, svg,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _, StyledExt as _,
    WindowExt as _,
    button::{Button, ButtonVariants as _},
    dialog::DialogFooter,
    h_flex,
    input::{Input, InputEvent, InputState},
    scroll::{Scrollbar, ScrollbarMode},
    v_flex,
};

use crate::{
    result_export::{
        TemporaryResultFile, is_temporary_result_path, move_temporary_result_to_trash,
        remove_empty_temporary_result_parent, temporary_result_files,
    },
    state_store::{DatabaseInfo, HistorySession, LastWorkspaceFile, RecentFile, StateStore},
};

const HISTORY_FILE_ROW_HEIGHT_REMS: f32 = 3.2;
const HISTORY_FILE_NAME_WIDTH_REMS: f32 = 17.;
const HISTORY_FILE_TIME_WIDTH_REMS: f32 = 7.;
const HISTORY_FILE_ACTIONS_WIDTH_REMS: f32 = 10.5;

fn persistent_list_scrollbar(id: &'static str, scroll_handle: &ScrollHandle) -> impl IntoElement {
    div()
        .relative()
        .h_full()
        .w(Scrollbar::width())
        .flex_none()
        .child(
            Scrollbar::vertical(scroll_handle)
                .id(id)
                .mode(ScrollbarMode::Always)
                .viewport_from_layout(),
        )
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum HistoryCategory {
    #[default]
    Files,
    TemporaryResults,
}

#[derive(Clone)]
pub enum HistoryDialogEvent {
    Open(PathBuf),
    ClearHistory,
    HistoryChanged {
        recent_files: Vec<RecentFile>,
        pinned_files: Vec<RecentFile>,
        last_workspace_files: Vec<LastWorkspaceFile>,
    },
}

pub struct HistoryDialog {
    filter: gpui::Entity<InputState>,
    session_scroll: ScrollHandle,
    temporary_result_scroll: ScrollHandle,
    category: HistoryCategory,
    sessions: Vec<HistorySession>,
    database_info: DatabaseInfo,
    temporary_results: Vec<TemporaryResultFile>,
    open_paths: HashSet<PathBuf>,
    store: Arc<StateStore>,
    confirming_delete_id: Option<i64>,
    deleting_id: Option<i64>,
    delete_task: Option<Task<()>>,
    deleting_temporary_results: bool,
    _subscriptions: Vec<Subscription>,
}

impl HistoryDialog {
    pub fn new(
        sessions: Vec<HistorySession>,
        database_info: DatabaseInfo,
        temporary_results: Vec<TemporaryResultFile>,
        open_paths: Vec<PathBuf>,
        store: Arc<StateStore>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let filter = cx.new(|cx| {
            InputState::new(window, cx).placeholder(crate::tr!(
                "按文件名、路径或查询筛选",
                "Filter by file name, path, or query"
            ))
        });
        let subscriptions =
            vec![cx.subscribe_in(&filter, window, |_, _, _: &InputEvent, _, cx| cx.notify())];
        Self {
            filter,
            session_scroll: ScrollHandle::new(),
            temporary_result_scroll: ScrollHandle::new(),
            category: HistoryCategory::Files,
            sessions,
            database_info,
            temporary_results,
            open_paths: open_paths.into_iter().collect(),
            store,
            confirming_delete_id: None,
            deleting_id: None,
            delete_task: None,
            deleting_temporary_results: false,
            _subscriptions: subscriptions,
        }
    }

    fn protection_reason(&self, session: &HistorySession) -> Option<&'static str> {
        if self.open_paths.contains(&session.path) {
            Some(crate::tr!("当前已打开", "Currently open"))
        } else if session.pinned {
            Some(crate::tr!("已收藏", "Favorited"))
        } else if session.marked_rows_count > 0 {
            Some(crate::tr!("包含行标记", "Contains marked lines"))
        } else {
            None
        }
    }

    fn begin_delete(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        if self.deleting_id.is_some() {
            return;
        }
        let Some(session) = self.sessions.iter().find(|session| session.id == id) else {
            return;
        };
        if let Some(reason) = self.protection_reason(session) {
            window.push_notification(
                crate::tr_args!(
                    "这条历史记录受保护：{reason}",
                    "This history entry is protected: {reason}"
                ),
                cx,
            );
            self.confirming_delete_id = None;
            cx.notify();
            return;
        }

        let store = self.store.clone();
        let open_paths = self.open_paths.iter().cloned().collect::<Vec<_>>();
        self.confirming_delete_id = None;
        self.deleting_id = Some(id);
        cx.notify();
        self.delete_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let removed = store.delete_history_session(id, &open_paths)?;
                    Ok::<_, anyhow::Error>((
                        removed,
                        store.session_history()?,
                        store.recent_files(8)?,
                        store.pinned_files()?,
                        store.last_workspace()?,
                    ))
                })
                .await;

            _ = this.update_in(cx, |this, window, cx| {
                this.deleting_id = None;
                this.delete_task = None;
                match result {
                    Ok((true, sessions, recent_files, pinned_files, last_workspace_files)) => {
                        this.sessions = sessions;
                        cx.emit(HistoryDialogEvent::HistoryChanged {
                            recent_files,
                            pinned_files,
                            last_workspace_files,
                        });
                        window.push_notification(
                            crate::tr!(
                                "历史记录已删除；日志文件未改变",
                                "History entry deleted; the log file was not changed"
                            ),
                            cx,
                        );
                    }
                    Ok((false, ..)) => {
                        window.push_notification(
                            crate::tr!(
                                "历史记录不存在或已变为受保护状态",
                                "The history entry no longer exists or is now protected"
                            ),
                            cx,
                        );
                    }
                    Err(error) => {
                        window.push_notification(
                            crate::tr_args!(
                                "历史记录未能删除：{error}",
                                "Couldn’t delete the history entry: {error}"
                            ),
                            cx,
                        );
                    }
                }
                cx.notify();
            });
        }));
    }

    fn confirm_delete_temporary_results(
        &mut self,
        paths: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let paths = paths
            .into_iter()
            .filter(|path| is_temporary_result_path(path) && !self.open_paths.contains(path))
            .collect::<Vec<_>>();
        if paths.is_empty() || self.deleting_temporary_results {
            return;
        }
        let count = paths.len();
        let dialog = cx.entity();
        window.open_alert_dialog(cx, move |alert, _, cx| {
            let dialog = dialog.clone();
            let paths = paths.clone();
            alert
                .title(if count == 1 {
                    crate::tr!("删除临时搜索结果？", "Delete temporary search result?").to_string()
                } else {
                    crate::tr_args!("删除 {count} 个临时搜索结果？", "Delete {count} temporary search results?")
                })
                .description(crate::tr!("这些临时文件会移入系统回收站；当前仍打开的结果不会进入本次操作。", "These temporary files will be moved to the system trash. Results that are still open are excluded."))
                .footer(
                    DialogFooter::new()
                        .justify_center()
                        .child(crate::dialog_focus::dialog_cancel_action(
                            "history-delete-temporary-cancel-action",
                            Button::new("history-delete-temporary-cancel")
                                .label(crate::tr!("取消", "Cancel")),
                            cx,
                        ))
                        .child(crate::dialog_focus::dialog_confirm_action(
                            "history-delete-temporary-confirm-action",
                            Button::new("history-delete-temporary-confirm")
                                .danger()
                                .label(crate::tr!("移入回收站", "Move to Trash")),
                            cx,
                        )),
                )
                .on_ok(move |_, window, cx| {
                    dialog.update(cx, |this, cx| {
                        this.delete_temporary_results(paths.clone(), window, cx)
                    });
                    true
                })
        });
    }

    fn delete_temporary_results(
        &mut self,
        paths: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.deleting_temporary_results {
            return;
        }
        self.deleting_temporary_results = true;
        cx.notify();
        self.delete_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let mut moved = 0_usize;
                    let mut failures = Vec::new();
                    for path in paths {
                        match move_temporary_result_to_trash(&path) {
                            Ok(true) => {
                                moved += 1;
                                remove_empty_temporary_result_parent(&path);
                            }
                            Ok(false) => remove_empty_temporary_result_parent(&path),
                            Err(error) => failures.push(error.to_string()),
                        }
                    }
                    Ok::<_, anyhow::Error>((moved, failures, temporary_result_files()?))
                })
                .await;
            _ = this.update_in(cx, |this, window, cx| {
                this.deleting_temporary_results = false;
                this.delete_task = None;
                match result {
                    Ok((moved, failures, files)) => {
                        this.temporary_results = files;
                        if failures.is_empty() {
                            window.push_notification(
                                format!("已将 {moved} 个临时搜索结果移入回收站"),
                                cx,
                            );
                        } else {
                            window.push_notification(
                                format!(
                                    "已清理 {moved} 个临时结果；另有 {} 个失败：{}",
                                    failures.len(),
                                    failures.join("；")
                                ),
                                cx,
                            );
                        }
                    }
                    Err(error) => window.push_notification(
                        crate::tr_args!(
                            "临时结果未能清理：{error}",
                            "Couldn’t clean temporary results: {error}"
                        ),
                        cx,
                    ),
                }
                cx.notify();
            });
        }));
    }

    fn render_temporary_result(
        &self,
        result: &TemporaryResultFile,
        index: usize,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let path = result.path.clone();
        let open = self.open_paths.contains(&path);
        v_flex()
            .id(("temporary-result", index))
            .w_full()
            .gap_2()
            .p_3()
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .justify_between()
                    .gap_3()
                    .child(
                        v_flex()
                            .min_w_0()
                            .gap_1()
                            .child(
                                div().font_semibold().child(
                                    result
                                        .path
                                        .file_name()
                                        .map(|name| name.to_string_lossy().into_owned())
                                        .unwrap_or_else(|| "临时搜索结果".to_string()),
                                ),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(result.path.display().to_string()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!(
                                        "{}{}",
                                        format_temporary_size(result.size),
                                        if open { " · 当前已打开" } else { "" }
                                    )),
                            ),
                    )
                    .child(
                        Button::new(("delete-temporary-result", index))
                            .small()
                            .danger()
                            .label(crate::tr!("移入回收站", "Move to Trash"))
                            .disabled(open || self.deleting_temporary_results)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.confirm_delete_temporary_results(
                                    vec![path.clone()],
                                    window,
                                    cx,
                                )
                            })),
                    ),
            )
    }

    fn render_session(&self, session: &HistorySession, cx: &mut Context<Self>) -> impl IntoElement {
        let id = session.id;
        let path = session.path.clone();
        let parent = session
            .path
            .parent()
            .map(|path| path.display().to_string())
            .unwrap_or_default();
        let protected_reason = self.protection_reason(session);
        let confirming = self.confirming_delete_id == Some(id);
        let deleting = self.deleting_id == Some(id);
        let query = if session.query_text.is_empty() {
            crate::tr!("无保存查询", "No saved query").to_string()
        } else {
            crate::tr_args!(
                "查询：{}",
                "Query: {}",
                history_query_summary(&session.query_text)
            )
        };
        let position = session
            .selected_row
            .map(|row| crate::tr_args!("第 {} 行", "Line {}", row + 1))
            .unwrap_or_else(|| crate::tr!("无保存行", "No saved line").to_string());
        let open_tooltip = format!(
            "{}\n{} · {} 个标记 · revision {}\n{}",
            session.path.display(),
            position,
            session.marked_rows_count,
            session.revision,
            query,
        );

        h_flex()
            .id(("history-session", id.unsigned_abs()))
            .w_full()
            .min_w_0()
            .h(rems(HISTORY_FILE_ROW_HEIGHT_REMS))
            .flex_none()
            .gap_3()
            .px_5()
            .hover(|row| row.bg(cx.theme().tokens.list_hover))
            .child(
                div()
                    .w_4()
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        svg()
                            .data(include_bytes!(
                                "../assets/icons/document-text-20-regular.svg"
                            ))
                            .size(rems(1.))
                            .text_color(cx.theme().primary),
                    ),
            )
            .child(
                div()
                    .w(rems(HISTORY_FILE_NAME_WIDTH_REMS))
                    .min_w_0()
                    .flex_shrink_1()
                    .truncate()
                    .text_sm()
                    .child(history_session_title(session)),
            )
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .truncate()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(parent),
            )
            .child(
                div()
                    .w(rems(HISTORY_FILE_TIME_WIDTH_REMS))
                    .flex_none()
                    .text_right()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format_history_time(session.last_opened_at)),
            )
            .child(
                h_flex()
                    .w(rems(HISTORY_FILE_ACTIONS_WIDTH_REMS))
                    .flex_none()
                    .justify_end()
                    .gap_1()
                    .child(
                        Button::new(("history-open-session", id.unsigned_abs()))
                            .xsmall()
                            .ghost()
                            .text_color(cx.theme().primary)
                            .label(crate::tr!("打开", "Open"))
                            .tooltip(open_tooltip)
                            .disabled(deleting)
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.emit(HistoryDialogEvent::Open(path.clone()));
                            })),
                    )
                    .when(confirming, |actions| {
                        actions
                            .child(
                                Button::new(("history-cancel-delete", id.unsigned_abs()))
                                    .xsmall()
                                    .ghost()
                                    .label(crate::tr!("取消", "Cancel"))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.confirming_delete_id = None;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new(("history-confirm-delete", id.unsigned_abs()))
                                    .xsmall()
                                    .danger()
                                    .label(crate::tr!("删除", "Delete"))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.begin_delete(id, window, cx);
                                    })),
                            )
                    })
                    .when(!confirming, |actions| {
                        actions.child(
                            Button::new(("history-delete-session", id.unsigned_abs()))
                                .xsmall()
                                .ghost()
                                .danger()
                                .label(crate::tr!("删除记录", "Delete entry"))
                                .loading(deleting)
                                .disabled(protected_reason.is_some() || self.deleting_id.is_some())
                                .tooltip(
                                    protected_reason
                                        .map(|reason| {
                                            crate::tr_args!(
                                                "无法删除：{reason}",
                                                "Can’t delete: {reason}"
                                            )
                                        })
                                        .unwrap_or_else(|| {
                                            crate::tr!(
                                                "只删除恢复记录，不删除日志文件",
                                                "Deletes only the recovery entry, not the log file"
                                            )
                                            .into()
                                        }),
                                )
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.confirming_delete_id = Some(id);
                                    cx.notify();
                                })),
                        )
                    }),
            )
    }
}

impl EventEmitter<HistoryDialogEvent> for HistoryDialog {}

impl Render for HistoryDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _performance_scope = crate::ui_performance::scope("HistoryDialog::render");
        let filter = self.filter.read(cx).value().to_lowercase();
        let visible_sessions = self
            .sessions
            .iter()
            .filter(|session| {
                filter.is_empty()
                    || session
                        .path
                        .to_string_lossy()
                        .to_lowercase()
                        .contains(&filter)
                    || session.query_text.to_lowercase().contains(&filter)
            })
            .collect::<Vec<_>>();
        let visible_count = visible_sessions.len();
        let visible_temporary_results = self
            .temporary_results
            .iter()
            .filter(|result| {
                filter.is_empty()
                    || result
                        .path
                        .to_string_lossy()
                        .to_lowercase()
                        .contains(&filter)
            })
            .collect::<Vec<_>>();
        let visible_temporary_count = visible_temporary_results.len();
        let deletable_temporary_paths = self
            .temporary_results
            .iter()
            .filter(|result| !self.open_paths.contains(&result.path))
            .map(|result| result.path.clone())
            .collect::<Vec<_>>();

        v_flex()
            .id("history-dialog-content")
            .size_full()
            .min_h_0()
            .overflow_hidden()
            .gap_3()
            .child(
                h_flex()
                    .flex_none()
                    .gap_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .pb_2()
                    .child(
                        Button::new("history-files-category")
                            .small()
                            .ghost()
                            .label(crate::tr_args!(
                                "文件记录 ({})",
                                "File entries ({})",
                                self.sessions.len()
                            ))
                            .selected(self.category == HistoryCategory::Files)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.category = HistoryCategory::Files;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("history-temporary-category")
                            .small()
                            .ghost()
                            .label(crate::tr_args!(
                                "临时搜索结果 ({})",
                                "Temporary search results ({})",
                                self.temporary_results.len()
                            ))
                            .selected(self.category == HistoryCategory::TemporaryResults)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.category = HistoryCategory::TemporaryResults;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .id("history-filter")
                    .w_full()
                    .flex_none()
                    .child(Input::new(&self.filter)),
            )
            .when(self.category == HistoryCategory::Files, |dialog| {
                dialog
                    .child(
                        h_flex()
                            .flex_none()
                            .justify_between()
                            .gap_3()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(div().min_w_0().truncate().child(format!(
                                "显示 {visible_count} / {} 条记录 · 数据库 {} · 共 {} 个会话",
                                self.sessions.len(),
                                format_temporary_size(self.database_info.byte_size),
                                self.database_info.session_count,
                            )))
                            .child(
                                h_flex()
                                    .flex_none()
                                    .gap_2()
                                    .child(crate::tr!(
                                        "删除记录不会删除日志文件",
                                        "Deleting entries does not delete log files"
                                    ))
                                    .child(
                                        Button::new("clear-file-history")
                                            .small()
                                            .ghost()
                                            .label(crate::tr!("清除历史…", "Clear history…"))
                                            .disabled(self.sessions.is_empty())
                                            .on_click(cx.listener(|_, _, _, cx| {
                                                cx.emit(HistoryDialogEvent::ClearHistory);
                                            })),
                                    ),
                            ),
                    )
                    .child(
                        h_flex()
                            .id("history-session-list")
                            .w_full()
                            .flex_1()
                            .min_h_0()
                            .rounded(cx.theme().radius_lg)
                            .border_1()
                            .border_color(cx.theme().border)
                            .bg(cx.theme().group_box)
                            .overflow_hidden()
                            .child(
                                v_flex()
                                    .id("history-session-list-viewport")
                                    .h_full()
                                    .flex_1()
                                    .min_w_0()
                                    .min_h_0()
                                    .overflow_y_scroll()
                                    .track_scroll(&self.session_scroll)
                                    .when(visible_sessions.is_empty(), |list| {
                                        list.items_center().justify_center().child(crate::tr!(
                                            "没有符合筛选条件的历史记录",
                                            "No history entries match the filter"
                                        ))
                                    })
                                    .children(visible_sessions.into_iter().map(|session| {
                                        self.render_session(session, cx).into_any_element()
                                    })),
                            )
                            .child(persistent_list_scrollbar(
                                "history-session-list-scrollbar",
                                &self.session_scroll,
                            )),
                    )
            })
            .when(
                self.category == HistoryCategory::TemporaryResults,
                |dialog| {
                    let paths = deletable_temporary_paths.clone();
                    dialog
                        .child(
                            h_flex()
                                .flex_none()
                                .justify_between()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(format!(
                                    "显示 {visible_temporary_count} / {} 个临时结果",
                                    self.temporary_results.len()
                                ))
                                .child(
                                    Button::new("delete-all-temporary-results")
                                        .small()
                                        .danger()
                                        .label(crate::tr_args!(
                                            "清理全部 ({})",
                                            "Clean all ({})",
                                            paths.len()
                                        ))
                                        .loading(self.deleting_temporary_results)
                                        .disabled(
                                            paths.is_empty() || self.deleting_temporary_results,
                                        )
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.confirm_delete_temporary_results(
                                                paths.clone(),
                                                window,
                                                cx,
                                            )
                                        })),
                                ),
                        )
                        .child(
                            h_flex()
                                .id("history-temporary-result-list")
                                .w_full()
                                .flex_1()
                                .min_h_0()
                                .overflow_hidden()
                                .child(
                                    v_flex()
                                        .id("history-temporary-result-list-viewport")
                                        .h_full()
                                        .flex_1()
                                        .min_w_0()
                                        .min_h_0()
                                        .gap_2()
                                        .overflow_y_scroll()
                                        .track_scroll(&self.temporary_result_scroll)
                                        .when(visible_temporary_results.is_empty(), |list| {
                                            list.items_center().justify_center().child(crate::tr!(
                                                "没有符合筛选条件的临时搜索结果",
                                                "No temporary search results match the filter"
                                            ))
                                        })
                                        .children(
                                            visible_temporary_results.into_iter().enumerate().map(
                                                |(index, result)| {
                                                    self.render_temporary_result(result, index, cx)
                                                        .into_any_element()
                                                },
                                            ),
                                        ),
                                )
                                .child(persistent_list_scrollbar(
                                    "history-temporary-result-list-scrollbar",
                                    &self.temporary_result_scroll,
                                )),
                        )
                },
            )
    }
}

fn format_temporary_size(bytes: u64) -> String {
    const KIB: f64 = 1024.;
    const MIB: f64 = KIB * 1024.;
    const GIB: f64 = MIB * 1024.;
    let bytes = bytes as f64;
    if bytes >= GIB {
        format!("{:.1} GiB", bytes / GIB)
    } else if bytes >= MIB {
        format!("{:.1} MiB", bytes / MIB)
    } else if bytes >= KIB {
        format!("{:.1} KiB", bytes / KIB)
    } else {
        format!("{} B", bytes as u64)
    }
}

fn history_session_title(session: &HistorySession) -> SharedString {
    session
        .path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned().into())
        .unwrap_or_else(|| session.path.display().to_string().into())
}

fn format_history_time(timestamp: i64) -> String {
    DateTime::from_timestamp(timestamp, 0)
        .map(|timestamp| {
            timestamp
                .with_timezone(&Local)
                .format("%m/%d %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}

fn history_query_summary(query: &str) -> String {
    const MAX_CHARS: usize = 160;
    let mut characters = query.chars();
    let summary = characters.by_ref().take(MAX_CHARS).collect::<String>();
    if characters.next().is_some() {
        format!("{summary}…")
    } else {
        summary
    }
}
