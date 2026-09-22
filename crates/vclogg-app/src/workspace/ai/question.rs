use super::*;
use gpui_kit::component::input::Textarea;

impl AiPanel {
    pub(super) fn answer_question(
        &mut self,
        option_id: Option<String>,
        answer: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(question) = self.pending_question.as_ref() else {
            return;
        };
        let Some(run) = self.run.as_ref() else {
            return;
        };
        let answer = answer.trim().to_owned();
        if answer.is_empty() {
            self.error = crate::tr!("请输入回答", "Enter an answer").into();
            cx.notify();
            return;
        }
        let result = ToolResult::ok(json!({
            "option_id": option_id,
            "answer": answer,
        }));
        match run.replies.try_send((question.call_id.clone(), result)) {
            Ok(()) => {
                self.pending_question = None;
                self.error.clear();
                self.progress = crate::tr!("正在继续", "Continuing").into();
                self.question_input
                    .update(cx, |input, cx| input.set_value("", window, cx));
            }
            Err(error) => self.error = error.to_string(),
        }
        cx.notify();
    }

    pub(super) fn submit_question_answer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .pending_question
            .as_ref()
            .is_some_and(|question| !question.allow_free_text)
        {
            return;
        }
        let answer = self.question_input.read(cx).value().to_string();
        self.answer_question(None, answer, window, cx);
    }

    pub(super) fn render_pending_question(&self, cx: &Context<Self>) -> AnyElement {
        let Some(question) = self.pending_question.as_ref() else {
            return div().into_any_element();
        };
        let options = question.options.clone();
        let mut card = v_flex()
            .gap_2()
            .p_3()
            .rounded(cx.theme().radius_lg)
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().muted)
            .child(
                div()
                    .text_sm()
                    .font_semibold()
                    .child(question.question.clone()),
            );
        if !question.detail.is_empty() {
            card = card.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(question.detail.clone()),
            );
        }
        if question.allow_free_text {
            card = card.child(
                Textarea::new(&self.question_input)
                    .aria_label(crate::tr!("问题回答", "Question answer")),
            );
        }
        card = card.child(h_flex().gap_2().flex_wrap().children(
            options.into_iter().enumerate().map(|(ix, option)| {
                let id = option.id.clone();
                let label = option.label.clone();
                let answer = option.label.clone();
                let mut button =
                    Button::new(SharedString::from(format!("ai-question-option-{id}")))
                        .small()
                        .text_label(label)
                        .tooltip(option.description)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.answer_question(Some(id.clone()), answer.clone(), window, cx)
                        }));
                if ix == 0 {
                    button = button.primary();
                }
                button
            }),
        ));
        card.into_any_element()
    }
}
