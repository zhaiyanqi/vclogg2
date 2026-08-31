use gpui::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, Styled as _, Window, div, prelude::FluentBuilder as _,
};
use gpui_component::{
    ActiveTheme as _,
    input::{Input, InputState},
    v_flex,
};

pub struct RenameTabDialog {
    input: Entity<InputState>,
    show_error: bool,
}

impl RenameTabDialog {
    pub fn new(initial_title: &str, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(initial_title)
                .placeholder(crate::tr!("输入标签名称", "Enter tab name"))
        });
        Self {
            input,
            show_error: false,
        }
    }

    pub fn input(&self) -> Entity<InputState> {
        self.input.clone()
    }

    pub fn title(&self, cx: &gpui::App) -> Option<String> {
        let title = self.input.read(cx).value().trim().to_string();
        (!title.is_empty()).then_some(title)
    }

    pub fn show_validation_error(&mut self, cx: &mut Context<Self>) {
        self.show_error = true;
        cx.notify();
    }
}

impl Render for RenameTabDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _performance_scope = crate::ui_performance::scope("RenameTabDialog::render");
        v_flex()
            .id("rename-tab-dialog-content")
            .w_96()
            .max_w_full()
            .gap_2()
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(crate::tr!(
                        "只修改标签显示名称，不会重命名磁盘上的日志文件",
                        "Only the displayed tab name changes; the log file on disk is not renamed"
                    )),
            )
            .child(Input::new(&self.input).w_full())
            .when(self.show_error, |this| {
                this.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().danger)
                        .child(crate::tr!("标签名称不能为空", "Tab name can’t be empty")),
                )
            })
    }
}
