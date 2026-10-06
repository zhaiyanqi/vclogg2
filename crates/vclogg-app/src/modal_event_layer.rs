use gpui_kit::component::TITLE_BAR_HEIGHT;
use gpui_kit::{
    AnyElement, HitboxBehavior, IntoElement as _, MouseDownEvent, MouseMoveEvent, ScrollWheelEvent,
    Styled as _, canvas,
};

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

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc, time::Duration};

    use gpui_kit::component::{WindowExt as _, button::Button};
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{
        AppContext as _, Context, FocusHandle, InteractiveElement as _, IntoElement,
        ParentElement as _, Render, TestAppContext, Window, WindowOptions, div,
    };

    use super::*;

    // Exercise the production barrier against Root-owned overlays, including
    // raw listeners like those installed by the selectable log elements.
    struct PointerContent {
        presses: Rc<Cell<usize>>,
        focus: FocusHandle,
    }

    impl Render for PointerContent {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let presses = self.presses.clone();
            div()
                .size_full()
                .child(
                    canvas(
                        |_, _, _| (),
                        move |_, _, window, _| {
                            window.on_mouse_event(move |_: &MouseDownEvent, phase, _, _| {
                                if phase.bubble() {
                                    presses.set(presses.get() + 1);
                                }
                            });
                        },
                    )
                    .absolute()
                    .size_full(),
                )
                .child(
                    Button::new("background-control")
                        .label("Background")
                        .track_focus(&self.focus),
                )
                .child(render_foreground_pointer_barrier())
        }
    }

    #[gpui_kit::test]
    fn root_hosts_overlays_without_leaking_pointer_events(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let presses = Rc::new(Cell::new(0));
        let (handle, content) = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
                cx.new(|cx| PointerContent {
                    presses: presses.clone(),
                    focus: cx.focus_handle(),
                })
            })
            .unwrap()
        });
        let focus = content.read_with(cx, |content, _| content.focus.clone());
        let overlay_presses = Rc::new(Cell::new(0));
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("background-control", cx);
            assert_eq!(presses.get(), 1, "the barrier must allow unoccluded input");
            focus.focus(window, cx);
            let dialog_presses = overlay_presses.clone();
            window.open_dialog(cx, move |dialog, _, _| {
                let overlay_presses = dialog_presses.clone();
                dialog.title("Dialog").child(
                    Button::new("overlay-control")
                        .label("Overlay")
                        .on_click(move |_, _, _| {
                            overlay_presses.set(overlay_presses.get() + 1);
                        }),
                )
            });
            window.render_frame(cx);
            assert!(window.find("overlay-control").visible());
            window.click("overlay-control", cx);
            assert_eq!(overlay_presses.get(), 1);
            assert_eq!(
                presses.get(),
                1,
                "dialog input must not reach raw content listeners"
            );
            window.press("escape", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.background_executor.advance_clock(Duration::from_secs(1));
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(!window.has_active_dialog(cx));
            assert!(
                focus.is_focused(window),
                "Escape must restore the trigger's focus"
            );
            window.open_sheet(cx, |sheet, _, _| {
                sheet
                    .title("Sheet")
                    .child(Button::new("sheet-control").label("Sheet control"))
            });
            window.render_frame(cx);
            assert!(window.find("sheet-control").visible());
            window.click("sheet-control", cx);
            assert_eq!(
                presses.get(),
                1,
                "sheet input must not reach raw content listeners"
            );
            window.close_sheet(cx);
        })
        .unwrap();
    }
}
