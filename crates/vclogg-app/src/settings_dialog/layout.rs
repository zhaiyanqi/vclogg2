//! Shared presentation for settings groups and their searchable rows.
use super::*;

pub(crate) fn external_link(
    id: &'static str,
    url: &'static str,
    label: &'static str,
    cx: &gpui_kit::App,
) -> Link {
    Link::new(id)
        .href(url)
        .open_with(|href, _, _, cx| cx.open_url(href))
        .accessibility_label(label)
        .px_1()
        .rounded(cx.theme().radius)
        .border_1()
        .border_color(cx.theme().ring.opacity(0.))
        .text_color(cx.theme().link)
        .text_decoration_1()
        .text_decoration_color(cx.theme().link.opacity(0.5))
        .hover(|link| {
            link.text_color(cx.theme().link.opacity(0.8))
                .text_decoration_color(cx.theme().link)
        })
        .active(|link| link.text_color(cx.theme().link.opacity(0.6)))
        .focus_visible(|link| link.border_color(cx.theme().ring))
        .cursor_pointer()
        .child(label)
}

pub(crate) fn group_surface(cx: &gpui_kit::App) -> gpui_kit::Div {
    v_flex()
        .bg(cx.theme().group_box)
        .text_color(cx.theme().group_box_foreground)
        .rounded(cx.theme().radius)
        .border_1()
        .border_color(cx.theme().border)
}

pub(crate) fn section(
    title: &'static str,
    note: Option<String>,
    rows: impl IntoIterator<Item = AnyElement>,
    cx: &gpui_kit::App,
) -> gpui_kit::Div {
    let mut surface = group_surface(cx);
    for (index, row) in rows.into_iter().enumerate() {
        if index > 0 {
            surface = surface.child(div().mx_3().h_px().flex_none().bg(cx.theme().border));
        }
        surface = surface.child(row);
    }
    v_flex()
        .flex_none()
        .gap_2()
        .child(
            div()
                .text_sm()
                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                .child(title),
        )
        .when_some(note, |group, note| {
            group.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(note),
            )
        })
        .child(surface)
}

pub(super) fn slider(state: &Entity<SliderState>, unit: &str, cx: &gpui_kit::App) -> gpui_kit::Div {
    let value = state.read(cx).value().start().round() as u16;
    h_flex()
        .gap_2()
        .w_48()
        .child(Slider::new(state).flex_1())
        .child(
            div()
                .w_12()
                .text_right()
                .text_sm()
                .child(format!("{value}{unit}")),
        )
}

impl SettingsDialog {
    pub(super) fn setting_row(
        &self,
        key: &'static str,
        title: &'static str,
        description: Option<&'static str>,
        control: impl IntoElement,
        cx: &gpui_kit::App,
    ) -> AnyElement {
        h_flex()
            .id(key)
            .anchor_scroll(self.location.anchor_for(key))
            .items_center()
            .gap_3()
            .px_3()
            .py_2()
            .min_h_10()
            .border_l_2()
            .border_color(cx.theme().transparent)
            .when(self.location.is(key), |row| {
                row.border_color(cx.theme().primary)
                    .bg(cx.theme().accent.opacity(0.4))
            })
            .child(
                v_flex()
                    .min_w_0()
                    .flex_1()
                    .gap_1()
                    .child(div().text_sm().child(title))
                    .when_some(description, |label, description| {
                        label.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(description),
                        )
                    }),
            )
            .child(h_flex().flex_none().justify_end().child(control))
            .into_any_element()
    }
}
