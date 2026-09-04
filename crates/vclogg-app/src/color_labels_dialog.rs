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

pub struct ColorLabelsDialog {
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
        self.subscribe_color(&text_color, cx);
        self.subscribe_color(&background_color, cx);
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
        self.subscribe_color(&text_color, cx);
        self.subscribe_color(&background_color, cx);
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

    fn subscribe_color(&mut self, color: &Entity<ColorPickerState>, cx: &mut Context<Self>) {
        self._subscriptions
            .push(cx.subscribe(color, |_, _, _: &ColorPickerEvent, cx| cx.notify()));
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
        let dialog = cx.entity();
        v_flex()
            .id("log-coloring-dialog-content")
            .size_full()
            .min_h_0()
            .overflow_hidden()
            .gap_4()
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        h_flex()
                            .justify_between()
                            .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child(
                                crate::tr!("日志级别", "Log levels"),
                            ))
                            .child(
                                Switch::new("log-coloring-enabled")
                                    .small()
                                    .checked(self.highlight_log_levels)
                                    .tooltip(crate::tr!(
                                        "启用日志级别着色",
                                        "Enable log-level coloring"
                                    ))
                                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                        this.highlight_log_levels = *checked;
                                        cx.notify();
                                    })),
                            ),
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
                v_flex()
                    .id("log-coloring-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .gap_4()
                    .child(
                        v_flex()
                            .gap_1()
                            .when(self.log_level_rows.is_empty(), |this| {
                                this.child(
                                    div()
                                        .py_4()
                                        .text_center()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(crate::tr!(
                                            "尚未配置日志级别",
                                            "No log levels configured"
                                        )),
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
                                            .min_w_0()
                                            .flex_1()
                                            .child(Input::new(&row.keyword).small()),
                                    )
                                    .child(
                                        div()
                                            .w_20()
                                            .rounded(cx.theme().radius / 2.)
                                            .bg(background)
                                            .text_center()
                                            .text_color(text_color)
                                            .child(crate::tr!("预览", "Preview")),
                                    )
                                    .child(
                                        v_flex()
                                            .gap_1()
                                            .child(div().text_sm().child(crate::tr!("文字", "Text")))
                                            .child(ColorPicker::new(&row.text_color).small()),
                                    )
                                    .child(
                                        v_flex()
                                            .gap_1()
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .child(crate::tr!("背景", "Background")),
                                            )
                                            .child(ColorPicker::new(&row.background_color).small()),
                                    )
                                    .child(
                                        Button::new(format!("remove-log-level-{remove_id}"))
                                            .small()
                                            .ghost()
                                            .icon(IconName::Delete)
                                            .tooltip(crate::tr!(
                                                "删除日志级别",
                                                "Delete log level"
                                            ))
                                            .on_click(move |_, _, cx| {
                                                dialog.update(cx, |this, cx| {
                                                    this.log_level_rows
                                                        .retain(|row| row.id != remove_id);
                                                    cx.notify();
                                                });
                                            }),
                                    )
                            }))
                            .child(
                                Button::new("add-log-level")
                                    .small()
                                    .icon(IconName::Plus)
                                    .label(crate::tr!("添加日志级别", "Add log level"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.add_log_level_rule(window, cx);
                                    })),
                            ),
                    )
                    .child(
                        v_flex()
                            .gap_1()
                            .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child(
                                crate::tr!("颜色标签", "Color labels"),
                            ))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(crate::tr!(
                                        "标签顺序决定“轮换颜色”的顺序；颜色会同步更新使用该标签的文件规则。",
                                        "Label order determines Cycle color order. Colors update file rules that use the label."
                                    )),
                            )
                            .when(self.label_rows.is_empty(), |this| {
                                this.child(
                                    div()
                                        .py_4()
                                        .text_center()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(crate::tr!(
                                            "尚未创建颜色标签",
                                            "No color labels created"
                                        )),
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
                                            .min_w_0()
                                            .flex_1()
                                            .child(Input::new(&row.name).small()),
                                    )
                                    .child(
                                        div()
                                            .w_20()
                                            .rounded(cx.theme().radius / 2.)
                                            .bg(background)
                                            .text_center()
                                            .text_color(text_color)
                                            .child(crate::tr!("预览", "Preview")),
                                    )
                                    .child(
                                        v_flex()
                                            .gap_1()
                                            .child(div().text_sm().child(crate::tr!("文字", "Text")))
                                            .child(ColorPicker::new(&row.text_color).small()),
                                    )
                                    .child(
                                        v_flex()
                                            .gap_1()
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .child(crate::tr!("背景", "Background")),
                                            )
                                            .child(ColorPicker::new(&row.background_color).small()),
                                    )
                                    .child(
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
                                                    this.label_rows.retain(|row| row.id != remove_id);
                                                    cx.notify();
                                                });
                                            }),
                                    )
                            }))
                            .child(
                                Button::new("add-color-label")
                                    .small()
                                    .icon(IconName::Plus)
                                    .label(crate::tr!("添加标签", "Add label"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.add_label(window, cx);
                                    })),
                            ),
                    ),
            )
    }
}
