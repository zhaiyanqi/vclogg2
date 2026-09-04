use gpui::{App, Entity, Focusable as _, Window};
use gpui_component::color_picker::ColorPickerState;

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
