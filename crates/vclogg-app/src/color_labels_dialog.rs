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
    switch::Switch,
    tab::{Tab, TabBar},
    v_flex,
};

use crate::color_labels::{
    ColorLabel, LogLevelColorRule, color_with_alpha, default_color_labels, default_log_level_rules,
};

struct LogLevelDraft {
    id: String,
    keyword: Entity<InputState>,
    text_color: Entity<ColorPickerState>,
    background_color: Entity<ColorPickerState>,
}

struct ColorLabelDraft {
    id: String,
    name: Entity<InputState>,
    text_color: Entity<ColorPickerState>,
    background_color: Entity<ColorPickerState>,
}

pub struct LogColoringConfig {
    pub highlight_log_levels: bool,
    pub log_level_rules: Vec<LogLevelColorRule>,
    pub labels: Vec<ColorLabel>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum LogColoringSection {
    #[default]
    LogLevels,
    ColorLabels,
}

pub struct ColorLabelsDialog {
    active_section: LogColoringSection,
    highlight_log_levels: bool,
    log_level_rows: Vec<LogLevelDraft>,
    label_rows: Vec<ColorLabelDraft>,
    next_log_level_id: u64,
    next_custom_label_id: u64,
    _subscriptions: Vec<Subscription>,
}

impl ColorLabelsDialog {
    pub fn new(
        highlight_log_levels: bool,
        log_level_rules: Vec<LogLevelColorRule>,
        labels: Vec<ColorLabel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self {
            active_section: LogColoringSection::default(),
            highlight_log_levels,
            log_level_rows: Vec::with_capacity(log_level_rules.len()),
            label_rows: Vec::with_capacity(labels.len()),
            next_log_level_id: 1,
            next_custom_label_id: 1,
            _subscriptions: Vec::new(),
        };
        for rule in log_level_rules {
            this.push_log_level_rule(rule, window, cx);
        }
        for label in labels {
            this.push_label(label, window, cx);
        }
        this
    }

    pub fn config(&self, cx: &gpui::App) -> Result<LogColoringConfig, String> {
        let log_level_rules = self
            .log_level_rows
            .iter()
            .map(|row| {
                let keyword = row.keyword.read(cx).value().trim().to_string();
                if keyword.is_empty() {
                    return Err(crate::tr!(
                        "日志级别关键词不能为空",
                        "Log-level keyword can’t be empty"
                    )
                    .to_string());
                }
                let (text_color, text_alpha) = picker_value(
                    &row.text_color,
                    crate::tr_args!(
                        "“{keyword}”尚未选择文字颜色",
                        "No text color is selected for “{keyword}”"
                    ),
                    cx,
                )?;
                let (background_color, background_alpha) = picker_value(
                    &row.background_color,
                    crate::tr_args!(
                        "“{keyword}”尚未选择背景色",
                        "No background color is selected for “{keyword}”"
                    ),
                    cx,
                )?;
                Ok(LogLevelColorRule {
                    id: row.id.clone(),
                    keyword,
                    text_color,
                    text_alpha,
                    background_color,
                    background_alpha,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let labels = self
            .label_rows
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
                let (text_color, text_alpha) = picker_value(
                    &row.text_color,
                    crate::tr_args!(
                        "“{name}”尚未选择文字颜色",
                        "No text color is selected for “{name}”"
                    ),
                    cx,
                )?;
                let (background_color, background_alpha) = picker_value(
                    &row.background_color,
                    crate::tr_args!(
                        "“{name}”尚未选择背景色",
                        "No background color is selected for “{name}”"
                    ),
                    cx,
                )?;
                Ok(ColorLabel {
                    id: row.id.clone(),
                    name,
                    text_color,
                    text_alpha,
                    background_color,
                    background_alpha,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(LogColoringConfig {
            highlight_log_levels: self.highlight_log_levels,
            log_level_rules,
            labels,
        })
    }

    fn push_log_level_rule(
        &mut self,
        rule: LogLevelColorRule,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let keyword = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(crate::tr!("关键词", "Keyword"))
                .default_value(rule.keyword)
        });
        let text_color = color_picker(rule.text_color, rule.text_alpha, window, cx);
        let background_color =
            color_picker(rule.background_color, rule.background_alpha, window, cx);
        self.subscribe_input(&keyword, cx);
        self.subscribe_color(&text_color, window, cx);
        self.subscribe_color(&background_color, window, cx);
        self.log_level_rows.push(LogLevelDraft {
            id: rule.id,
            keyword,
            text_color,
            background_color,
        });
    }

    fn push_label(&mut self, label: ColorLabel, window: &mut Window, cx: &mut Context<Self>) {
        let name = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(crate::tr!("标签名称", "Label name"))
                .default_value(label.localized_name())
        });
        let text_color = color_picker(label.text_color, label.text_alpha, window, cx);
        let background_color =
            color_picker(label.background_color, label.background_alpha, window, cx);
        self.subscribe_input(&name, cx);
        self.subscribe_color(&text_color, window, cx);
        self.subscribe_color(&background_color, window, cx);
        self.label_rows.push(ColorLabelDraft {
            id: label.id,
            name,
            text_color,
            background_color,
        });
    }

    fn subscribe_input(&mut self, input: &Entity<InputState>, cx: &mut Context<Self>) {
        self._subscriptions
            .push(cx.subscribe(input, |_, _, _: &InputEvent, cx| cx.notify()));
    }

    fn subscribe_color(
        &mut self,
        color: &Entity<ColorPickerState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self._subscriptions.push(cx.subscribe_in(
            color,
            window,
            |_, picker, _: &ColorPickerEvent, window, cx| {
                crate::dialog_focus::restore_color_picker_trigger(picker, window, cx);
                cx.notify();
            },
        ));
    }

    fn add_log_level_rule(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let id = loop {
            let candidate = format!("log-level-custom-{}", self.next_log_level_id);
            self.next_log_level_id = self.next_log_level_id.saturating_add(1);
            if self.log_level_rows.iter().all(|row| row.id != candidate) {
                break candidate;
            }
        };
        let defaults = default_log_level_rules();
        let mut rule = defaults[self.log_level_rows.len() % defaults.len()].clone();
        rule.id = id;
        rule.keyword.clear();
        self.push_log_level_rule(rule, window, cx);
        if let Some(row) = self.log_level_rows.last() {
            let focus = row.keyword.read(cx).focus_handle(cx);
            window.defer(cx, move |window, cx| focus.focus(window, cx));
        }
        cx.notify();
    }

    fn add_label(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let id = loop {
            let candidate = format!("color-label-custom-{}", self.next_custom_label_id);
            self.next_custom_label_id = self.next_custom_label_id.saturating_add(1);
            if self.label_rows.iter().all(|row| row.id != candidate) {
                break candidate;
            }
        };
        let defaults = default_color_labels();
        let mut label = defaults[self.label_rows.len() % defaults.len()].clone();
        label.id = id;
        label.name = crate::tr_args!("颜色标签{}", "Color label {}", self.label_rows.len() + 1);
        self.push_label(label, window, cx);
        if let Some(row) = self.label_rows.last() {
            let focus = row.name.read(cx).focus_handle(cx);
            window.defer(cx, move |window, cx| focus.focus(window, cx));
        }
        cx.notify();
    }

    fn render_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        TabBar::new("log-coloring-tabs")
            .w_full()
            .flex_none()
            .small()
            .underline()
            .selected_index(match self.active_section {
                LogColoringSection::LogLevels => 0,
                LogColoringSection::ColorLabels => 1,
            })
            .border_b_1()
            .border_color(cx.theme().border)
            .child(Tab::new().label(crate::tr!("日志级别", "Log levels")))
            .child(Tab::new().label(crate::tr!("颜色标签", "Color labels")))
            .on_click(cx.listener(|this, index: &usize, _, cx| {
                this.active_section = if *index == 0 {
                    LogColoringSection::LogLevels
                } else {
                    LogColoringSection::ColorLabels
                };
                cx.notify();
            }))
    }

    fn render_log_levels(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let dialog = cx.entity();
        v_flex()
            .size_full()
            .min_h_0()
            .gap_3()
            .child(
                h_flex()
                    .justify_between()
                    .gap_4()
                    .child(
                        v_flex()
                            .min_w_0()
                            .flex_1()
                            .gap_1()
                            .child(
                                div()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child(crate::tr!("日志级别规则", "Log-level rules")),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(crate::tr!(
                                        "按 ASCII 单词匹配关键词；靠后的规则优先。",
                                        "Keywords match ASCII words; later rules take precedence."
                                    )),
                            ),
                    )
                    .child(
                        h_flex()
                            .flex_none()
                            .gap_3()
                            .child(
                                Switch::new("log-coloring-enabled")
                                    .small()
                                    .checked(self.highlight_log_levels)
                                    .label(crate::tr!("启用着色", "Enable coloring"))
                                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                        this.highlight_log_levels = *checked;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("add-log-level")
                                    .small()
                                    .icon(IconName::Plus)
                                    .label(crate::tr!("添加规则", "Add rule"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.add_log_level_rule(window, cx);
                                    })),
                            ),
                    ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .rounded(cx.theme().radius)
                    .border_1()
                    .border_color(cx.theme().border)
                    .child(
                        h_flex()
                            .flex_none()
                            .gap_3()
                            .px_3()
                            .py_2()
                            .bg(cx.theme().muted)
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(
                                div()
                                    .min_w_0()
                                    .flex_1()
                                    .child(crate::tr!("关键词", "Keyword")),
                            )
                            .child(
                                div()
                                    .w_24()
                                    .flex_none()
                                    .text_center()
                                    .child(crate::tr!("预览", "Preview")),
                            )
                            .child(
                                div()
                                    .w_20()
                                    .flex_none()
                                    .text_center()
                                    .child(crate::tr!("文字颜色", "Text")),
                            )
                            .child(
                                div()
                                    .w_20()
                                    .flex_none()
                                    .text_center()
                                    .child(crate::tr!("背景色", "Background")),
                            )
                            .child(div().w_8().flex_none()),
                    )
                    .child(
                        v_flex()
                            .id("log-level-rules-scroll")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .when(self.log_level_rows.is_empty(), |this| {
                                this.child(
                                    v_flex()
                                        .size_full()
                                        .items_center()
                                        .justify_center()
                                        .gap_1()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(crate::tr!(
                                            "尚未配置日志级别",
                                            "No log levels configured"
                                        ))
                                        .child(div().text_sm().child(crate::tr!(
                                            "使用“添加规则”创建第一条规则",
                                            "Use Add rule to create the first rule"
                                        ))),
                                )
                            })
                            .children(self.log_level_rows.iter().map(|row| {
                                let row_id = row.id.clone();
                                let remove_id = row.id.clone();
                                let dialog = dialog.clone();
                                let text_color = row
                                    .text_color
                                    .read(cx)
                                    .displayed_color()
                                    .unwrap_or_else(|| rgb(0).into());
                                let background = row
                                    .background_color
                                    .read(cx)
                                    .displayed_color()
                                    .unwrap_or_else(|| rgb(0).into());
                                h_flex()
                                    .id(format!("log-level-color-row-{row_id}"))
                                    .gap_3()
                                    .px_3()
                                    .py_2()
                                    .border_t_1()
                                    .border_color(cx.theme().border)
                                    .child(
                                        div()
                                            .min_w_0()
                                            .flex_1()
                                            .child(Input::new(&row.keyword).small()),
                                    )
                                    .child(
                                        h_flex().w_24().flex_none().justify_center().child(
                                            h_flex()
                                                .h_7()
                                                .w_full()
                                                .justify_center()
                                                .rounded(cx.theme().radius / 2.)
                                                .border_1()
                                                .border_color(cx.theme().border)
                                                .bg(background)
                                                .text_color(text_color)
                                                .text_sm()
                                                .child(crate::tr!("示例日志", "Sample log")),
                                        ),
                                    )
                                    .child(
                                        h_flex()
                                            .w_20()
                                            .flex_none()
                                            .justify_center()
                                            .child(ColorPicker::new(&row.text_color).small()),
                                    )
                                    .child(
                                        h_flex()
                                            .w_20()
                                            .flex_none()
                                            .justify_center()
                                            .child(ColorPicker::new(&row.background_color).small()),
                                    )
                                    .child(
                                        h_flex().w_8().flex_none().justify_end().child(
                                            Button::new(format!("remove-log-level-{remove_id}"))
                                                .small()
                                                .ghost()
                                                .icon(IconName::Delete)
                                                .tooltip(crate::tr!(
                                                    "删除日志级别规则",
                                                    "Delete log-level rule"
                                                ))
                                                .on_click(move |_, _, cx| {
                                                    dialog.update(cx, |this, cx| {
                                                        this.log_level_rows
                                                            .retain(|row| row.id != remove_id);
                                                        cx.notify();
                                                    });
                                                }),
                                        ),
                                    )
                            })),
                    ),
            )
    }

    fn render_color_labels(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let dialog = cx.entity();
        v_flex()
            .size_full()
            .min_h_0()
            .gap_3()
            .child(
                h_flex()
                    .justify_between()
                    .gap_4()
                    .child(
                        v_flex()
                            .min_w_0()
                            .flex_1()
                            .gap_1()
                            .child(
                                div()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child(crate::tr!("颜色标签", "Color labels")),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(crate::tr!(
                                        "列表顺序决定轮换颜色的顺序；修改颜色会同步更新引用该标签的文件规则。",
                                        "List order controls Cycle color. Color changes update file rules that reference the label."
                                    )),
                            ),
                    )
                    .child(
                        Button::new("add-color-label")
                            .small()
                            .flex_none()
                            .icon(IconName::Plus)
                            .label(crate::tr!("添加标签", "Add label"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.add_label(window, cx);
                            })),
                    ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .rounded(cx.theme().radius)
                    .border_1()
                    .border_color(cx.theme().border)
                    .child(
                        h_flex()
                            .flex_none()
                            .gap_3()
                            .px_3()
                            .py_2()
                            .bg(cx.theme().muted)
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(
                                div()
                                    .min_w_0()
                                    .flex_1()
                                    .child(crate::tr!("标签名称", "Label name")),
                            )
                            .child(
                                div()
                                    .w_24()
                                    .flex_none()
                                    .text_center()
                                    .child(crate::tr!("预览", "Preview")),
                            )
                            .child(
                                div()
                                    .w_20()
                                    .flex_none()
                                    .text_center()
                                    .child(crate::tr!("文字颜色", "Text")),
                            )
                            .child(
                                div()
                                    .w_20()
                                    .flex_none()
                                    .text_center()
                                    .child(crate::tr!("背景色", "Background")),
                            )
                            .child(div().w_8().flex_none()),
                    )
                    .child(
                        v_flex()
                            .id("color-labels-scroll")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .when(self.label_rows.is_empty(), |this| {
                                this.child(
                                    v_flex()
                                        .size_full()
                                        .items_center()
                                        .justify_center()
                                        .gap_1()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(crate::tr!(
                                            "尚未创建颜色标签",
                                            "No color labels created"
                                        ))
                                        .child(
                                            div().text_sm().child(crate::tr!(
                                                "使用“添加标签”创建第一个标签",
                                                "Use Add label to create the first label"
                                            )),
                                        ),
                                )
                            })
                            .children(self.label_rows.iter().map(|row| {
                                let row_id = row.id.clone();
                                let remove_id = row.id.clone();
                                let dialog = dialog.clone();
                                let text_color = row
                                    .text_color
                                    .read(cx)
                                    .displayed_color()
                                    .unwrap_or_else(|| rgb(0).into());
                                let background = row
                                    .background_color
                                    .read(cx)
                                    .displayed_color()
                                    .unwrap_or_else(|| rgb(0).into());
                                h_flex()
                                    .id(format!("color-label-row-{row_id}"))
                                    .gap_3()
                                    .px_3()
                                    .py_2()
                                    .border_t_1()
                                    .border_color(cx.theme().border)
                                    .child(
                                        div()
                                            .min_w_0()
                                            .flex_1()
                                            .child(Input::new(&row.name).small()),
                                    )
                                    .child(
                                        h_flex()
                                            .w_24()
                                            .flex_none()
                                            .justify_center()
                                            .child(
                                                h_flex()
                                                    .h_7()
                                                    .w_full()
                                                    .justify_center()
                                                    .rounded(cx.theme().radius / 2.)
                                                    .border_1()
                                                    .border_color(cx.theme().border)
                                                    .bg(background)
                                                    .text_color(text_color)
                                                    .text_sm()
                                                    .child(crate::tr!("示例日志", "Sample log")),
                                            ),
                                    )
                                    .child(
                                        h_flex()
                                            .w_20()
                                            .flex_none()
                                            .justify_center()
                                            .child(ColorPicker::new(&row.text_color).small()),
                                    )
                                    .child(
                                        h_flex()
                                            .w_20()
                                            .flex_none()
                                            .justify_center()
                                            .child(ColorPicker::new(&row.background_color).small()),
                                    )
                                    .child(
                                        h_flex().w_8().flex_none().justify_end().child(
                                            Button::new(format!("remove-color-label-{remove_id}"))
                                                .small()
                                                .ghost()
                                                .icon(IconName::Delete)
                                                .tooltip(crate::tr!(
                                                    "删除颜色标签",
                                                    "Delete color label"
                                                ))
                                                .on_click(move |_, _, cx| {
                                                    dialog.update(cx, |this, cx| {
                                                        this.label_rows
                                                            .retain(|row| row.id != remove_id);
                                                        cx.notify();
                                                    });
                                                }),
                                        ),
                                    )
                            })),
                    ),
            )
    }
}

fn color_picker(
    color: u32,
    alpha: u8,
    window: &mut Window,
    cx: &mut Context<ColorLabelsDialog>,
) -> Entity<ColorPickerState> {
    cx.new(|cx| ColorPickerState::new(window, cx).default_value(color_with_alpha(color, alpha)))
}

fn picker_value(
    picker: &Entity<ColorPickerState>,
    error: String,
    cx: &gpui::App,
) -> Result<(u32, u8), String> {
    let color = picker.read(cx).value().ok_or(error)?;
    let rgba = u32::from(Rgba::from(color));
    Ok((rgba >> 8, (rgba & 0xff) as u8))
}

impl Render for ColorLabelsDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _performance_scope = crate::ui_performance::scope("ColorLabelsDialog::render");
        v_flex()
            .id("log-coloring-dialog-content")
            .size_full()
            .min_h_0()
            .overflow_hidden()
            .gap_3()
            .child(self.render_tabs(cx))
            .child(match self.active_section {
                LogColoringSection::LogLevels => self.render_log_levels(cx).into_any_element(),
                LogColoringSection::ColorLabels => self.render_color_labels(cx).into_any_element(),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    #[gpui::test]
    fn committed_color_restores_focus_to_the_picker_trigger(cx: &mut TestAppContext) {
        cx.update(gpui_component::init);
        let (dialog, cx) = cx.add_window_view(|window, cx| {
            ColorLabelsDialog::new(
                false,
                default_log_level_rules(),
                default_color_labels(),
                window,
                cx,
            )
        });

        let picker = dialog.read_with(cx, |dialog, _| dialog.log_level_rows[0].text_color.clone());
        let popup_input = picker.read_with(cx, |picker, _| picker.hex_input().clone());

        cx.update(|window, cx| {
            picker.update(cx, |picker, cx| picker.set_open(true, cx));
            popup_input.focus_handle(cx).focus(window, cx);
            assert!(popup_input.focus_handle(cx).is_focused(window));

            picker.update(cx, |picker, cx| {
                picker.select_color(rgb(0x12_34_56).into(), window, cx)
            });
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            assert!(
                picker.focus_handle(cx).is_focused(window),
                "closing the nested color popup must return focus to its trigger"
            );
        });
    }
}
