use gpui::{
    AppContext as _, Context, Entity, Focusable as _, HighlightStyle, InteractiveElement as _,
    IntoElement, ParentElement as _, Render, Rgba, ScrollHandle, StatefulInteractiveElement as _,
    Styled as _, StyledText, Subscription, Window, div, prelude::FluentBuilder as _, rems, rgb,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    color_picker::{ColorPickerEvent, ColorPickerState},
    h_flex,
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu, PopupMenuItem},
    scroll::{ScrollableElement as _, Scrollbar, ScrollbarMode},
    switch::Switch,
    tab::{Tab, TabBar},
    v_flex,
};

use crate::color_labels::{
    ColorLabel, LogLevelColorRule, LogRuleMatch, color_with_alpha, default_color_labels,
    default_log_level_rules,
};

mod groups;
use crate::log_coloring::{LogColoringGroup, LogColoringSettings};
use groups::LogGroupDraft;

struct LogLevelDraft {
    match_kind: LogRuleMatch,
    resolved_config: LogLevelColorRule,
    resolved: std::sync::Arc<crate::color_labels::ResolvedLogLevelRules>,
    match_error: Option<String>,
    id: String,
    keyword: Entity<InputState>,
    keyword_only: bool,
    text_color: Entity<ColorPickerState>,
    background_color: Entity<ColorPickerState>,
    _subscriptions: [Subscription; 3],
}

struct ColorLabelDraft {
    id: String,
    name: Entity<InputState>,
    text_color: Entity<ColorPickerState>,
    background_color: Entity<ColorPickerState>,
    _subscriptions: [Subscription; 3],
}

#[derive(Clone)]
pub struct LogColoringConfig {
    pub(crate) keyword_match_styles: crate::keyword_match_style::KeywordMatchStyles,
    pub(crate) selection_styles: crate::selection_style::SelectionStyles,
    pub highlight_log_levels: bool,
    pub(crate) log_coloring: LogColoringSettings,
    pub labels: Vec<ColorLabel>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum LogColoringSection {
    #[default]
    LogLevels,
    ColorLabels,
    SelectionStyle,
    KeywordMatch,
}

pub struct ColorLabelsDialog {
    keyword_match: Entity<crate::keyword_match_style_section::KeywordMatchStyleSection>,
    selection_style: Entity<crate::selection_style_section::SelectionStyleSection>,
    saving: bool,
    error: Option<String>,
    active_section: LogColoringSection,
    highlight_log_levels: bool,
    groups: Vec<LogGroupDraft>,
    selected_group: usize,
    active_group_id: String,
    group_scroll: ScrollHandle,
    label_rows: Vec<ColorLabelDraft>,
    label_scroll: ScrollHandle,
    next_log_level_id: u64,
    next_custom_label_id: u64,
}

impl ColorLabelsDialog {
    pub fn new(
        highlight_log_levels: bool,
        log_coloring: LogColoringSettings,
        labels: Vec<ColorLabel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let selection_style = cx.new(|cx| {
            crate::selection_style_section::SelectionStyleSection::new(
                Default::default(),
                window,
                cx,
            )
        });
        let keyword_match = cx.new(|cx| {
            crate::keyword_match_style_section::KeywordMatchStyleSection::new(
                Default::default(),
                window,
                cx,
            )
        });
        let mut this = Self {
            keyword_match,
            selection_style,
            saving: false,
            error: None,
            active_section: LogColoringSection::default(),
            highlight_log_levels,
            groups: Vec::new(),
            selected_group: 0,
            active_group_id: log_coloring.active_group_id.clone(),
            group_scroll: ScrollHandle::new(),
            label_rows: Vec::with_capacity(labels.len()),
            label_scroll: ScrollHandle::new(),
            next_log_level_id: 1,
            next_custom_label_id: 1,
        };
        for group in log_coloring.groups {
            this.push_group(group, window, cx);
        }
        this.selected_group = this
            .groups
            .iter()
            .position(|group| group.id == this.active_group_id)
            .unwrap_or(0);
        for label in labels {
            this.push_label(label, window, cx);
        }
        this.refresh_rule_previews(cx);
        this
    }

    pub(crate) fn with_selection_styles(
        mut self,
        styles: crate::selection_style::SelectionStyles,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        self.selection_style = cx.new(|cx| {
            crate::selection_style_section::SelectionStyleSection::new(styles, window, cx)
        });
        self
    }

    pub(crate) fn with_keyword_match_styles(
        mut self,
        styles: crate::keyword_match_style::KeywordMatchStyles,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        self.keyword_match = cx.new(|cx| {
            crate::keyword_match_style_section::KeywordMatchStyleSection::new(styles, window, cx)
        });
        self
    }

    pub(crate) fn is_saving(&self) -> bool {
        self.saving
    }

    pub(crate) fn begin_save(&mut self, cx: &mut Context<Self>) {
        self.saving = true;
        self.error = None;
        self.selection_style
            .update(cx, |section, cx| section.set_saving(true, cx));
        self.keyword_match
            .update(cx, |section, cx| section.set_saving(true, cx));
        cx.notify();
    }

    pub(crate) fn save_failed(&mut self, error: String, cx: &mut Context<Self>) {
        self.saving = false;
        self.error = Some(error);
        self.selection_style
            .update(cx, |section, cx| section.set_saving(false, cx));
        self.keyword_match
            .update(cx, |section, cx| section.set_saving(false, cx));
        cx.notify();
    }

    pub fn config(&self, cx: &gpui::App) -> Result<LogColoringConfig, String> {
        let groups = self
            .groups
            .iter()
            .map(|group| {
                self.group_config(group, cx)
                    .map_err(|error| format!("{}: {error}", group.name.read(cx).value()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let log_coloring = LogColoringSettings {
            groups,
            active_group_id: self.active_group_id.clone(),
        };
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
            keyword_match_styles: self.keyword_match.read(cx).draft(),
            selection_styles: self.selection_style.read(cx).draft(),
            highlight_log_levels: self.highlight_log_levels,
            log_coloring,
            labels,
        })
    }

    fn group_config(
        &self,
        group: &LogGroupDraft,
        cx: &gpui::App,
    ) -> Result<LogColoringGroup, String> {
        let rules = group
            .rows
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
                if row.match_kind == LogRuleMatch::Regex {
                    regex::Regex::new(&keyword).map_err(|error| format!("{keyword}: {error}"))?;
                }
                Ok(LogLevelColorRule {
                    match_kind: row.match_kind,
                    id: row.id.clone(),
                    keyword,
                    keyword_only: row.keyword_only,
                    text_color,
                    text_alpha,
                    background_color,
                    background_alpha,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let name = group.name.read(cx).value().trim().to_string();
        if name.is_empty() {
            return Err(crate::tr!("分组名称不能为空", "Group name cannot be empty").into());
        }
        Ok(LogColoringGroup {
            id: group.id.clone(),
            name,
            preset: group.preset.clone(),
            example: group.example.clone(),
            rules,
        })
    }

    fn push_log_level_rule(
        &mut self,
        rule: LogLevelColorRule,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let resolved_config = rule.clone();
        let match_error = if rule.match_kind == LogRuleMatch::Regex {
            regex::Regex::new(&rule.keyword)
                .err()
                .map(|error| error.to_string())
        } else {
            None
        };
        let resolved = crate::color_labels::resolve_log_level_rules(std::slice::from_ref(&rule));
        let keyword = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(crate::tr!("关键词", "Keyword"))
                .default_value(rule.keyword)
        });
        let text_color = color_picker(rule.text_color, rule.text_alpha, window, cx);
        let background_color =
            color_picker(rule.background_color, rule.background_alpha, window, cx);
        let subscriptions = [
            Self::subscribe_input(&keyword, cx),
            Self::subscribe_color(&text_color, window, cx),
            Self::subscribe_color(&background_color, window, cx),
        ];
        self.groups[self.selected_group].rows.push(LogLevelDraft {
            match_kind: rule.match_kind,
            resolved_config,
            resolved,
            match_error,
            id: rule.id,
            keyword,
            keyword_only: rule.keyword_only,
            text_color,
            background_color,
            _subscriptions: subscriptions,
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
        let subscriptions = [
            Self::subscribe_input(&name, cx),
            Self::subscribe_color(&text_color, window, cx),
            Self::subscribe_color(&background_color, window, cx),
        ];
        self.label_rows.push(ColorLabelDraft {
            id: label.id,
            name,
            text_color,
            background_color,
            _subscriptions: subscriptions,
        });
    }

    fn subscribe_input(input: &Entity<InputState>, cx: &mut Context<Self>) -> Subscription {
        cx.subscribe(input, |this, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                for group in &mut this.groups {
                    if let Some(row) = group
                        .rows
                        .iter()
                        .find(|row| row.keyword.entity_id() == input.entity_id())
                    {
                        group.preview_rule_id = Some(row.id.clone());
                    }
                }
                this.refresh_rule_previews(cx);
                cx.notify();
            }
        })
    }

    fn subscribe_color(
        color: &Entity<ColorPickerState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Subscription {
        cx.subscribe_in(
            color,
            window,
            |this, picker, _: &ColorPickerEvent, window, cx| {
                crate::dialog_focus::restore_color_picker_trigger(picker, window, cx);
                this.refresh_rule_previews(cx);
                cx.notify();
            },
        )
    }

    fn add_log_level_rule(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let id = loop {
            let candidate = format!("log-level-custom-{}", self.next_log_level_id);
            self.next_log_level_id = self.next_log_level_id.saturating_add(1);
            if self.groups[self.selected_group]
                .rows
                .iter()
                .all(|row| row.id != candidate)
            {
                break candidate;
            }
        };
        let defaults = default_log_level_rules();
        let mut rule =
            defaults[self.groups[self.selected_group].rows.len() % defaults.len()].clone();
        rule.id = id;
        rule.keyword.clear();
        self.push_log_level_rule(rule, window, cx);
        if let Some(row) = self.groups[self.selected_group].rows.last() {
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
                LogColoringSection::SelectionStyle => 2,
                LogColoringSection::KeywordMatch => 3,
            })
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                Tab::new()
                    .label(crate::tr!("日志着色", "Log coloring"))
                    .disabled(self.saving),
            )
            .child(
                Tab::new()
                    .label(crate::tr!("颜色标签", "Color labels"))
                    .disabled(self.saving),
            )
            .child(
                Tab::new()
                    .label(crate::tr!("选中样式", "Selection style"))
                    .disabled(self.saving),
            )
            .child(
                Tab::new()
                    .label(crate::tr!("搜索匹配", "Search matches"))
                    .disabled(self.saving),
            )
            .on_click(cx.listener(|this, index: &usize, _, cx| {
                if this.saving {
                    return;
                }
                this.active_section = match index {
                    0 => LogColoringSection::LogLevels,
                    1 => LogColoringSection::ColorLabels,
                    2 => LogColoringSection::SelectionStyle,
                    _ => LogColoringSection::KeywordMatch,
                };
                cx.notify();
            }))
    }

    fn color_picker(&self, picker: &Entity<ColorPickerState>, cx: &gpui::App) -> gpui::AnyElement {
        if self.saving {
            gpui_base::ColorSwatch::new(
                ("saving-highlight-color", picker.entity_id()),
                picker.read(cx).value().unwrap_or(cx.theme().transparent),
            )
            .disabled(true)
            .size_6()
            .rounded(cx.theme().radius)
            .into_any_element()
        } else {
            crate::app_color_picker::new(picker)
                .small()
                .into_any_element()
        }
    }

    fn render_log_rules(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let dialog = cx.entity();
        v_flex()
            .w_full()
            .flex_1()
            .min_h_0()
            .overflow_hidden()
            .gap_3()
            .child(
                h_flex()
                    .flex_none()
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
                                        "关键词按 ASCII 单词匹配；正则可用 (?P<highlight>…) 指定着色范围。",
                                        "Keywords match ASCII words; regex may use (?P<highlight>…) to select the colored range."
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
                                    .disabled(self.saving)
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
                                    .disabled(self.saving)
                                    .icon(IconName::Plus)
                                    .label(crate::tr!("添加规则", "Add rule"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.add_log_level_rule(window, cx);
                                    })),
                            ),
                    ),
            )
            .child(div().id("log-coloring-table-horizontal").relative().flex_1().min_h_0().min_w_0()
                .overflow_x_scroll().track_scroll(&self.groups[self.selected_group].horizontal_scroll)
                .child(
                v_flex().min_w(rems(42.)).h_full()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .rounded(cx.theme().radius)
                    .border_1()
                    .border_color(cx.theme().border)
                    .child(
                        h_flex()
                            .flex_none()
                            .mr(Scrollbar::width())
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
                                    .w(rems(7.))
                                    .flex_none()
                                    .text_center()
                                    .child(crate::tr!("仅高亮关键字", "Keyword only")),
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
                            .h_full()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .overflow_y_scroll()
                            .track_scroll(&self.groups[self.selected_group].scroll)
                            .when(self.groups[self.selected_group].rows.is_empty(), |this| {
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
                            .children(self.groups[self.selected_group].rows.iter().map(|row| {
                                let row_id = format!("{}-{}", self.groups[self.selected_group].id, row.id);
                                let mode_id = row.id.clone();
                                let keyword_only_id = row.id.clone();
                                let remove_id = row.id.clone();
                                let dialog = dialog.clone();
                                let preview = self.rule_example(row, cx);
                                let style = row.resolved.matching_style(&preview);
                                let preview_highlights = row.resolved.matching_keyword_ranges(&preview)
                                    .into_iter().map(|(range, style)| (range, HighlightStyle {
                                        color: Some(style.foreground), background_color: Some(style.background),
                                        ..Default::default()
                                    })).collect::<Vec<_>>();
                                h_flex()
                                    .id(format!("log-level-color-row-{row_id}"))
                                    .flex_none()
                                    .gap_3()
                                    .px_3()
                                    .py_2()
                                    .border_t_1()
                                    .border_color(cx.theme().border)
                                    .child(div().min_w_0().flex_1().child(
                                        v_flex().gap_1().child(h_flex().gap_1()
                                            .child(Button::new(format!("rule-mode-{row_id}")).small()
                                                .disabled(self.saving).label(if row.match_kind == LogRuleMatch::Regex { "Regex" } else { crate::tr!("关键词", "Keyword") })
                                                .tooltip(crate::tr!("切换关键词 / 正则匹配", "Switch keyword / regex matching"))
                                                .on_click(cx.listener({ let id = mode_id.clone(); move |this, _, _, cx| {
                                                    if let Some(row) = this.groups[this.selected_group].rows.iter_mut().find(|row| row.id == id) {
                                                        row.match_kind = if row.match_kind == LogRuleMatch::Regex { LogRuleMatch::Keyword } else { LogRuleMatch::Regex };
                                                    }
                                                    this.groups[this.selected_group].preview_rule_id = Some(id.clone());
                                                    this.refresh_rule_previews(cx); cx.notify();
                                                }})))
                                            .child(div().flex_1().min_w_0().child(Input::new(&row.keyword).small().disabled(self.saving))))
                                            .when_some(row.match_error.clone(), |view, error| view.child(div().text_xs().text_color(cx.theme().danger).child(error))),
                                    ))
                                    .child(
                                        h_flex().w_24().flex_none().justify_center().child(
                                            h_flex()
                                                .h_7()
                                                .w_full()
                                                .justify_center()
                                                .rounded(cx.theme().radius / 2.)
                                                .border_1()
                                                .border_color(cx.theme().border)
                                                .bg(cx.theme().background)
                                                .text_color(cx.theme().foreground)
                                                .when_some(style, |preview, style| {
                                                    preview.bg(style.background).text_color(style.foreground)
                                                })
                                                .text_sm()
                                                .child(
                                                    div().min_w_0().truncate().child(
                                                        StyledText::new(preview)
                                                            .with_highlights(preview_highlights),
                                                    ),
                                                ),
                                        ),
                                    )
                                    .child(
                                        h_flex()
                                            .w(rems(7.))
                                            .flex_none()
                                            .justify_center()
                                            .child(
                                                Checkbox::new(format!(
                                                    "log-level-keyword-only-{row_id}"
                                                ))
                                                .small()
                                                .checked(row.keyword_only)
                                                .disabled(self.saving)
                                                .aria_label(crate::tr!(
                                                    "仅高亮关键字",
                                                    "Keyword only"
                                                ))
                                                .tooltip(crate::tr!(
                                                    "勾选后仅对匹配关键字着色，否则对整行着色",
                                                    "Color only matching keywords when checked; otherwise color the entire line"
                                                ))
                                                .on_click(cx.listener(
                                                    move |this, checked: &bool, _, cx| {
                                                        if let Some(row) = this.groups[this.selected_group].rows
                                                            .iter_mut()
                                                            .find(|row| row.id == keyword_only_id)
                                                        {
                                                            row.keyword_only = *checked;
                                                            this.groups[this.selected_group].preview_rule_id = Some(keyword_only_id.clone());
                                                            this.refresh_rule_previews(cx);
                                                            cx.notify();
                                                        }
                                                    },
                                                )),
                                            ),
                                    )
                                    .child(
                                        h_flex()
                                            .w_20()
                                            .flex_none()
                                            .justify_center()
                                            .child(self.color_picker(&row.text_color, cx)),
                                    )
                                    .child(
                                        h_flex()
                                            .w_20()
                                            .flex_none()
                                            .justify_center()
                                            .child(self.color_picker(&row.background_color, cx)),
                                    )
                                    .child(
                                        h_flex().w_8().flex_none().justify_end().child(
                                            Button::new(format!("remove-log-level-{remove_id}"))
                                                .small()
                                                .disabled(self.saving)
                                                .ghost()
                                                .icon(IconName::Delete)
                                                .tooltip(crate::tr!(
                                                    "删除日志级别规则",
                                                    "Delete log-level rule"
                                                ))
                                                .on_click(move |_, _, cx| {
                                                    dialog.update(cx, |this, cx| {
                                                        this.groups[this.selected_group].rows
                                                            .retain(|row| row.id != remove_id);
                                                        this.refresh_rule_previews(cx);
                                                        cx.notify();
                                                    });
                                                }),
                                        ),
                                    )
                            }))
                            .map(|list| {
                                color_rule_list(
                                    list,
                                    "log-level-rules-scroll-scrollbar",
                                    &self.groups[self.selected_group].scroll,
                                )
                            }),
                    ),
            ).horizontal_scrollbar(&self.groups[self.selected_group].horizontal_scroll))
    }

    fn render_color_labels(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let dialog = cx.entity();
        v_flex()
            .w_full()
            .flex_1()
            .min_h_0()
            .overflow_hidden()
            .gap_3()
            .child(
                h_flex()
                    .flex_none()
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
                            .small().disabled(self.saving)
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
                            .mr(Scrollbar::width())
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
                            .h_full()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .overflow_y_scroll()
                            .track_scroll(&self.label_scroll)
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
                                    .flex_none()
                                    .gap_3()
                                    .px_3()
                                    .py_2()
                                    .border_t_1()
                                    .border_color(cx.theme().border)
                                    .child(
                                        div()
                                            .min_w_0()
                                            .flex_1()
                                            .child(Input::new(&row.name).small().disabled(self.saving)),
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
                                            .child(self.color_picker(&row.text_color, cx)),
                                    )
                                    .child(
                                        h_flex()
                                            .w_20()
                                            .flex_none()
                                            .justify_center()
                                            .child(self.color_picker(&row.background_color, cx)),
                                    )
                                    .child(
                                        h_flex().w_8().flex_none().justify_end().child(
                                            Button::new(format!("remove-color-label-{remove_id}"))
                                                .small().disabled(self.saving)
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
                            }))
                            .map(|list| {
                                color_rule_list(list, "color-labels-scroll-scrollbar", &self.label_scroll)
                            }),
                    ),
            )
    }
}

fn color_rule_list(
    viewport: impl IntoElement,
    scrollbar_id: &'static str,
    scroll_handle: &ScrollHandle,
) -> impl IntoElement {
    h_flex()
        .items_stretch()
        .w_full()
        .flex_1()
        .min_h_0()
        .overflow_hidden()
        .child(viewport)
        .child(
            div()
                .relative()
                .h_full()
                .w(Scrollbar::width())
                .flex_none()
                .child(
                    Scrollbar::vertical(scroll_handle)
                        .id(scrollbar_id)
                        .mode(ScrollbarMode::Always)
                        .viewport_from_layout(),
                ),
        )
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
                LogColoringSection::SelectionStyle => {
                    self.selection_style.clone().into_any_element()
                }
                LogColoringSection::KeywordMatch => self.keyword_match.clone().into_any_element(),
            })
            .when_some(self.error.clone(), |content, error| {
                content.child(
                    div()
                        .flex_none()
                        .text_sm()
                        .text_color(cx.theme().danger)
                        .child(error),
                )
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
                LogColoringSettings::default(),
                default_color_labels(),
                window,
                cx,
            )
        });

        let picker = dialog.read_with(cx, |dialog, _| dialog.groups[0].rows[0].text_color.clone());
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
