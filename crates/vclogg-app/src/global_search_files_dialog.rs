use std::{collections::BTreeSet, path::PathBuf};

use chrono::{DateTime, Local};
use gpui::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, relative, rems,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    h_flex, v_flex,
};

const GLOBAL_SEARCH_FILE_ROW_HEIGHT_REMS: f32 = 3.2;
const GLOBAL_SEARCH_FILE_IDENTITY_WIDTH_REMS: f32 = 22.;
const GLOBAL_SEARCH_FILE_TIME_WIDTH_REMS: f32 = 7.;

#[derive(Clone)]
pub struct GlobalSearchFileOption {
    pub document_id: u64,
    pub title: SharedString,
    pub path: PathBuf,
    pub opened_at: i64,
    pub selected: bool,
}

pub struct GlobalSearchFilesDialog {
    files: Vec<GlobalSearchFileOption>,
}

impl GlobalSearchFilesDialog {
    pub fn new(files: Vec<GlobalSearchFileOption>) -> Self {
        Self { files }
    }

    pub fn selected_document_ids(&self) -> BTreeSet<u64> {
        self.files
            .iter()
            .filter(|file| file.selected)
            .map(|file| file.document_id)
            .collect()
    }
}

impl Render for GlobalSearchFilesDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _performance_scope = crate::ui_performance::scope("GlobalSearchFilesDialog::render");
        let selected_count = self.files.iter().filter(|file| file.selected).count();
        let total_count = self.files.len();

        v_flex()
            .id("global-search-files-dialog")
            .w_full()
            .min_w_0()
            .gap_3()
            .child(
                h_flex()
                    .justify_between()
                    .gap_3()
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(crate::tr_args!("已选择 {selected_count} / {total_count} 个文件", "Selected {selected_count} of {total_count} files")),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("global-search-select-all-files")
                                    .small()
                                    .ghost()
                                    .label(crate::tr!("全选", "Select all"))
                                    .disabled(selected_count == total_count)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        for file in &mut this.files {
                                            file.selected = true;
                                        }
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("global-search-clear-files")
                                    .small()
                                    .ghost()
                                    .label(crate::tr!("清空", "Clear"))
                                    .disabled(selected_count == 0)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        for file in &mut this.files {
                                            file.selected = false;
                                        }
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
            .child(
                v_flex()
                    .id("global-search-file-list")
                    .w_full()
                    .h_80()
                    .min_h_0()
                    .rounded(cx.theme().radius_lg)
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().group_box)
                    .overflow_y_scroll()
                    .children(self.files.iter().map(|file| {
                        let document_id = file.document_id;
                        let opened_at = format_opened_at(file.opened_at);
                        h_flex()
                            .id(("global-search-file-row", document_id))
                            .w_full()
                            .min_w_0()
                            .h(rems(GLOBAL_SEARCH_FILE_ROW_HEIGHT_REMS))
                            .flex_none()
                            .gap_3()
                            .px_5()
                            .hover(|row| row.bg(cx.theme().tokens.list_hover))
                            .child(
                                Checkbox::new(("global-search-file", document_id))
                                    .small()
                                    .w(rems(GLOBAL_SEARCH_FILE_IDENTITY_WIDTH_REMS))
                                    .min_w_0()
                                    .flex_none()
                                    .items_center()
                                    .checked(file.selected)
                                    .child(
                                        div().min_w_0().py_1().child(
                                            div()
                                                .w_full()
                                                .truncate()
                                                .line_height(relative(1.4))
                                                .child(file.title.clone()),
                                        ),
                                    )
                                    .on_click(cx.listener(move |this, selected: &bool, _, cx| {
                                        if let Some(file) = this
                                            .files
                                            .iter_mut()
                                            .find(|file| file.document_id == document_id)
                                        {
                                            file.selected = *selected;
                                            cx.notify();
                                        }
                                    })),
                            )
                            .child(
                                div()
                                    .min_w_0()
                                    .flex_1()
                                    .py_1()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(
                                        div()
                                            .w_full()
                                            .truncate()
                                            .line_height(relative(1.4))
                                            .child(file.path.display().to_string()),
                                    ),
                            )
                            .child(
                                div()
                                    .w(rems(GLOBAL_SEARCH_FILE_TIME_WIDTH_REMS))
                                    .flex_none()
                                    .py_1()
                                    .line_height(relative(1.4))
                                    .text_right()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(opened_at),
                            )
                    })),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(crate::tr!("可以保存空选择；没有参与文件时，全局搜索不可执行。", "An empty selection can be saved. Global search is unavailable when no files participate.")),
            )
    }
}

fn format_opened_at(timestamp: i64) -> String {
    DateTime::from_timestamp(timestamp, 0)
        .map(|timestamp| {
            timestamp
                .with_timezone(&Local)
                .format("%m/%d %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}
