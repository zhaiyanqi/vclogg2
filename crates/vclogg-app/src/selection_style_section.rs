use crate::{
    selectable_log_text::{LogText, SelectableLogText, TextSelectionCache},
    selection_style::{
        SelectionBorder, SelectionRadius, SelectionStyles, SelectionThemeStyle, SelectionWeight,
    },
    ui_theme,
};
use gpui::{
    AppContext as _, Context, Entity, Hsla, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, Rgba, ScrollHandle, SharedString, StatefulInteractiveElement as _,
    Styled as _, StyledText, Subscription, Window, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    color_picker::{ColorPicker, ColorPickerEvent, ColorPickerState},
    h_flex,
    scroll::{Scrollbar, ScrollbarMode},
    slider::{Slider, SliderEvent, SliderState},
    switch::Switch,
    theme::ThemeMode,
    v_flex,
};

#[derive(Clone, Copy)]
enum Setting {
    RowBackground,
    BorderColor,
    Border,
    Weight,
    Radius,
    TextBackground,
    TextForeground,
    Underline,
    DimInactive,
    InactiveOpacity,
}

impl Setting {
    fn id(self) -> &'static str {
        match self {
            Self::RowBackground => "row-background",
            Self::BorderColor => "border-color",
            Self::Border => "border",
            Self::Weight => "weight",
            Self::Radius => "radius",
            Self::TextBackground => "text-background",
            Self::TextForeground => "text-foreground",
            Self::Underline => "underline",
            Self::DimInactive => "dim-inactive",
            Self::InactiveOpacity => "inactive-opacity",
        }
    }
}

struct ThemeControls {
    colors: [Entity<ColorPickerState>; 4],
    opacity: Entity<SliderState>,
}

pub(crate) struct SelectionStyleSection {
    draft: SelectionStyles,
    dark: bool,
    preview_inactive: bool,
    controls: [ThemeControls; 2],
    scroll: ScrollHandle,
    preview_selections: TextSelectionCache<usize>,
    saving: bool,
    _subscriptions: Vec<Subscription>,
}

impl SelectionStyleSection {
    pub(crate) fn new(draft: SelectionStyles, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut subscriptions = Vec::new();
        let controls = [false, true].map(|dark| {
            let style = draft.theme(dark);
            let colors = Self::picker_colors(style, dark)
                .map(|color| cx.new(|cx| ColorPickerState::new(window, cx).default_value(color)));
            for (index, picker) in colors.iter().enumerate() {
                subscriptions.push(cx.subscribe_in(
                    picker,
                    window,
                    move |this: &mut Self, picker, _: &ColorPickerEvent, window, cx| {
                        crate::dialog_focus::restore_color_picker_trigger(picker, window, cx);
                        if this.saving {
                            return;
                        }
                        if let Some(color) = picker.read(cx).value() {
                            let value = Some(format!("#{:08x}", u32::from(Rgba::from(color))));
                            let style = this.draft.theme_mut(dark);
                            match index {
                                0 => style.row_background = value,
                                1 => style.row_border_color = value,
                                2 => style.text_background = value,
                                _ => style.text_foreground = value,
                            }
                            cx.notify();
                        }
                    },
                ));
            }
            let opacity = cx.new(|_| {
                SliderState::new()
                    .min(10.)
                    .max(100.)
                    .step(5.)
                    .default_value(f32::from(style.inactive_opacity.clamp(10, 100)))
            });
            subscriptions.push(cx.subscribe(
                &opacity,
                move |this: &mut Self, slider, _: &SliderEvent, cx| {
                    if !this.saving {
                        this.draft.theme_mut(dark).inactive_opacity =
                            slider.read(cx).value().start().round() as u8;
                        cx.notify();
                    }
                },
            ));
            ThemeControls { colors, opacity }
        });
        Self {
            draft,
            dark: ui_theme::is_dark(cx),
            preview_inactive: false,
            controls,
            scroll: ScrollHandle::new(),
            preview_selections: TextSelectionCache::default(),
            saving: false,
            _subscriptions: subscriptions,
        }
    }

    pub(crate) fn draft(&self) -> SelectionStyles {
        self.draft.clone()
    }
    pub(crate) fn set_saving(&mut self, saving: bool, cx: &mut Context<Self>) {
        self.saving = saving;
        cx.notify();
    }

    fn picker_colors(style: &SelectionThemeStyle, dark: bool) -> [Hsla; 4] {
        let resolved = style.resolve(dark, true);
        let palette = ui_theme::palette_for_mode(if dark {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        });
        [
            resolved.row_background,
            style
                .row_border_color
                .as_deref()
                .and_then(|s| gpui_component::theme::try_parse_color(s).ok())
                .unwrap_or(palette.row_selected_border),
            resolved.text.background,
            resolved.text.foreground.unwrap_or(palette.log_text),
        ]
    }

    fn sync_controls(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let controls = &self.controls[usize::from(self.dark)];
        let style = self.draft.theme(self.dark);
        for (picker, color) in controls
            .colors
            .iter()
            .zip(Self::picker_colors(style, self.dark))
        {
            picker.update(cx, |picker, cx| picker.set_value(color, window, cx));
        }
        controls.opacity.update(cx, |slider, cx| {
            slider.set_value(f32::from(style.inactive_opacity), window, cx)
        });
    }

    fn reset(&mut self, setting: Setting, window: &mut Window, cx: &mut Context<Self>) {
        let style = self.draft.theme_mut(self.dark);
        let defaults = SelectionThemeStyle::default();
        match setting {
            Setting::RowBackground => style.row_background = None,
            Setting::BorderColor => style.row_border_color = None,
            Setting::Border => style.row_border = defaults.row_border,
            Setting::Weight => style.row_weight = defaults.row_weight,
            Setting::Radius => style.row_radius = defaults.row_radius,
            Setting::TextBackground => style.text_background = None,
            Setting::TextForeground => style.text_foreground = None,
            Setting::Underline => style.text_underline = false,
            Setting::DimInactive => style.dim_inactive = false,
            Setting::InactiveOpacity => style.inactive_opacity = defaults.inactive_opacity,
        }
        self.sync_controls(window, cx);
        cx.notify();
    }

    fn reset_button(&self, setting: Setting, cx: &mut Context<Self>) -> Button {
        Button::new(format!("selection-reset-{}", setting.id()))
            .small()
            .ghost()
            .label(crate::tr!("重置", "Reset"))
            .tooltip(crate::tr!(
                "恢复此项默认值",
                "Restore this value to its default"
            ))
            .disabled(self.saving)
            .on_click(cx.listener(move |this, _, window, cx| this.reset(setting, window, cx)))
    }

    fn field(
        &self,
        label: impl Into<SharedString>,
        control: impl IntoElement,
        setting: Setting,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        h_flex()
            .flex_none()
            .gap_3()
            .child(div().w_32().flex_none().text_sm().child(label.into()))
            .child(div().min_w_0().flex_1().child(control))
            .child(self.reset_button(setting, cx))
    }

    fn group(title: SharedString, description: SharedString, cx: &gpui::App) -> gpui::Div {
        v_flex()
            .gap_1()
            .flex_none()
            .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child(title))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(description),
            )
    }

    fn color_field(
        &self,
        index: usize,
        label: SharedString,
        setting: Setting,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let value = self.controls[usize::from(self.dark)].colors[index]
            .read(cx)
            .value();
        let control = h_flex()
            .gap_3()
            .child(self.color_picker_control(index, cx))
            .when_some(value, |control, color| {
                let rgb = u32::from(Rgba::from(color)) >> 8;
                let opacity = (color.a * 100.).round() as u8;
                control.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("#{rgb:06X} · {opacity}%")),
                )
            });
        self.field(label, control, setting, cx)
    }

    fn color_picker_control(&self, index: usize, cx: &Context<Self>) -> gpui::AnyElement {
        let picker = &self.controls[usize::from(self.dark)].colors[index];
        if self.saving {
            // The styled ColorPicker does not expose disabled; use its standard disabled swatch
            // while saving, so neither pointer nor keyboard can open another popup.
            gpui_base::ColorSwatch::new(
                format!("selection-saving-color-{index}"),
                picker.read(cx).value().unwrap_or(cx.theme().transparent),
            )
            .disabled(true)
            .size_6()
            .rounded(cx.theme().radius)
            .into_any_element()
        } else {
            ColorPicker::new(picker).small().into_any_element()
        }
    }

    fn choices(&self, setting: Setting, cx: &mut Context<Self>) -> gpui::Div {
        let style = self.draft.theme(self.dark);
        let (labels, selected): ([SharedString; 3], usize) = match setting {
            Setting::Border => (
                [
                    crate::tr!("无", "None").into(),
                    crate::tr!("上下线", "Top and bottom").into(),
                    crate::tr!("完整框", "Frame").into(),
                ],
                match style.row_border {
                    SelectionBorder::None => 0,
                    SelectionBorder::Horizontal => 1,
                    SelectionBorder::Frame => 2,
                },
            ),
            Setting::Weight => (
                [
                    crate::tr!("细", "Thin").into(),
                    crate::tr!("中", "Medium").into(),
                    crate::tr!("粗", "Thick").into(),
                ],
                match style.row_weight {
                    SelectionWeight::Thin => 0,
                    SelectionWeight::Medium => 1,
                    SelectionWeight::Thick => 2,
                },
            ),
            _ => (
                [
                    crate::tr!("直角", "Square").into(),
                    crate::tr!("小", "Small").into(),
                    crate::tr!("中", "Medium").into(),
                ],
                match style.row_radius {
                    SelectionRadius::Square => 0,
                    SelectionRadius::Small => 1,
                    SelectionRadius::Medium => 2,
                },
            ),
        };
        h_flex()
            .gap_2()
            .children(labels.into_iter().enumerate().map(|(index, label)| {
                Button::new(format!("selection-{}-{index}", setting.id()))
                    .small()
                    .label(label)
                    .selected(selected == index)
                    .disabled(self.saving)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let style = this.draft.theme_mut(this.dark);
                        match setting {
                            Setting::Border => {
                                style.row_border = [
                                    SelectionBorder::None,
                                    SelectionBorder::Horizontal,
                                    SelectionBorder::Frame,
                                ][index]
                            }
                            Setting::Weight => {
                                style.row_weight = [
                                    SelectionWeight::Thin,
                                    SelectionWeight::Medium,
                                    SelectionWeight::Thick,
                                ][index]
                            }
                            _ => {
                                style.row_radius = [
                                    SelectionRadius::Square,
                                    SelectionRadius::Small,
                                    SelectionRadius::Medium,
                                ][index]
                            }
                        }
                        cx.notify();
                    }))
            }))
    }

    fn render_fields(&self, cx: &mut Context<Self>) -> gpui::Div {
        let style = self.draft.theme(self.dark);
        let rows = v_flex()
            .gap_3()
            .child(Self::group(
                crate::tr!("整行选择", "Selected rows").into(),
                crate::tr!(
                    "单击或多选日志行时的外观；连续行共用一个外框。",
                    "Appearance when selecting log rows. Adjacent rows share one outline."
                )
                .into(),
                cx,
            ))
            .child(self.color_field(
                0,
                crate::tr!("背景色", "Background").into(),
                Setting::RowBackground,
                cx,
            ))
            .child(self.field(
                crate::tr!("边框", "Border"),
                self.choices(Setting::Border, cx),
                Setting::Border,
                cx,
            ))
            .when(style.row_border != SelectionBorder::None, |rows| {
                rows.child(self.color_field(
                    1,
                    crate::tr!("边框颜色", "Border color").into(),
                    Setting::BorderColor,
                    cx,
                ))
                .child(self.field(
                    crate::tr!("边框粗细", "Border weight"),
                    self.choices(Setting::Weight, cx),
                    Setting::Weight,
                    cx,
                ))
            })
            .child(self.field(
                crate::tr!("圆角", "Corners"),
                self.choices(Setting::Radius, cx),
                Setting::Radius,
                cx,
            ));
        let foreground = h_flex()
            .gap_2()
            .children([false, true].map(|custom| {
                Button::new(if custom {
                    "selection-custom-color"
                } else {
                    "selection-original-color"
                })
                .small()
                .label(if custom {
                    crate::tr!("指定颜色", "Custom color")
                } else {
                    crate::tr!("保留原色", "Original colors")
                })
                .selected(custom == style.text_foreground.is_some())
                .disabled(self.saving)
                .on_click(cx.listener(move |this, _, _, cx| {
                    let value = this.controls[usize::from(this.dark)].colors[3]
                        .read(cx)
                        .value();
                    this.draft.theme_mut(this.dark).text_foreground = if custom {
                        value.map(|color| format!("#{:08x}", u32::from(Rgba::from(color))))
                    } else {
                        None
                    };
                    cx.notify();
                }))
            }))
            .when(style.text_foreground.is_some(), |control| {
                control.child(self.color_picker_control(3, cx))
            });
        let text = v_flex().gap_3()
            .child(Self::group(crate::tr!("文字选择", "Selected text").into(), crate::tr!("拖选或双击选词时的外观；保留原色可继续显示搜索和标签颜色。", "Appearance when dragging over text or selecting a word. Original colors preserve search and label colors.").into(), cx))
            .child(self.color_field(2, crate::tr!("背景色", "Background").into(), Setting::TextBackground, cx))
            .child(self.field(crate::tr!("文字颜色", "Text color"), foreground, Setting::TextForeground, cx))
            .child(self.field(crate::tr!("下划线", "Underline"), Switch::new("selection-underline").small().checked(style.text_underline).disabled(self.saving)
                .on_click(cx.listener(|this, checked: &bool, _, cx| { this.draft.theme_mut(this.dark).text_underline = *checked; cx.notify(); })), Setting::Underline, cx));
        let inactive = v_flex().gap_3()
            .child(Self::group(crate::tr!("失去焦点", "Unfocused selection").into(), crate::tr!("切换到其他区域或窗口时淡化选区，选中内容仍然保留。", "Dim selections when another region or window gains focus. The selection is retained.").into(), cx))
            .child(self.field(crate::tr!("淡化选区", "Dim selection"), Switch::new("selection-dim-inactive").small().checked(style.dim_inactive).disabled(self.saving)
                .on_click(cx.listener(|this, checked: &bool, _, cx| { this.draft.theme_mut(this.dark).dim_inactive = *checked; cx.notify(); })), Setting::DimInactive, cx))
            .when(style.dim_inactive, |fields| fields.child(self.field(crate::tr!("保留强度", "Remaining opacity"), h_flex().gap_3()
                .child(Slider::new(&self.controls[usize::from(self.dark)].opacity).flex_1().disabled(self.saving))
                .child(format!("{}%", style.inactive_opacity)), Setting::InactiveOpacity, cx)));
        v_flex()
            .gap_5()
            .px_1()
            .py_1()
            .child(rows)
            .child(div().h(px(1.)).bg(cx.theme().border))
            .child(text)
            .child(div().h(px(1.)).bg(cx.theme().border))
            .child(inactive)
    }

    fn render_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = ui_theme::palette_for_mode(if self.dark {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        });
        let resolved = self
            .draft
            .theme(self.dark)
            .resolve(self.dark, !self.preview_inactive);
        let samples = [
            crate::tr!("普通日志：连接已建立", "Normal log: connected").to_string(),
            crate::tr!(
                "ERROR  日志着色与连续选中行",
                "ERROR  Log coloring and selected rows"
            )
            .to_string(),
            crate::tr!(
                "INFO  搜索命中与连续选中行",
                "INFO  Search match and selected rows"
            )
            .to_string(),
            crate::tr!(
                "选中文字：中文 text sample | 未选中",
                "Selected text: text sample | unselected"
            )
            .to_string(),
        ];
        let example_level = crate::color_labels::resolve_log_level_rules(
            &crate::color_labels::default_log_level_rules(),
        )
        .matching_style("ERROR");
        let rows = samples
            .into_iter()
            .enumerate()
            .map(|(index, sample)| {
                let text = LogText::new(sample.into());
                let len = text.display().len();
                let range = if index == 3 {
                    let start = text
                        .display()
                        .find("中文")
                        .or_else(|| text.display().find("text sample"))
                        .unwrap_or(0);
                    start..text.display().find(" | ").unwrap_or(len)
                } else {
                    0..0
                };
                let highlights = if index == 2 || index == 3 {
                    let range = if index == 2 {
                        0..4
                    } else {
                        let start = text.display().find("text").unwrap_or(0);
                        start..start + 4
                    };
                    vec![(
                        range,
                        gpui::HighlightStyle {
                            color: Some(colors.search_match_foreground),
                            background_color: Some(colors.search_match),
                            ..Default::default()
                        },
                    )]
                } else {
                    Vec::new()
                };
                let selection = self.preview_selections.handle(index, &text, window, cx);
                let styled =
                    StyledText::new(text.display().clone()).with_highlights(highlights.clone());
                let preview = SelectableLogText::new(
                    selection,
                    index as u64,
                    text,
                    styled,
                    resolved.text.background,
                )
                .selection_style(resolved.text, highlights)
                .preview_range(range);
                div()
                    .relative()
                    .flex_none()
                    .px_3()
                    .py_1()
                    .overflow_hidden()
                    .when(index == 1, |row| {
                        row.when_some(example_level, |row, style| {
                            row.bg(style.background).text_color(style.foreground)
                        })
                    })
                    .when(index == 1 || index == 2, |row| {
                        row.child(resolved.row_overlay(index == 1, index == 2, cx))
                    })
                    .child(preview)
            })
            .collect::<Vec<_>>();
        v_flex()
            .flex_none()
            .gap_2()
            .child(
                h_flex()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .child(crate::tr!("示例预览", "Preview")),
                    )
                    .child(
                        Switch::new("selection-preview-inactive")
                            .small()
                            .label(crate::tr!("模拟失去焦点", "Preview unfocused"))
                            .checked(self.preview_inactive)
                            .disabled(self.saving)
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.preview_inactive = *checked;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                v_flex()
                    .overflow_hidden()
                    .rounded(cx.theme().radius)
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(colors.log_background)
                    .text_color(colors.log_text)
                    .text_sm()
                    .children(rows),
            )
    }
}

impl Render for SelectionStyleSection {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let scroll = &self.scroll;
        let fields = self.render_fields(cx);
        let fields = h_flex()
            .w_full()
            .flex_1()
            .min_h_0()
            .overflow_hidden()
            .child(
                v_flex()
                    .id("selection-style-fields")
                    .h_full()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(scroll)
                    .child(fields),
            )
            .child(
                div()
                    .relative()
                    .h_full()
                    .w(Scrollbar::width())
                    .flex_none()
                    .child(
                        Scrollbar::vertical(scroll)
                            .id("selection-fields-scrollbar")
                            .mode(ScrollbarMode::Always)
                            .viewport_from_layout(),
                    ),
            );
        v_flex()
            .id("selection-style-content")
            .size_full()
            .min_h_0()
            .overflow_hidden()
            .gap_3()
            .child(
                h_flex()
                    .flex_none()
                    .gap_2()
                    .child(div().text_sm().child(crate::tr!("主题", "Theme")))
                    .children([false, true].map(|dark| {
                        Button::new(if dark {
                            "selection-dark"
                        } else {
                            "selection-light"
                        })
                        .small()
                        .label(if dark {
                            crate::tr!("深色", "Dark")
                        } else {
                            crate::tr!("浅色", "Light")
                        })
                        .selected(self.dark == dark)
                        .disabled(self.saving)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.dark = dark;
                            cx.notify();
                        }))
                    }))
                    .child(div().flex_1())
                    .child(
                        Button::new("selection-reset-theme")
                            .small()
                            .ghost()
                            .label(crate::tr!("恢复当前主题默认", "Reset this theme"))
                            .disabled(self.saving)
                            .on_click(cx.listener(|this, _, window, cx| {
                                *this.draft.theme_mut(this.dark) = SelectionThemeStyle::default();
                                this.sync_controls(window, cx);
                                cx.notify();
                            })),
                    ),
            )
            .child(fields)
            .child(self.render_preview(window, cx))
    }
}
