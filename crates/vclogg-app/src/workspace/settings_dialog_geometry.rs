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

    pub(super) fn render_settings_resize_grip(workspace: WeakEntity<Self>, cx: &App) -> AnyElement {
        let keyboard_workspace = workspace.clone();
        div()
            .id("settings-dialog-resize")
            .focusable()
            .tab_stop(true)
            .border_1()
            .border_color(cx.theme().transparent)
            .focus_visible(|style| style.border_color(cx.theme().ring))
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                let (horizontal, vertical) = match event.keystroke.key.as_str() {
                    "left" => (-1., 0.),
                    "right" => (1., 0.),
                    "up" => (0., -1.),
                    "down" => (0., 1.),
                    _ => return,
                };
                _ = keyboard_workspace.update(cx, |this, cx| {
                    let current = this.settings_dialog_geometry.size(window);
                    let next = SettingsDialogGeometry::clamp(
                        size(
                            current.width + window.rem_size() * horizontal,
                            current.height + window.rem_size() * vertical,
                        ),
                        window,
                    );
                    if next != current {
                        this.settings_dialog_geometry.preferred_size = Some(next);
                        this.settings_dialog_geometry.modified = true;
                        this.save_settings_dialog_size(window, cx);
                        cx.notify();
                    }
                });
                cx.stop_propagation();
            })
            .relative()
            .flex()
            .items_center()
            .justify_center()
            .size_5()
            .flex_shrink_0()
            .text_color(cx.theme().muted_foreground)
            .child("↘")
            .tooltip(|window, cx| {
                gpui_kit::component::tooltip::Tooltip::new(crate::tr!(
                    "拖动调整大小，或聚焦后使用方向键",
                    "Drag to resize, or focus and use arrow keys"
                ))
                .build(window, cx)
            })
            .child(
                canvas(
                    |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
                    move |_, hitbox, window, cx| {
                        let cursor = gpui_kit::CursorStyle::ResizeUpLeftDownRight;
                        window.set_cursor_style(cursor, &hitbox);
                        if workspace.upgrade().is_some_and(|this| {
                            this.read(cx).settings_dialog_geometry.gesture.is_some()
                        }) {
                            window.set_window_cursor_style(cursor);
                        }
                        window.on_mouse_event({
                            let workspace = workspace.clone();
                            move |event: &MouseDownEvent, phase, window, cx| {
                                if phase.bubble()
                                    || event.button != MouseButton::Left
                                    || !hitbox.is_hovered(window)
                                    || !hitbox.bounds.contains(&event.position)
                                {
                                    return;
                                }
                                if workspace
                                    .update(cx, |this, cx| {
                                        this.settings_dialog_geometry.gesture =
                                            Some(ResizeGesture {
                                                start: event.position,
                                                initial_size: this
                                                    .settings_dialog_geometry
                                                    .size(window),
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
                                        let Some(gesture) = this.settings_dialog_geometry.gesture
                                        else {
                                            return false;
                                        };
                                        if !event.dragging() {
                                            return this.finish_settings_dialog_resize(window, cx);
                                        }
                                        // The dialog stays centered, so each dragged edge moves
                                        // by half the change in the corresponding dimension.
                                        let delta = event.position - gesture.start;
                                        let next = SettingsDialogGeometry::clamp(
                                            size(
                                                gesture.initial_size.width + delta.x * 2.,
                                                gesture.initial_size.height + delta.y * 2.,
                                            ),
                                            window,
                                        );
                                        if next != this.settings_dialog_geometry.size(window) {
                                            this.settings_dialog_geometry.preferred_size =
                                                Some(next);
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
                .size_full(),
            )
            .into_any_element()
    }
}
