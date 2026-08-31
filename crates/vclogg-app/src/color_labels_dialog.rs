use gpui::{
    AppContext as _, Context, Entity, Focusable as _, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, Rgba, StatefulInteractiveElement as _, Styled as _, Subscription,
    Window, div, prelude::FluentBuilder as _, rgb,
};
use gpui_component::{
    ActiveTheme as _, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    color_picker::{ColorPicker, ColorPickerEvent, ColorPickerState},
    h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};

use crate::color_labels::{ColorLabel, color_with_alpha, default_color_labels};

struct ColorLabelDraft {
    id: String,
    name: Entity<InputState>,
    color: Entity<ColorPickerState>,
}

pub struct ColorLabelsDialog {
    rows: Vec<ColorLabelDraft>,
    next_custom_id: u64,
    _subscriptions: Vec<Subscription>,
}

impl ColorLabelsDialog {
    pub fn new(labels: Vec<ColorLabel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            rows: Vec::with_capacity(labels.len()),
            next_custom_id: 1,
            _subscriptions: Vec::new(),
        };
        for label in labels {
            this.push_label(label, window, cx);
        }
        this
    }

    pub fn labels(&self, cx: &gpui::App) -> Result<Vec<ColorLabel>, String> {
        self.rows
            .iter()
            .map(|row| {
                let name = row.name.read(cx).value().trim().to_string();
                if name.is_empty() {
                    return Err(crate::tr!(
                        "颜色标签名称不能为空",
                        "Color label name can’t be empty"
                    )
                    .to_string());
                }
                let color = row.color.read(cx).value().ok_or_else(|| {
                    crate::tr_args!("“{name}”尚未选择颜色", "No color is selected for “{name}”")
                })?;
                let rgba = u32::from(Rgba::from(color));
                Ok(ColorLabel {
                    id: row.id.clone(),
                    name,
                    color: rgba >> 8,
                    alpha: (rgba & 0xff) as u8,
                })
            })
            .collect()
    }

    fn push_label(&mut self, label: ColorLabel, window: &mut Window, cx: &mut Context<Self>) {
        let name = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(crate::tr!("标签名称", "Label name"))
                .default_value(label.localized_name())
        });
        let color = cx.new(|cx| {
            ColorPickerState::new(window, cx)
                .default_value(color_with_alpha(label.color, label.alpha))
        });
        self._subscriptions
            .push(cx.subscribe(&name, |_, _, _: &InputEvent, cx| cx.notify()));
        self._subscriptions
            .push(cx.subscribe(&color, |_, _, _: &ColorPickerEvent, cx| cx.notify()));
        self.rows.push(ColorLabelDraft {
            id: label.id,
            name,
            color,
        });
    }

    fn add_label(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let id = loop {
            let candidate = format!("color-label-custom-{}", self.next_custom_id);
            self.next_custom_id = self.next_custom_id.saturating_add(1);
            if self.rows.iter().all(|row| row.id != candidate) {
                break candidate;
            }
        };
        let defaults = default_color_labels();
        let default = &defaults[self.rows.len() % defaults.len()];
        self.push_label(
            ColorLabel {
                id,
                name: crate::tr_args!("颜色标签{}", "Color label {}", self.rows.len() + 1),
                color: default.color,
                alpha: default.alpha,
            },
            window,
            cx,
        );
        if let Some(row) = self.rows.last() {
            let focus = row.name.read(cx).focus_handle(cx);
            window.defer(cx, move |window, cx| focus.focus(window, cx));
        }
        cx.notify();
    }

    fn remove_label(&mut self, id: &str, cx: &mut Context<Self>) {
        self.rows.retain(|row| row.id != id);
        cx.notify();
    }
}

impl Render for ColorLabelsDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _performance_scope = crate::ui_performance::scope("ColorLabelsDialog::render");
        let dialog = cx.entity();
        v_flex()
            .id("color-labels-dialog-content")
            .size_full()
            .min_h_0()
            .overflow_hidden()
            .gap_4()
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        crate::tr!("标签顺序决定“轮换颜色”的顺序；修改颜色会立即更新使用该标签的文件规则。", "Label order determines how Cycle color works. Changing a color immediately updates file rules that use the label."),
                    ),
            )
            .child(
                v_flex()
                    .id("color-labels-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .gap_1()
                    .when(self.rows.is_empty(), |this| {
                        this.child(
                            v_flex()
                                .items_center()
                                .gap_1()
                                .py_8()
                                .text_color(cx.theme().muted_foreground)
                                .child(crate::tr!("尚未创建颜色标签", "No color labels created"))
                                .child(div().text_sm().child(crate::tr!("添加标签后即可继续轮换颜色", "Add a label to use Cycle color"))),
                        )
                    })
                    .children(self.rows.iter().map(|row| {
                        let row_id = row.id.clone();
                        let remove_id = row.id.clone();
                        let dialog = dialog.clone();
                        let color = row
                            .color
                            .read(cx)
                            .displayed_color()
                            .unwrap_or_else(|| rgb(0).into());
                        h_flex()
                            .id(format!("color-label-row-{row_id}"))
                            .items_center()
                            .gap_3()
                            .px_3()
                            .py_2()
                            .rounded(cx.theme().radius)
                            .border_1()
                            .border_color(cx.theme().border)
                            .bg(cx.theme().background)
                            .child(
                                div()
                                    .size_7()
                                    .flex_shrink_0()
                                    .rounded(cx.theme().radius / 2.)
                                    .border_1()
                                    .border_color(cx.theme().input)
                                    .bg(color),
                            )
                            .child(
                                div()
                                    .min_w_0()
                                    .flex_1()
                                    .child(Input::new(&row.name).small()),
                            )
                            .child(ColorPicker::new(&row.color).small())
                            .child(
                                Button::new(format!("remove-color-label-{remove_id}"))
                                    .small()
                                    .ghost()
                                    .icon(IconName::Delete)
                                    .tooltip(crate::tr!("删除颜色标签", "Delete color label"))
                                    .on_click(move |_, _, cx| {
                                        dialog.update(cx, |this, cx| {
                                            this.remove_label(&remove_id, cx)
                                        });
                                    }),
                            )
                    })),
            )
            .child(
                h_flex().flex_none().justify_between().child(
                    Button::new("add-color-label")
                        .small()
                        .icon(IconName::Plus)
                        .label(crate::tr!("添加标签", "Add label"))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.add_label(window, cx);
                        })),
                ),
            )
    }
}
