use gpui::{
    AppContext as _, Context, Entity, Hsla, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, Rgba, ScrollHandle, StatefulInteractiveElement as _, Styled as _,
    StyledText, Subscription, Window, div,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    color_picker::{ColorPicker, ColorPickerEvent, ColorPickerState},
    h_flex,
    scroll::{Scrollbar, ScrollbarMode},
    theme::ThemeMode,
    v_flex,
};

use crate::{keyword_match_style::KeywordMatchStyles, ui_theme};

pub(crate) struct KeywordMatchStyleSection {
    draft: KeywordMatchStyles,
    dark: bool,
    // Theme -> search/quick find -> foreground/background. All pickers survive tab switches.
    controls: [[[Entity<ColorPickerState>; 2]; 2]; 2],
    scroll: ScrollHandle,
    saving: bool,
    _subscriptions: Vec<Subscription>,
}

impl KeywordMatchStyleSection {
    pub(crate) fn new(
        draft: KeywordMatchStyles,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut subscriptions = Vec::new();
        let controls = [false, true].map(|dark| {
            [false, true].map(|quick_find| {
                Self::colors(&draft, dark, quick_find).map(|color| {
                    cx.new(|cx| ColorPickerState::new(window, cx).default_value(color))
                })
            })
        });
        for (theme_ix, groups) in controls.iter().enumerate() {
            for (kind_ix, colors) in groups.iter().enumerate() {
                for (color_ix, picker) in colors.iter().enumerate() {
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
                                let style = this.draft.style_mut(theme_ix == 1, kind_ix == 1);
                                if color_ix == 0 {
                                    style.foreground = value;
                                } else {
                                    style.background = value;
                                }
                                cx.notify();
                            }
                        },
                    ));
                }
            }
        }
        Self {
            draft,
            dark: ui_theme::is_dark(cx),
            controls,
            scroll: ScrollHandle::new(),
            saving: false,
            _subscriptions: subscriptions,
        }
    }

    pub(crate) fn draft(&self) -> KeywordMatchStyles {
        self.draft.clone()
    }

    pub(crate) fn set_saving(&mut self, saving: bool, cx: &mut Context<Self>) {
        self.saving = saving;
        cx.notify();
    }

    fn colors(draft: &KeywordMatchStyles, dark: bool, quick_find: bool) -> [Hsla; 2] {
        let style = draft.style(dark, quick_find).resolve(dark, quick_find);
        [style.color.unwrap(), style.background_color.unwrap()]
    }

    fn sync_controls(&self, window: &mut Window, cx: &mut Context<Self>) {
        for quick_find in [false, true] {
            for (picker, color) in self.controls[usize::from(self.dark)][usize::from(quick_find)]
                .iter()
                .zip(Self::colors(&self.draft, self.dark, quick_find))
            {
                picker.update(cx, |picker, cx| picker.set_value(color, window, cx));
            }
        }
    }

    fn render_color(
        &self,
        quick_find: bool,
        background: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let picker = &self.controls[usize::from(self.dark)][usize::from(quick_find)]
            [usize::from(background)];
        let color = picker.read(cx).value().unwrap_or(cx.theme().transparent);
        let control = if self.saving {
            gpui_base::ColorSwatch::new(("keyword-saving-color", picker.entity_id()), color)
                .disabled(true)
                .size_6()
                .rounded(cx.theme().radius)
                .into_any_element()
        } else {
            ColorPicker::new(picker).small().into_any_element()
        };
        let value = format!(
            "#{:06X} · {}%",
            u32::from(Rgba::from(color)) >> 8,
            (color.a * 100.).round() as u8
        );
        h_flex()
            .gap_3()
            .w_full()
            .child(div().flex_1().text_sm().child(if background {
                crate::tr!("背景颜色", "Background color")
            } else {
                crate::tr!("文字颜色", "Text color")
            }))
            .child(control)
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(value),
            )
            .child(
                Button::new(format!("keyword-reset-color-{quick_find}-{background}"))
                    .small()
                    .ghost()
                    .label(crate::tr!("恢复默认", "Reset"))
                    .disabled(self.saving)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        let style = this.draft.style_mut(this.dark, quick_find);
                        if background {
                            style.background = None;
                        } else {
                            style.foreground = None;
                        }
                        this.sync_controls(window, cx);
                        cx.notify();
                    })),
            )
    }

    fn render_group(&self, quick_find: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let style = self.draft.style(self.dark, quick_find);
        v_flex()
            .flex_none()
            .gap_3()
            .child(
                div()
                    .text_sm()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(if quick_find {
                        crate::tr!("页内查找", "Quick find")
                    } else {
                        crate::tr!("搜索匹配", "Search matches")
                    }),
            )
            .child(self.render_color(quick_find, false, cx))
            .child(self.render_color(quick_find, true, cx))
            .child(
                h_flex().gap_4().children(
                    [
                        ("bold", crate::tr!("加粗", "Bold"), style.bold),
                        ("italic", crate::tr!("斜体", "Italic"), style.italic),
                        (
                            "underline",
                            crate::tr!("下划线", "Underline"),
                            style.underline,
                        ),
                    ]
                    .map(|(id, label, checked)| {
                        Checkbox::new(format!("keyword-{quick_find}-{id}"))
                            .label(label)
                            .checked(checked)
                            .disabled(self.saving)
                            .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                                let style = this.draft.style_mut(this.dark, quick_find);
                                match id {
                                    "bold" => style.bold = *checked,
                                    "italic" => style.italic = *checked,
                                    _ => style.underline = *checked,
                                }
                                cx.notify();
                            }))
                    }),
                ),
            )
    }

    fn render_preview(&self, cx: &Context<Self>) -> impl IntoElement {
        let palette = ui_theme::palette_for_mode(if self.dark {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        });
        v_flex()
            .flex_none()
            .gap_2()
            .child(div().text_sm().child(crate::tr!("预览", "Preview")))
            .child(
                v_flex()
                    .gap_2()
                    .p_3()
                    .rounded(cx.theme().radius)
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(palette.log_background)
                    .text_color(palette.log_text)
                    .text_sm()
                    .children([false, true].map(|quick_find| {
                        let prefix = if quick_find {
                            crate::tr!("页内查找：", "Quick find: ")
                        } else {
                            crate::tr!("搜索匹配：", "Search matches: ")
                        };
                        let keyword = crate::tr!("命中关键词", "matched keyword");
                        let text = format!("{prefix}{keyword} — log message");
                        StyledText::new(text).with_highlights([(
                            prefix.len()..prefix.len() + keyword.len(),
                            self.draft
                                .style(self.dark, quick_find)
                                .resolve(self.dark, quick_find),
                        )])
                    })),
            )
    }
}

impl Render for KeywordMatchStyleSection {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .id("keyword-match-content")
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
                            "keyword-dark"
                        } else {
                            "keyword-light"
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
                        Button::new("keyword-reset-theme")
                            .small()
                            .ghost()
                            .label(crate::tr!("恢复当前主题默认", "Reset this theme"))
                            .disabled(self.saving)
                            .on_click(cx.listener(|this, _, window, cx| {
                                *this.draft.theme_mut(this.dark) = Default::default();
                                this.sync_controls(window, cx);
                                cx.notify();
                            })),
                    ),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .items_start()
                    .child(
                        v_flex()
                            .id("keyword-fields-scroll")
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .overflow_y_scroll()
                            .track_scroll(&self.scroll)
                            .child(
                                v_flex()
                                    .w_full()
                                    .flex_none()
                                    .p_1()
                                    .gap_6()
                                    .child(self.render_group(false, cx))
                                    .child(self.render_group(true, cx)),
                            ),
                    )
                    .child(
                        div().flex_none().h_full().child(
                            Scrollbar::vertical(&self.scroll)
                                .id("keyword-fields-scrollbar")
                                .mode(ScrollbarMode::Always)
                                .viewport_from_layout(),
                        ),
                    ),
            )
            .child(self.render_preview(cx))
    }
}
