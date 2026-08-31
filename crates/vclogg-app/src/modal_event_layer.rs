use gpui::{
    AnyElement, HitboxBehavior, IntoElement as _, MouseDownEvent, MouseMoveEvent, ScrollWheelEvent,
    Styled as _, canvas,
};
use gpui_component::TITLE_BAR_HEIGHT;

/// Blocks raw pointer listeners owned by workspace content wherever a later
/// foreground surface occludes this layer.
///
/// This element must be rendered after workspace content and before the
/// popup/dialog layers. Its normal hitbox is a per-window, per-position
/// sentinel: without a foreground occluder it remains hovered and does nothing;
/// behind one it stops bubbling after foreground listeners have run. Mouse-up
/// is intentionally allowed through so an interaction that started before the
/// surface opened can release its state.
pub(crate) fn render_foreground_pointer_barrier() -> AnyElement {
    canvas(
        |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
        |_, hitbox, window, _| {
            let bounds = hitbox.bounds;
            let down_hitbox = hitbox.clone();
            window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                if phase.bubble()
                    && bounds.contains(&event.position)
                    && !down_hitbox.is_hovered(window)
                {
                    cx.stop_propagation();
                }
            });

            let bounds = hitbox.bounds;
            let move_hitbox = hitbox.clone();
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                // GPUI translates an external FileDrop into MouseMove without changing a
                // preceding keyboard input modality. In that case `is_hovered` is false even
                // without an occluder, so use the modality-independent scroll hit test.
                let occluded = if window.last_input_was_keyboard() {
                    !move_hitbox.should_handle_scroll(window)
                } else {
                    !move_hitbox.is_hovered(window)
                };
                if phase.bubble() && bounds.contains(&event.position) && occluded {
                    cx.stop_propagation();
                }
            });

            let bounds = hitbox.bounds;
            window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
                if phase.bubble()
                    && bounds.contains(&event.position)
                    && !hitbox.should_handle_scroll(window)
                {
                    cx.stop_propagation();
                }
            });
        },
    )
    .absolute()
    .top(TITLE_BAR_HEIGHT)
    .right_0()
    .bottom_0()
    .left_0()
    .into_any_element()
}
