use std::{collections::BTreeSet, path::PathBuf};

use chrono::{DateTime, Local};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    dialog::{Dialog, DialogFooter},
    h_flex, v_flex,
};
use gpui_kit::{
    App, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div, relative, rems,
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

    pub(crate) fn configure_dialog(dialog: Dialog, picker: Entity<Self>, cx: &mut App) -> Dialog {
        dialog
            .title(crate::tr!(
                "参与多标签搜索的文件",
                "Files in multi-tab search"
            ))
            .close_button(false)
            .child(picker)
            .footer(
                DialogFooter::new()
                    .child(crate::dialog_focus::dialog_cancel_action(
                        "global-search-files-cancel-action",
                        Button::new("global-search-files-cancel")
                            .label(crate::tr!("取消", "Cancel")),
                        cx,
                    ))
                    .child(crate::dialog_focus::dialog_confirm_action(
                        "global-search-files-save-action",
                        Button::new("global-search-files-save")
                            .primary()
                            .label(crate::tr!("保存", "Save")),
                        cx,
                    )),
            )
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

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    use gpui_kit::component::{Root, Theme, WindowExt as _};
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{AppContext as _, TestAppContext, px, size};

    use super::*;

    struct Empty;
    impl Render for Empty {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
        }
    }

    fn open_picker(saved: Rc<RefCell<Vec<BTreeSet<u64>>>>, window: &mut Window, cx: &mut App) {
        let picker = cx.new(|_| {
            GlobalSearchFilesDialog::new(
                (1..=30)
                    .map(|id| GlobalSearchFileOption {
                        document_id: id,
                        title: format!("日志 {id}.log").into(),
                        path: PathBuf::from(format!("/logs/{id}.log")),
                        opened_at: 0,
                        selected: id == 1,
                    })
                    .collect(),
            )
        });
        let width = window.viewport_size().width - window.rem_size() * 2.;
        window.open_dialog(cx, move |dialog, _, cx| {
            let picker_for_save = picker.clone();
            let saved = saved.clone();
            GlobalSearchFilesDialog::configure_dialog(dialog.w(width), picker.clone(), cx).on_ok(
                move |_, _, cx| {
                    saved
                        .borrow_mut()
                        .push(picker_for_save.read(cx).selected_document_ids());
                    true
                },
            )
        });
    }

    #[gpui_kit::test]
    fn footer_stays_visible_and_save_and_cancel_keep_their_contract(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        for (width, height, font_size) in [(1280., 800., 15.), (800., 480., 15.), (800., 480., 20.)]
        {
            cx.update(|cx| Theme::update(cx, |theme| theme.font_size = px(font_size)));
            let handle = cx.open_window(size(px(width), px(height)), |window, cx| {
                let view = cx.new(|_| Empty);
                Root::new(view, window, cx)
            });
            let saved = Rc::new(RefCell::new(Vec::new()));
            cx.update_window(handle.into(), |_, window, cx| {
                open_picker(saved.clone(), window, cx);
                window.render_frame(cx);
                for id in ["global-search-files-cancel", "global-search-files-save"] {
                    let button = window.find(id);
                    assert!(button.visible(), "{id} must be visible");
                    let bounds = button.bounds();
                    assert!(
                        bounds.top() >= px(0.) && bounds.bottom() <= px(height),
                        "{id} outside {width}x{height}: {bounds:?}"
                    );
                }
                window.click("global-search-clear-files", cx);
                window.click("global-search-files-save", cx);
            })
            .unwrap();
            cx.run_until_parked();
            assert_eq!(
                *saved.borrow(),
                vec![BTreeSet::new()],
                "saving an empty selection is supported"
            );
            cx.update_window(handle.into(), |_, window, cx| {
                assert!(!window.has_active_dialog(cx));
                open_picker(saved.clone(), window, cx);
                window.render_frame(cx);
                window.click("global-search-select-all-files", cx);
                window.click("global-search-files-cancel", cx);
            })
            .unwrap();
            cx.run_until_parked();
            cx.update_window(handle.into(), |_, window, cx| {
                assert!(!window.has_active_dialog(cx));
            })
            .unwrap();
            assert_eq!(
                saved.borrow().len(),
                1,
                "Cancel must not commit the edited selection"
            );
        }
    }
}
