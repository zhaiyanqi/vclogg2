use gpui::{
    App, Entity, Focusable as _, InteractiveElement as _, IntoElement, ParentElement as _, Window,
    div,
};
use gpui_component::{
    button::Button,
    color_picker::ColorPickerState,
    dialog::{Cancel, Confirm},
};

/// Wraps a dialog button in a stable focus node and dispatches Confirm from that node.
///
/// gpui-component buttons deliberately prevent pointer focus on mouse down. Dispatching a
/// dialog action through `Window::dispatch_action` therefore depends on whichever control was
/// focused before the click. That control may already have disappeared with a nested popup,
/// causing the action to miss the dialog. Dispatching from this rendered wrapper gives the
/// action a valid path through the dialog regardless of the window's current focus.
pub(crate) fn dialog_confirm_action(
    id: &'static str,
    button: Button,
    cx: &mut App,
) -> impl IntoElement {
    dialog_confirm_action_when(id, button, true, cx)
}

pub(crate) fn dialog_confirm_action_when(
    id: &'static str,
    button: Button,
    enabled: bool,
    cx: &mut App,
) -> impl IntoElement {
    let action_focus = cx.focus_handle();
    let dispatch_focus = action_focus.clone();
    let button = button.on_click(move |_, window, cx| {
        if enabled {
            dispatch_focus.dispatch_action(&Confirm { secondary: false }, window, cx)
        }
    });
    div().id(id).track_focus(&action_focus).child(button)
}

/// Wraps a dialog button in a stable focus node and dispatches Cancel from that node.
pub(crate) fn dialog_cancel_action(
    id: &'static str,
    button: Button,
    cx: &mut App,
) -> impl IntoElement {
    let action_focus = cx.focus_handle();
    let dispatch_focus = action_focus.clone();
    let button =
        button.on_click(move |_, window, cx| dispatch_focus.dispatch_action(&Cancel, window, cx));
    div().id(id).track_focus(&action_focus).child(button)
}

/// Restores the parent surface's focus path after a committed color closes its popup.
///
/// `ColorPickerState::select_color` closes the controlled popup directly. Unlike the
/// picker's ordinary dismiss path, that transition does not return focus from a focused
/// popup child (notably the hex input) to the trigger. Dialog footer actions are routed
/// through the focused path, so leaving focus on the removed child makes Save/Confirm
/// appear inert until the user focuses the dialog again.
pub(crate) fn restore_color_picker_trigger(
    picker: &Entity<ColorPickerState>,
    window: &mut Window,
    cx: &mut App,
) {
    if !picker.read(cx).is_open() {
        picker.focus_handle(cx).focus(window, cx);
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc};

    use gpui::{
        Context, FocusHandle, Modifiers, Render, Styled as _, TestAppContext, point,
        prelude::FluentBuilder as _, px,
    };

    use super::*;

    struct DialogActionHarness {
        confirmed: Rc<Cell<bool>>,
        dialog_focus: FocusHandle,
        stale_focus: FocusHandle,
        show_stale_control: bool,
    }

    impl Render for DialogActionHarness {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let confirmed = self.confirmed.clone();
            let popup = div()
                .size(px(120.))
                .when(self.show_stale_control, |popup| {
                    popup.child(
                        div()
                            .id("stale-dialog-control")
                            .size(px(1.))
                            .track_focus(&self.stale_focus),
                    )
                })
                .child(dialog_confirm_action(
                    "dialog-action-test-confirm-action",
                    Button::new("dialog-action-test-confirm").label("Confirm"),
                    cx,
                ));

            gpui_base::Dialog::new(cx)
                .focus_handle(self.dialog_focus.clone())
                .popup(popup)
                .on_ok(move |_, _, _| {
                    confirmed.set(true);
                    false
                })
        }
    }

    #[gpui::test]
    fn pointer_confirm_uses_its_rendered_path_when_current_focus_is_stale(cx: &mut TestAppContext) {
        cx.update(gpui_component::init);
        let confirmed = Rc::new(Cell::new(false));
        let (harness, cx) = cx.add_window_view({
            let confirmed = confirmed.clone();
            move |_, cx| DialogActionHarness {
                confirmed,
                dialog_focus: cx.focus_handle(),
                stale_focus: cx.focus_handle(),
                show_stale_control: true,
            }
        });

        let stale_focus = harness.read_with(cx, |harness, _| harness.stale_focus.clone());
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
            stale_focus.focus(window, cx);
            harness.update(cx, |harness, cx| {
                harness.show_stale_control = false;
                cx.notify();
            });
            window.draw(cx).clear(cx);
            assert!(stale_focus.is_focused(window));
        });

        cx.simulate_click(point(px(20.), px(16.)), Modifiers::default());
        assert!(
            confirmed.get(),
            "pointer confirm must reach the dialog even when the old focus node was removed"
        );
    }

    #[gpui::test]
    fn stable_dialog_action_preserves_keyboard_activation(cx: &mut TestAppContext) {
        cx.update(gpui_component::init);
        let confirmed = Rc::new(Cell::new(false));
        let (_, cx) = cx.add_window_view({
            let confirmed = confirmed.clone();
            move |_, cx| DialogActionHarness {
                confirmed,
                dialog_focus: cx.focus_handle(),
                stale_focus: cx.focus_handle(),
                show_stale_control: false,
            }
        });

        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
            window.focus_next(cx);
        });
        cx.simulate_keystrokes("enter");

        assert!(
            confirmed.get(),
            "Enter must still activate the dialog button"
        );
    }
}
