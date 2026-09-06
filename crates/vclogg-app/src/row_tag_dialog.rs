use std::{cell::Cell, rc::Rc};

use gpui::{
    App, AppContext as _, Context, Div, DragMoveEvent, Entity, FocusHandle, Hsla,
    InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton, MouseUpEvent,
    ParentElement as _, Pixels, Point, Render, Rgba, SharedString, StatefulInteractiveElement as _,
    Styled as _, StyledText, Subscription, UniformListScrollHandle, Window, div,
    prelude::FluentBuilder as _, px, uniform_list,
};
use gpui_base::{GlobalState, TextSelection};
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

use crate::{
    log_table::{
        LogTableDelegate, line_marker, line_marker_column_width, log_cell_horizontal_padding,
        log_line_number_cell, log_row_separator_overlay,
    },
    log_tag_layer::{LogTagLayer, PositionedTag, TagDragPreview, TagGeometryHandle},
    log_tags::{TagPreset, TagStyle},
    selectable_log_text::{LogText, SelectableLogText, TextSelectionCache},
};

/// A bounded snapshot of the source row and its presentation, without a live table or file reader.
pub(crate) struct RowTagPreview {
    text: LogText,
    source_row: usize,
    position: Point<Pixels>,
    font_size: u16,
    font_family: SharedString,
    row_height: Pixels,
    text_color: Hsla,
    show_line_numbers: bool,
    line_number_width: u16,
    line_number_text_color: Hsla,
    line_number_background: Hsla,
    show_line_number_separators: bool,
    show_row_separators: bool,
}

impl RowTagPreview {
    pub(crate) fn new(
        text: SharedString,
        source_row: usize,
        position: Point<Pixels>,
        row_height: Pixels,
        delegate: &LogTableDelegate,
        cx: &App,
    ) -> Self {
        Self {
            text: LogText::new(text),
            source_row,
            position,
            font_size: delegate.log_font_size(),
            font_family: delegate.resolved_font_family(cx),
            row_height,
            text_color: delegate.log_text_color(cx),
            show_line_numbers: delegate.show_line_numbers(),
            line_number_width: delegate.line_number_width(),
            line_number_text_color: delegate.line_number_text_color(cx),
            line_number_background: delegate.line_number_background_color(cx),
            show_line_number_separators: delegate.show_line_number_row_separators(),
            show_row_separators: delegate.show_row_separators(),
        }
    }

    fn render(&self, text: SelectableLogText, tags: LogTagLayer, cx: &App) -> Div {
        let font = px(self.font_size as f32);
        h_flex()
            .w_full()
            .h(self.row_height)
            .overflow_hidden()
            .bg(cx.theme().tokens.table)
            .child(
                h_flex()
                    .w(line_marker_column_width())
                    .h_full()
                    .flex_none()
                    .justify_center()
                    .child(line_marker(true, false, cx)),
            )
            .when(self.show_line_numbers, |row| {
                row.child(
                    log_line_number_cell(
                        self.source_row,
                        self.font_size,
                        self.row_height,
                        self.line_number_text_color,
                        self.line_number_background,
                        self.show_line_number_separators,
                        cx,
                    )
                    .w(px(self.line_number_width as f32))
                    .h_full()
                    .flex_none(),
                )
            })
            .child(
                div()
                    .relative()
                    .flex()
                    .items_center()
                    .flex_1()
                    .min_w_0()
                    .h(self.row_height)
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .px(log_cell_horizontal_padding(cx))
                    .text_size(font)
                    .font_family(self.font_family.clone())
                    .line_height(self.row_height)
                    .text_color(self.text_color)
                    .when(self.show_row_separators, |cell| {
                        cell.child(log_row_separator_overlay(false, cx))
                    })
                    .child(text)
                    .child(tags),
            )
    }
}

struct DraggedPreviewTag;

struct PreviewTagDrag {
    grab_offset: Point<Pixels>,
    previous_position: Option<Point<Pixels>>,
}

/// The dialog owns the whole draft. No input entity is retained by a virtual log row.
pub(crate) struct RowTagDialog {
    label: Entity<InputState>,
    filter: Entity<InputState>,
    font_size: Entity<SliderState>,
    height: Entity<SliderState>,
    transparency: Entity<SliderState>,
    text_color: Entity<ColorPickerState>,
    background: Entity<ColorPickerState>,
    preset: TagPreset,
    presets: Vec<TagPreset>,
    visible: Vec<usize>,
    preset_scroll: UniformListScrollHandle,
    log_font: Pixels,
    row_height: Pixels,
    preview: RowTagPreview,
    preview_selections: TextSelectionCache<usize>,
    preview_geometry: TagGeometryHandle,
    // Display-only position, never included in the submitted preset.
    preview_position: Option<Point<Pixels>>,
    preview_drag: Option<PreviewTagDrag>,
    preview_focus: FocusHandle,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl RowTagDialog {
    pub(crate) fn new(
        preset: TagPreset,
        presets: Vec<TagPreset>,
        preview: RowTagPreview,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let log_font = px(preview.font_size as f32);
        let row_height = px(f32::from(preview.row_height).floor().max(3.));
        let label = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(preset.label.clone())
                .placeholder(crate::tr!("输入标记文字", "Enter mark text"))
                .validate(|value, _| value.chars().count() <= 128 && !value.contains(['\n', '\r']))
        });
        let filter = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(crate::tr!("搜索历史标记", "Search mark history"))
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
        let transparency = cx.new(|_| {
            SliderState::new()
                .min(0.)
                .max(100.)
                .step(1.)
                .default_value(f32::from(preset.style.transparency.min(100)))
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
            cx.subscribe(&transparency, |_, _, _: &SliderEvent, cx| cx.notify()),
            Self::observe_color(&text_color, window, cx),
            Self::observe_color(&background, window, cx),
            cx.observe_window_activation(window, |this, window, cx| {
                if !window.is_window_active() {
                    this.cancel_preview_drag(window, cx);
                }
            }),
        ];
        Self {
            label,
            filter,
            font_size,
            height,
            transparency,
            text_color,
            background,
            preset,
            visible: (0..presets.len()).collect(),
            preset_scroll: UniformListScrollHandle::new(),
            presets,
            log_font,
            row_height,
            preview,
            preview_selections: TextSelectionCache::default(),
            preview_geometry: Rc::new(Cell::new(None)),
            preview_position: None,
            preview_drag: None,
            preview_focus: cx.focus_handle(),
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
                transparency: self.transparency.read(cx).value().start().round() as u8,
                bold: self.preset.style.bold,
                pill: self.preset.style.pill,
                vertical_center: self.preset.style.vertical_center,
            },
        }
    }

    fn update_preview_drag(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(drag) = &self.preview_drag else {
            return;
        };
        let Some(geometry) = self.preview_geometry.get() else {
            return;
        };
        let mut position = geometry.drag_position(position, drag.grab_offset);
        if self.preset.style.vertical_center {
            // Keep the free-position draft for when centering is turned off again.
            position.y = self.preview_position.unwrap_or(self.preview.position).y;
        }
        if self.preview_position != Some(position) {
            self.preview_position = Some(position);
            cx.notify();
        }
    }

    fn finish_preview_drag(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.preview_drag.is_none() {
            return;
        }
        self.update_preview_drag(position, cx);
        self.preview_drag = None;
        cx.stop_active_drag(window);
        cx.stop_propagation();
        cx.notify();
    }

    pub(crate) fn cancel_preview_drag(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(drag) = self.preview_drag.take() {
            self.preview_position = drag.previous_position;
            cx.stop_active_drag(window);
            cx.notify();
        }
    }

    fn render_preview(
        &mut self,
        preset: &TagPreset,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let text = &self.preview.text;
        let selection = self
            .preview_selections
            .handle(self.preview.source_row, text, window, cx);
        let text = SelectableLogText::new(
            selection,
            self.preview.source_row as u64,
            text.clone(),
            StyledText::new(text.display().clone()),
            crate::ui_theme::text_selection_highlight(cx),
        )
        .preview_range(0..0);
        let owner = cx.weak_entity();
        let tag = div()
            .id("tag-dialog-preview-tag")
            .max_w(self.log_font * 24.)
            .overflow_hidden()
            // Keep opacity outside Tag so its built-in hover style cannot override it.
            .opacity(preset.style.opacity())
            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                GlobalState::suppress_text_selection(cx);
                TextSelection::clear(window, cx);
                cx.stop_propagation();
            })
            .on_drag(DraggedPreviewTag, move |_, offset, window, cx| {
                _ = owner.update(cx, |this, cx| {
                    if this.preview_geometry.get().is_some() {
                        this.preview_drag = Some(PreviewTagDrag {
                            grab_offset: offset,
                            previous_position: this.preview_position,
                        });
                        this.preview_focus.focus(window, cx);
                        cx.notify();
                    }
                });
                cx.new(|_| TagDragPreview)
            })
            .child(
                preset
                    .render_tag(self.log_font, self.preview.row_height, cx)
                    .child(div().min_w_0().truncate().child(preset.label.clone())),
            );
        let tags = LogTagLayer::new(
            "tag-dialog-preview-layer",
            vec![PositionedTag {
                position: self.preview_position.unwrap_or(self.preview.position),
                vertical_center: preset.style.vertical_center,
                geometry: self.preview_geometry.clone(),
                element: Some(tag.into_any_element()),
            }],
        );
        self.preview
            .render(text, tags, cx)
            .id("row-tag-preview")
            .track_focus(&self.preview_focus)
            .on_drag_move::<DraggedPreviewTag>(cx.listener(
                |this, event: &DragMoveEvent<DraggedPreviewTag>, _, cx| {
                    if this.preview_drag.is_some() {
                        this.update_preview_drag(event.event.position, cx);
                        cx.stop_propagation();
                    }
                },
            ))
            .capture_any_mouse_up(cx.listener(|this, event: &MouseUpEvent, window, cx| {
                if event.button == MouseButton::Left {
                    this.finish_preview_drag(event.position, window, cx);
                }
            }))
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, window, cx| {
                    this.finish_preview_drag(event.position, window, cx);
                }),
            )
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" && this.preview_drag.is_some() {
                    this.cancel_preview_drag(window, cx);
                    cx.stop_propagation();
                }
            }))
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
        self.transparency.update(cx, |slider, cx| {
            slider.set_value(f32::from(preset.style.transparency.min(100)), window, cx)
        });
        self.text_color
            .update(cx, |picker, cx| picker.set_value(text, window, cx));
        self.background
            .update(cx, |picker, cx| picker.set_value(background, window, cx));
        self.preset = preset;
        self.error = None;
        cx.notify();
    }

    fn slider_field(
        &self,
        label: &'static str,
        slider: &Entity<SliderState>,
        suffix: &'static str,
        cx: &App,
    ) -> impl IntoElement {
        v_flex()
            .flex_1()
            .min_w_0()
            .gap_2()
            .child(h_flex().justify_between().child(label).child(
                div().text_color(cx.theme().muted_foreground).child(format!(
                    "{}{suffix}",
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
            preview.label = crate::tr!("标记", "Mark").to_string();
        }
        let row_height_pixels = f32::from(self.row_height) as u16;
        let form = v_flex()
            .flex_1()
            .min_w_0()
            .min_h_0()
            // Input's focus ring extends outside its border. Keep it inside
            // the scroll clip without replacing the component's own chrome.
            .p_2()
            .gap_4()
            .child(
                v_flex()
                    .gap_2()
                    .child(crate::tr!("文字", "Text"))
                    .child(Input::new(&self.label).w_full()),
            )
            .child(
                h_flex()
                    .gap_4()
                    .child(self.slider_field(
                        crate::tr!("文字大小", "Text size"),
                        &self.font_size,
                        " px",
                        cx,
                    ))
                    .child(self.slider_field(
                        crate::tr!("标记高度", "Mark height"),
                        &self.height,
                        " px",
                        cx,
                    )),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(crate::tr_args!(
                        "当前行高 {row_height_pixels} px；复用时自动适配行高",
                        "Row height: {row_height_pixels} px; reused marks fit their row"
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
                            .child(crate::tr!("标记颜色", "Mark color"))
                            .child(ColorPicker::new(&self.background)),
                    ),
            )
            .child(
                v_flex()
                    .gap_2()
                    .child(h_flex().child(self.slider_field(
                        crate::tr!("透明度", "Transparency"),
                        &self.transparency,
                        "%",
                        cx,
                    )))
                    .child(
                        h_flex()
                            .justify_between()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(crate::tr!("不透明", "Opaque"))
                            .child(crate::tr!("完全透明", "Transparent")),
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
            .child(
                Checkbox::new("tag-vertical-center")
                    .label(crate::tr!("垂直居中", "Center vertically"))
                    .checked(self.preset.style.vertical_center)
                    .on_click(cx.listener(|this, value, _, cx| {
                        this.preset.style.vertical_center = *value;
                        cx.notify();
                    })),
            )
            .when_some(self.error.clone(), |content, error| {
                content.child(div().text_sm().text_color(cx.theme().danger).child(error))
            })
            .overflow_y_scrollbar();
        h_flex()
            .h_full()
            .min_h_0()
            .w_full()
            .items_stretch()
            .gap_4()
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .gap_4()
                    .child(form)
                    .child(
                        v_flex()
                            .flex_none()
                            .px_2()
                            .pb_2()
                            .gap_2()
                            .child(crate::tr!("日志行预览", "Log row preview"))
                            .child(
                                div()
                                    .py_3()
                                    .bg(cx.theme().tokens.table)
                                    .border_1()
                                    .border_color(cx.theme().border)
                                    .rounded(cx.theme().radius)
                                    .child(self.render_preview(&preview, window, cx)),
                            ),
                    ),
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
                    .pr_2()
                    .py_2()
                    .child(crate::tr!("历史标记", "Mark history"))
                    .child(Input::new(&self.filter).small())
                    .when(self.visible.is_empty(), |content| {
                        content.child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(crate::tr!("没有可复用的标记", "No matching marks")),
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
                                                    div()
                                                        .min_w_0()
                                                        .truncate()
                                                        .child(preset.label.clone()),
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
