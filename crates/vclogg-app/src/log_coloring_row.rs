//! Shared group-row presentation for the highlight editor and sidebar.
use gpui::{
    ClickEvent, ElementId, InteractiveElement as _, MouseButton, ParentElement as _, SharedString,
    Styled as _, div, prelude::FluentBuilder as _, relative,
};
use gpui_component::{
    ActiveTheme as _, Icon, IconName, Selectable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
};

pub(crate) fn ignore_secondary_mouse_down(
    event: &gpui::MouseDownEvent,
    window: &mut gpui::Window,
    cx: &mut gpui::App,
) {
    if event.button != MouseButton::Left {
        // GPUI's default mouse focus transfer happens before a click is dispatched.
        window.prevent_default();
        cx.stop_propagation();
    }
}

pub(crate) fn is_activation(event: &ClickEvent) -> bool {
    match event {
        ClickEvent::Mouse(event) => {
            event.down.button == MouseButton::Left && event.up.button == MouseButton::Left
        }
        _ => true,
    }
}
pub(crate) fn group_button(
    id: impl Into<ElementId>,
    name: impl Into<SharedString>,
    rule_count: usize,
    selected: bool,
    active: bool,
    cx: &gpui::App,
) -> Button {
    let name = name.into();
    let state = if active {
        crate::tr!("当前激活", "Active")
    } else {
        crate::tr!("未激活", "Inactive")
    };
    let accessible_name = crate::tr_args!(
        "{name}，{rule_count} 条规则，{state}",
        "{name}, {rule_count} rules, {state}"
    );
    crate::button_accessibility::with_label(
        Button::new(id)
            .capture_any_mouse_down(ignore_secondary_mouse_down)
            .small()
            .ghost()
            .w_full()
            .h_8()
            .flex_none()
            .selected(selected)
            // Supply content directly: Button's built-in label forces a clipped 1em line box.
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .gap_2()
                    .text_sm()
                    .line_height(relative(1.25))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(cx.theme().foreground)
                            .child(name),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_color(cx.theme().muted_foreground)
                            .child(rule_count.to_string()),
                    )
                    .child(h_flex().w_5().h_5().flex_none().justify_center().when(
                        active,
                        |slot| {
                            slot.child(
                                Icon::new(IconName::Check)
                                    .small()
                                    .text_color(cx.theme().primary),
                            )
                        },
                    )),
            ),
        accessible_name,
    )
}
