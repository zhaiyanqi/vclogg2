use super::*;

#[derive(Default)]
pub(super) struct SettingsDialogGeometry {
    preferred_size: Option<Size<Pixels>>,
    modified: bool,
    gesture: Option<ResizeGesture>,
    save_task: Option<Task<()>>,
}

#[derive(Clone, Copy)]
struct ResizeGesture {
    start: Point<Pixels>,
    initial_size: Size<Pixels>,
    direction: ResizeDirection,
}

#[derive(Clone, Copy)]
struct ResizeDirection {
    horizontal: f32,
    vertical: f32,
}

impl ResizeDirection {
    fn cursor(self) -> gpui_kit::CursorStyle {
        match (self.horizontal, self.vertical) {
            (0., _) => gpui_kit::CursorStyle::ResizeUpDown,
            (_, 0.) => gpui_kit::CursorStyle::ResizeLeftRight,
            (x, y) if x == y => gpui_kit::CursorStyle::ResizeUpLeftDownRight,
            _ => gpui_kit::CursorStyle::ResizeUpRightDownLeft,
        }
    }
}

impl SettingsDialogGeometry {
    pub(super) fn size(&self, window: &Window) -> Size<Pixels> {
        Self::clamp(
            self.preferred_size
                .unwrap_or_else(|| size(window.rem_size() * 80., window.rem_size() * 50.)),
            window,
        )
    }

    fn clamp(requested: Size<Pixels>, window: &Window) -> Size<Pixels> {
        let viewport = window.viewport_size();
        let max_width = (viewport.width - window.rem_size() * 2.).max(px(0.));
        let max_height = (viewport.height - window.rem_size() * 4.).max(px(0.));
        size(
            requested.width.max(window.rem_size() * 48.).min(max_width),
            requested
                .height
                .max(window.rem_size() * 28.)
                .min(max_height),
        )
    }
}

impl Workspace {
    pub(super) fn restore_settings_dialog_size(
        &mut self,
        stored: Option<[f32; 2]>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.settings_dialog_geometry.modified {
            if self.settings_dialog_geometry.gesture.is_none() {
                self.save_settings_dialog_size(window, cx);
            }
        } else {
            self.settings_dialog_geometry.preferred_size =
                stored.map(|[width, height]| size(px(width), px(height)));
        }
    }

    fn save_settings_dialog_size(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(preferred) = self.settings_dialog_geometry.preferred_size else {
            return;
        };
        let Some(store) = self.persistence.store.clone() else {
            return;
        };
        let previous = self.settings_dialog_geometry.save_task.take();
        self.settings_dialog_geometry.save_task =
            Some(cx.spawn_in(window, async move |this, cx| {
                if let Some(previous) = previous {
                    previous.await;
                }
                let result = cx
                    .background_spawn(async move {
                        store.save_settings_dialog_size([
                            preferred.width.as_f32(),
                            preferred.height.as_f32(),
                        ])
                    })
                    .await;
                if let Err(error) = result {
                    _ = this.update_in(cx, |_, window, cx| {
                        window.notify_message(
                            crate::tr_args!(
                                "设置窗口尺寸未能保存：{error}",
                                "Couldn’t save the settings window size: {error}"
                            ),
                            cx,
                        );
                    });
                }
            }));
    }

    pub(super) fn finish_settings_dialog_resize(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.settings_dialog_geometry.gesture.take().is_none() {
            return false;
        }
        window.release_pointer();
        if self.settings_dialog_geometry.modified {
            self.save_settings_dialog_size(window, cx);
        }
        cx.notify();
        true
    }

    pub(super) fn render_settings_resize_layer(workspace: WeakEntity<Self>) -> AnyElement {
        canvas(
            |bounds, window, _| {
                let edge = window.rem_size() * 0.375;
                let corner = window.rem_size() * 0.75;
                let width = bounds.size.width;
                let height = bounds.size.height;
                // Edges exclude the corners so each pointer position has one owner.
                [
                    (
                        0.,
                        -1.,
                        corner,
                        px(0.),
                        (width - corner * 2.).max(px(0.)),
                        edge,
                    ),
                    (
                        0.,
                        1.,
                        corner,
                        height - edge,
                        (width - corner * 2.).max(px(0.)),
                        edge,
                    ),
                    (
                        -1.,
                        0.,
                        px(0.),
                        corner,
                        edge,
                        (height - corner * 2.).max(px(0.)),
                    ),
                    (
                        1.,
                        0.,
                        width - edge,
                        corner,
                        edge,
                        (height - corner * 2.).max(px(0.)),
                    ),
                    (-1., -1., px(0.), px(0.), corner, corner),
                    (1., -1., width - corner, px(0.), corner, corner),
                    (-1., 1., px(0.), height - corner, corner, corner),
                    (1., 1., width - corner, height - corner, corner, corner),
                ]
                .map(|(horizontal, vertical, x, y, w, h)| {
                    let hitbox = window.insert_hitbox(
                        Bounds::new(bounds.origin + point(x, y), size(w, h)),
                        HitboxBehavior::Normal,
                    );
                    (
                        ResizeDirection {
                            horizontal,
                            vertical,
                        },
                        hitbox,
                    )
                })
            },
            move |_, grips, window, cx| {
                for (direction, hitbox) in &grips {
                    window.set_cursor_style(direction.cursor(), hitbox);
                }
                if let Some(gesture) = workspace
                    .upgrade()
                    .and_then(|this| this.read(cx).settings_dialog_geometry.gesture)
                {
                    window.set_window_cursor_style(gesture.direction.cursor());
                }
                window.on_mouse_event({
                    let workspace = workspace.clone();
                    move |event: &MouseDownEvent, phase, window, cx| {
                        if phase.bubble() || event.button != MouseButton::Left {
                            return;
                        }
                        let Some((direction, hitbox)) = grips.iter().find(|(_, hitbox)| {
                            hitbox.is_hovered(window) && hitbox.bounds.contains(&event.position)
                        }) else {
                            return;
                        };
                        if workspace
                            .update(cx, |this, cx| {
                                this.settings_dialog_geometry.gesture = Some(ResizeGesture {
                                    start: event.position,
                                    initial_size: this.settings_dialog_geometry.size(window),
                                    direction: *direction,
                                });
                                cx.notify();
                            })
                            .is_ok()
                        {
                            window.capture_pointer(hitbox.id);
                            window.prevent_default();
                            cx.stop_propagation();
                        }
                    }
                });
                window.on_mouse_event({
                    let workspace = workspace.clone();
                    move |event: &MouseMoveEvent, phase, window, cx| {
                        if phase.bubble() {
                            return;
                        }
                        let consumed = workspace
                            .update(cx, |this, cx| {
                                let Some(gesture) = this.settings_dialog_geometry.gesture else {
                                    return false;
                                };
                                if !event.dragging() {
                                    return this.finish_settings_dialog_resize(window, cx);
                                }
                                // Centered dialogs move both opposite edges equally.
                                let delta = event.position - gesture.start;
                                let next = SettingsDialogGeometry::clamp(
                                    size(
                                        gesture.initial_size.width
                                            + delta.x * (2. * gesture.direction.horizontal),
                                        gesture.initial_size.height
                                            + delta.y * (2. * gesture.direction.vertical),
                                    ),
                                    window,
                                );
                                if next != this.settings_dialog_geometry.size(window) {
                                    this.settings_dialog_geometry.preferred_size = Some(next);
                                    this.settings_dialog_geometry.modified = true;
                                    cx.notify();
                                }
                                true
                            })
                            .unwrap_or(false);
                        if consumed {
                            cx.stop_propagation();
                        }
                    }
                });
                window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                    if phase.bubble() || event.button != MouseButton::Left {
                        return;
                    }
                    if workspace
                        .update(cx, |this, cx| {
                            this.finish_settings_dialog_resize(window, cx)
                        })
                        .unwrap_or(false)
                    {
                        cx.stop_propagation();
                    }
                });
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .into_any_element()
    }
}
