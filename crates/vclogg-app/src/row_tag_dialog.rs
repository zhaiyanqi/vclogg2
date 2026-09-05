use gpui::{
    App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Pixels, Render, Rgba,
    Styled as _, Subscription, UniformListScrollHandle, Window, div, prelude::FluentBuilder as _,
    px, uniform_list,
};
use gpui_component::{
    ActiveTheme as _, Selectable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    color_picker::{ColorPicker, ColorPickerEvent, ColorPickerState},
    h_flex,
    input::{Input, InputEvent, InputState},
    scroll::{ScrollableElement as _, Scrollbar, ScrollbarMode},
    slider::{Slider, SliderEvent, SliderState},
    v_flex,
};

use crate::log_tags::{TagPreset, TagStyle};

/// The dialog owns the whole draft. No input entity is retained by a virtual log row.
pub(crate) struct RowTagDialog {
    label: Entity<InputState>,
    filter: Entity<InputState>,
    font_size: Entity<SliderState>,
    height: Entity<SliderState>,
    text_color: Entity<ColorPickerState>,
    background: Entity<ColorPickerState>,
    preset: TagPreset,
    presets: Vec<TagPreset>,
    visible: Vec<usize>,
    preset_scroll: UniformListScrollHandle,
    log_font: Pixels,
    row_height: Pixels,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl RowTagDialog {
    pub(crate) fn new(
        preset: TagPreset,
        presets: Vec<TagPreset>,
        log_font: Pixels,
        row_height: Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let row_height = px(f32::from(row_height).floor().max(3.));
        let label = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(preset.label.clone())
                .placeholder(crate::tr!("输入标签文字", "Enter tag text"))
                .validate(|value, _| value.chars().count() <= 128 && !value.contains(['\n', '\r']))
        });
        let filter = cx.new(|cx| {
            InputState::new(window, cx).placeholder(crate::tr!("搜索已用标签", "Search used tags"))
        });
        let (font, height_value) = preset.dimensions(log_font, row_height);
        let font_size = cx.new(|_| {
            SliderState::new()
                .min(1.)
                .max(f32::from(row_height) - 2.)
                .step(1.)
                .default_value(f32::from(font).round())
        });
        let height = cx.new(|_| {
            SliderState::new()
                .min(3.)
                .max(f32::from(row_height))
                .step(1.)
                .default_value(f32::from(height_value).round())
        });
        let (foreground, background_color) = preset.colors(cx);
        let text_color = cx.new(|cx| ColorPickerState::new(window, cx).default_value(foreground));
        let background =
            cx.new(|cx| ColorPickerState::new(window, cx).default_value(background_color));
        let subscriptions = vec![
            cx.subscribe(&label, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.error = None;
                    cx.notify();
                }
            }),
            cx.subscribe(&filter, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    let filter = this.filter.read(cx).value().to_lowercase();
                    this.visible = this
                        .presets
                        .iter()
                        .enumerate()
                        .filter(|(_, preset)| preset.label.to_lowercase().contains(&filter))
                        .map(|(ix, _)| ix)
                        .collect();
                    cx.notify();
                }
            }),
            cx.subscribe_in(
                &font_size,
                window,
                |this, _, _: &SliderEvent, window, cx| {
                    let minimum = this.font_size.read(cx).value().start() + 2.;
                    if this.height.read(cx).value().start() < minimum {
                        this.height
                            .update(cx, |height, cx| height.set_value(minimum, window, cx));
                    }
                    cx.notify();
                },
            ),
            cx.subscribe_in(&height, window, |this, _, _: &SliderEvent, window, cx| {
                let maximum = this.height.read(cx).value().start() - 2.;
                if this.font_size.read(cx).value().start() > maximum {
                    this.font_size
                        .update(cx, |font, cx| font.set_value(maximum, window, cx));
                }
                cx.notify();
            }),
            Self::observe_color(&text_color, window, cx),
            Self::observe_color(&background, window, cx),
        ];
        Self {
            label,
            filter,
            font_size,
            height,
            text_color,
            background,
            preset,
            visible: (0..presets.len()).collect(),
            preset_scroll: UniformListScrollHandle::new(),
            presets,
            log_font,
            row_height,
            error: None,
            _subscriptions: subscriptions,
        }
    }

    fn observe_color(
        picker: &Entity<ColorPickerState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Subscription {
        cx.subscribe_in(
            picker,
            window,
            |_, picker, _: &ColorPickerEvent, window, cx| {
                crate::dialog_focus::restore_color_picker_trigger(picker, window, cx);
                cx.notify();
            },
        )
    }

    pub(crate) fn input(&self) -> Entity<InputState> {
        self.label.clone()
    }

    pub(crate) fn is_composing(editor: &Entity<Self>, window: &mut Window, cx: &mut App) -> bool {
        use gpui::EntityInputHandler as _;
        let input = editor.read(cx).input();
        input.update(cx, |input, cx| {
            input.marked_text_range(window, cx).is_some()
        })
    }

    pub(crate) fn value(&self, cx: &App) -> TagPreset {
        let height = self.height.read(cx).value().start().round() as u16;
        TagPreset {
            label: self.label.read(cx).value().trim().to_string(),
            color: self.preset.color,
            style: TagStyle {
                font_size: Some(
                    (self.font_size.read(cx).value().start().round() as u16)
                        .min(height.saturating_sub(2))
                        .max(1),
                ),
                height: Some(height),
                text_color: self
                    .text_color
                    .read(cx)
                    .value()
                    .map(|color| u32::from(Rgba::from(color))),
                background_color: self
                    .background
                    .read(cx)
                    .value()
                    .map(|color| u32::from(Rgba::from(color))),
                bold: self.preset.style.bold,
                pill: self.preset.style.pill,
            },
        }
    }

    pub(crate) fn show_error(&mut self, message: String, cx: &mut Context<Self>) {
        self.error = Some(message);
        cx.notify();
    }

    fn use_preset(&mut self, preset: TagPreset, window: &mut Window, cx: &mut Context<Self>) {
        let (font, height) = preset.dimensions(self.log_font, self.row_height);
        let (text, background) = preset.colors(cx);
        self.label.update(cx, |input, cx| {
            input.set_value(preset.label.clone(), window, cx)
        });
        self.font_size.update(cx, |slider, cx| {
            slider.set_value(f32::from(font).round(), window, cx)
        });
        self.height.update(cx, |slider, cx| {
            slider.set_value(f32::from(height).round(), window, cx)
        });
        self.text_color
            .update(cx, |picker, cx| picker.set_value(text, window, cx));
        self.background
            .update(cx, |picker, cx| picker.set_value(background, window, cx));
        self.preset = preset;
        self.error = None;
        cx.notify();
    }

    fn size_field(
        &self,
        label: &'static str,
        slider: &Entity<SliderState>,
        cx: &App,
    ) -> impl IntoElement {
        v_flex()
            .flex_1()
            .min_w_0()
            .gap_2()
            .child(h_flex().justify_between().child(label).child(
                div().text_color(cx.theme().muted_foreground).child(format!(
                    "{} px",
                    slider.read(cx).value().start().round() as u16
                )),
            ))
            .child(Slider::new(slider).w_full())
    }
}

impl Render for RowTagDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut preview = self.value(cx);
        if preview.label.is_empty() {
            preview.label = crate::tr!("标签", "Tag").to_string();
        }
        let row_height_pixels = f32::from(self.row_height) as u16;
        let height = (window.viewport_size().height - window.rem_size() * 12.)
            .min(window.rem_size() * 29.)
            .max(window.rem_size() * 16.);
        h_flex()
            .h(height)
            .w_full()
            .items_stretch()
            .gap_4()
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .gap_4()
                    .child(
                        v_flex()
                            .gap_2()
                            .child(crate::tr!("文字", "Text"))
                            .child(Input::new(&self.label).w_full()),
                    )
                    .child(
                        v_flex().gap_2().child(crate::tr!("预览", "Preview")).child(
                            h_flex()
                                .h_12()
                                .px_3()
                                .overflow_hidden()
                                .bg(cx.theme().muted)
                                .rounded(cx.theme().radius)
                                .child(
                                    preview
                                        .render_tag(self.log_font, self.row_height, cx)
                                        .child(
                                            div().min_w_0().truncate().child(preview.label.clone()),
                                        ),
                                ),
                        ),
                    )
                    .child(
                        h_flex()
                            .gap_4()
                            .child(self.size_field(
                                crate::tr!("文字大小", "Text size"),
                                &self.font_size,
                                cx,
                            ))
                            .child(self.size_field(
                                crate::tr!("标签高度", "Tag height"),
                                &self.height,
                                cx,
                            )),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(crate::tr_args!(
                                "当前行高 {row_height_pixels} px；复用时自动适配行高",
                                "Row height: {row_height_pixels} px; reused tags fit their row"
                            )),
                    )
                    .child(
                        h_flex()
                            .gap_4()
                            .child(
                                v_flex()
                                    .flex_1()
                                    .gap_2()
                                    .child(crate::tr!("文字颜色", "Text color"))
                                    .child(ColorPicker::new(&self.text_color)),
                            )
                            .child(
                                v_flex()
                                    .flex_1()
                                    .gap_2()
                                    .child(crate::tr!("标签颜色", "Tag color"))
                                    .child(ColorPicker::new(&self.background)),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_4()
                            .child(
                                Checkbox::new("tag-bold")
                                    .label(crate::tr!("加粗", "Bold"))
                                    .checked(self.preset.style.bold)
                                    .on_click(cx.listener(|this, value, _, cx| {
                                        this.preset.style.bold = *value;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Checkbox::new("tag-pill")
                                    .label(crate::tr!("胶囊圆角", "Pill corners"))
                                    .checked(self.preset.style.pill)
                                    .on_click(cx.listener(|this, value, _, cx| {
                                        this.preset.style.pill = *value;
                                        cx.notify();
                                    })),
                            ),
                    )
                    .when_some(self.error.clone(), |content, error| {
                        content.child(div().text_sm().text_color(cx.theme().danger).child(error))
                    })
                    .overflow_y_scrollbar(),
            )
            .child(
                v_flex()
                    .w_56()
                    .flex_none()
                    .min_h_0()
                    .gap_2()
                    .border_l_1()
                    .border_color(cx.theme().border)
                    .pl_4()
                    .child(crate::tr!("已用标签", "Used tags"))
                    .child(Input::new(&self.filter).small())
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(crate::tr!(
                                "点击复用，再按需修改",
                                "Choose a tag, then adjust it"
                            )),
                    )
                    .when(self.visible.is_empty(), |content| {
                        content.child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(crate::tr!("没有可复用的标签", "No matching tags")),
                        )
                    })
                    .child(
                        h_flex()
                            .flex_1()
                            .min_h_0()
                            .child(
                                uniform_list(
                                    "tag-presets",
                                    self.visible.len(),
                                    cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                                        range
                                            .map(|ix| {
                                                let preset = this.presets[this.visible[ix]].clone();
                                                let selected = this.value(cx) == preset;
                                                let button = Button::new(format!(
                                                    "tag-preset-{}",
                                                    preset.id()
                                                ))
                                                .small()
                                                .ghost()
                                                .w_full()
                                                .justify_start()
                                                .selected(selected)
                                                .child(
                                                    preset
                                                        .render_tag(
                                                            this.log_font,
                                                            this.row_height.min(px(24.)),
                                                            cx,
                                                        )
                                                        .child(
                                                            div()
                                                                .min_w_0()
                                                                .truncate()
                                                                .child(preset.label.clone()),
                                                        ),
                                                )
                                                .on_click(cx.listener(
                                                    move |this, _, window, cx| {
                                                        this.use_preset(preset.clone(), window, cx)
                                                    },
                                                ));
                                                div().h_8().w_full().child(button)
                                            })
                                            .collect::<Vec<_>>()
                                    }),
                                )
                                .flex_1()
                                .min_h_0()
                                .h_full()
                                .track_scroll(&self.preset_scroll),
                            )
                            .child(
                                div().w(Scrollbar::width()).h_full().flex_none().child(
                                    Scrollbar::vertical(&self.preset_scroll)
                                        .id("tag-history-scrollbar")
                                        .mode(ScrollbarMode::Always)
                                        .viewport_from_layout(),
                                ),
                            ),
                    ),
            )
    }
}
