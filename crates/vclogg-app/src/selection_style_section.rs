use crate::settings_dialog::layout::section;
use crate::{
    selection_style::{
        SelectionBorder, SelectionRadius, SelectionStyles, SelectionThemeStyle, SelectionWeight,
    },
    ui_theme,
};
use gpui_kit::component::{
    ActiveTheme as _, Selectable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    color_picker::{ColorPicker, ColorPickerEvent, ColorPickerState},
    h_flex,
    scroll::{Scrollbar, ScrollbarMode},
    slider::{Slider, SliderEvent, SliderState},
    switch::Switch,
    theme::ThemeMode,
    v_flex,
};
use gpui_kit::{
    AppContext as _, Context, Entity, Hsla, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, Rgba, ScrollHandle, SharedString, StatefulInteractiveElement as _,
    Styled as _, Subscription, Window, div, prelude::FluentBuilder as _,
};

#[derive(Clone, Copy)]
enum Setting {
    RowBackground,
    RowForeground,
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
            Self::RowForeground => "row-foreground",
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
    colors: [Entity<ColorPickerState>; 5],
    opacity: Entity<SliderState>,
}

pub(crate) struct SelectionStyleSection {
    draft: SelectionStyles,
    dark: bool,
    preview_inactive: bool,
    controls: [ThemeControls; 2],
    scroll: ScrollHandle,
    location: crate::settings_dialog::search::OptionLocation,
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
                        if let Some(color) = picker.read(cx).value() {
                            let value = Some(format!("#{:08x}", u32::from(Rgba::from(color))));
                            let style = this.draft.theme_mut(dark);
                            match index {
                                0 => style.row_background = value,
                                1 => style.row_border_color = value,
                                2 => style.text_background = value,
                                3 => style.text_foreground = value,
                                _ => style.row_foreground = value,
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
                    this.draft.theme_mut(dark).inactive_opacity =
                        slider.read(cx).value().start().round() as u8;
                    cx.notify();
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
            location: Default::default(),
            _subscriptions: subscriptions,
        }
    }

    pub(crate) fn draft(&self) -> SelectionStyles {
        self.draft.clone()
    }

    fn picker_colors(style: &SelectionThemeStyle, dark: bool) -> [Hsla; 5] {
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
                .and_then(|s| gpui_kit::component::theme::try_parse_color(s).ok())
                .unwrap_or(palette.row_selected_border),
            resolved.text.background,
            resolved.text.foreground.unwrap_or(palette.log_text),
            resolved.row_foreground.unwrap_or(palette.log_text),
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
            Setting::RowForeground => style.row_foreground = None,
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
            .on_click(cx.listener(move |this, _, window, cx| this.reset(setting, window, cx)))
    }

    fn field(
        &self,
        label: impl Into<SharedString>,
        control: impl IntoElement,
        setting: Setting,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let row = h_flex()
            .px_3()
            .py_2()
            .min_h_10()
            .flex_none()
            .gap_3()
            .child(div().w_32().flex_none().text_sm().child(label.into()))
            .child(div().min_w_0().flex_1().child(control))
            .child(self.reset_button(setting, cx));
        self.location.mark(setting.id(), row, cx)
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

    fn color_picker_control(&self, index: usize, _cx: &Context<Self>) -> gpui_kit::AnyElement {
        let picker = &self.controls[usize::from(self.dark)].colors[index];
        ColorPicker::new(picker).small().into_any_element()
    }

    fn choices(&self, setting: Setting, cx: &mut Context<Self>) -> gpui_kit::Div {
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

    fn foreground_control(&self, row: bool, cx: &mut Context<Self>) -> gpui_kit::Div {
        let style = self.draft.theme(self.dark);
        let (index, foreground, scope) = if row {
            (4, &style.row_foreground, "row")
        } else {
            (3, &style.text_foreground, "text")
        };
        h_flex()
            .gap_2()
            .children([false, true].map(|custom| {
                Button::new(format!("selection-{scope}-color-{custom}"))
                    .small()
                    .label(if custom {
                        crate::tr!("指定颜色", "Custom color")
                    } else {
                        crate::tr!("保留原色", "Original colors")
                    })
                    .selected(custom == foreground.is_some())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let value = this.controls[usize::from(this.dark)].colors[index]
                            .read(cx)
                            .value();
                        let style = this.draft.theme_mut(this.dark);
                        let foreground = if row {
                            &mut style.row_foreground
                        } else {
                            &mut style.text_foreground
                        };
                        *foreground = if custom {
                            value.map(|color| format!("#{:08x}", u32::from(Rgba::from(color))))
                        } else {
                            None
                        };
                        cx.notify();
                    }))
            }))
            .when(foreground.is_some(), |control| {
                control.child(self.color_picker_control(index, cx))
            })
    }

    fn render_fields(&self, cx: &mut Context<Self>) -> gpui_kit::Div {
        let style = self.draft.theme(self.dark);

        let mut rows = vec![
            self.color_field(
                0,
                crate::tr!("背景色", "Background").into(),
                Setting::RowBackground,
                cx,
            )
            .into_any_element(),
            self.field(
                crate::tr!("文字颜色", "Text color"),
                self.foreground_control(true, cx),
                Setting::RowForeground,
                cx,
            )
            .into_any_element(),
            self.field(
                crate::tr!("边框", "Border"),
                self.choices(Setting::Border, cx),
                Setting::Border,
                cx,
            )
            .into_any_element(),
        ];
        if style.row_border != SelectionBorder::None
            || self.location.is("border-color")
            || self.location.is("weight")
        {
            rows.extend([
                self.color_field(
                    1,
                    crate::tr!("边框颜色", "Border color").into(),
                    Setting::BorderColor,
                    cx,
                )
                .into_any_element(),
                self.field(
                    crate::tr!("边框粗细", "Border weight"),
                    self.choices(Setting::Weight, cx),
                    Setting::Weight,
                    cx,
                )
                .into_any_element(),
            ]);
        }
        rows.push(
            self.field(
                crate::tr!("圆角", "Corners"),
                self.choices(Setting::Radius, cx),
                Setting::Radius,
                cx,
            )
            .into_any_element(),
        );
        let mut inactive = vec![
            self.field(
                crate::tr!("淡化选区", "Dim selection"),
                Switch::new("selection-dim-inactive")
                    .small()
                    .checked(style.dim_inactive)
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.draft.theme_mut(this.dark).dim_inactive = *checked;
                        cx.notify();
                    })),
                Setting::DimInactive,
                cx,
            )
            .into_any_element(),
        ];
        if style.dim_inactive || self.location.is("inactive-opacity") {
            inactive.push(
                self.field(
                    crate::tr!("保留强度", "Remaining opacity"),
                    h_flex()
                        .gap_3()
                        .child(Slider::new(&self.controls[usize::from(self.dark)].opacity).flex_1())
                        .child(format!("{}%", style.inactive_opacity)),
                    Setting::InactiveOpacity,
                    cx,
                )
                .into_any_element(),
            );
        }
        v_flex()
            .gap_6()
            .p_1()
            .child(section(crate::tr!("整行选择", "Selected rows"), Some(crate::tr!(
                "单击或多选日志行时的外观；连续行共用一个外框。",
                "Appearance when selecting log rows. Adjacent rows share one outline.",
            ).to_owned()), rows, cx))
            .child(section(
                crate::tr!("文字选择", "Selected text"),
                Some(crate::tr!(
                    "拖选或双击选词时的外观；保留原色可继续显示搜索和标签颜色。",
                    "Appearance when dragging over text or selecting a word. Original colors preserve search and label colors.",
                ).to_owned()),
                [
                    self.color_field(2, crate::tr!("背景色", "Background").into(), Setting::TextBackground, cx)
                        .into_any_element(),
                    self.field(
                        crate::tr!("文字颜色", "Text color"),
                        self.foreground_control(false, cx),
                        Setting::TextForeground,
                        cx,
                    )
                    .into_any_element(),
                    self.field(
                        crate::tr!("下划线", "Underline"),
                        Switch::new("selection-underline")
                            .small()
                            .checked(style.text_underline)
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.draft.theme_mut(this.dark).text_underline = *checked;
                                cx.notify();
                            })),
                        Setting::Underline,
                        cx,
                    )
                    .into_any_element(),
                ],
                cx,
            ))
            .child(section(crate::tr!("失去焦点", "Unfocused selection"), Some(crate::tr!(
                "切换到其他区域或窗口时淡化选区，选中内容仍然保留。",
                "Dim selections when another region or window gains focus. The selection is retained.",
            ).to_owned()), inactive, cx))
    }

    pub(crate) fn locate(&mut self, key: Option<&'static str>, cx: &mut Context<Self>) {
        self.location.select(key, &self.scroll);
        cx.notify();
    }

    pub(crate) fn editing_dark(&self) -> bool {
        self.dark
    }
    pub(crate) fn preview_inactive(&self) -> bool {
        self.preview_inactive
    }
}

impl Render for SelectionStyleSection {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.location.reveal(window, cx);
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
                            .on_click(cx.listener(|this, _, window, cx| {
                                *this.draft.theme_mut(this.dark) = SelectionThemeStyle::default();
                                this.sync_controls(window, cx);
                                cx.notify();
                            })),
                    ),
            )
            .child(fields)
            .child(
                Switch::new("selection-preview-inactive")
                    .small()
                    .label(crate::tr!("模拟失去焦点", "Preview unfocused"))
                    .checked(self.preview_inactive)
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.preview_inactive = *checked;
                        cx.notify();
                    })),
            )
    }
}
