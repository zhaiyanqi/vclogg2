use super::*;

impl Workspace {
    pub(super) fn render_predefined_filters_popover(
        &self,
        has_document: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let _performance_scope =
            crate::ui_performance::scope("Workspace::render_predefined_filters_popover");
        let workspace = cx.entity();
        let filters = self.predefined_filters.clone();
        let query = self.query.read(cx).value().to_string();
        let saving = self.predefined_filters_saving;
        let preferred_size = self.filter_popover.preferred_size;

        let close_workspace = cx.weak_entity();
        Popover::new("predefined-filters-popover")
            // Popover's top_1() visually shifts the surface below the outer
            // dismissal hitbox. Use a layout margin so the bottom resize grip
            // stays inside that hitbox while preserving the trigger gap.
            .top_0()
            .mt_1()
            .on_open_change(move |open, window, cx| {
                if !open {
                    _ = close_workspace.update(cx, |this, cx| {
                        this.finish_filter_popover_resize(window, cx);
                    });
                }
            })
            .p_0()
            .text_sm()
            .trigger(
                Button::new("predefined-filters")
                    .small()
                    .h(px(f32::from(
                        self.app_settings.search_toolbar_control_height(),
                    )))
                    .outline()
                    .icon(IconName::BookOpen)
                    .map(|button| {
                        self.search_toolbar_button_label(button, crate::tr!("过滤器", "Filters"))
                    })
                    .dropdown_caret(true)
                    .loading(saving)
                    .disabled(!has_document)
                    .tooltip(crate::tr!(
                        "选择、组合或编辑预定义过滤器",
                        "Select, combine, or edit predefined filters"
                    )),
            )
            .content(move |_, window, popover_cx| {
                let filters = filters.clone();
                let menu_size = FilterPopoverState::size(preferred_size, window, filters.len());
                let resize_layer = Self::render_filter_popover_resize_layer(workspace.downgrade());
                let mut options = v_flex().w_full().gap_1().p_2().pr_4();

                if filters.is_empty() {
                    options = options.child(
                        v_flex()
                            .items_center()
                            .gap_1()
                            .px_4()
                            .py_6()
                            .text_color(popover_cx.theme().muted_foreground)
                            .child(crate::tr!("尚未配置过滤器", "No filters configured"))
                            .child(div().text_xs().child(crate::tr!(
                                "可从下方进入编辑器添加",
                                "Open the editor below to add one"
                            ))),
                    );
                } else {
                    for filter in filters {
                        let checked = query_includes_filter(&query, &filter.value);
                        let selected_filter = filter.clone();
                        let filter_value = filter.value.clone();
                        let choose_workspace = workspace.clone();
                        let choose = window.listener_for(
                            &choose_workspace,
                            move |this, checked: &bool, window, cx| {
                                this.choose_predefined_filter(
                                    selected_filter.clone(),
                                    *checked,
                                    window,
                                    cx,
                                );
                            },
                        );

                        options = options.child(
                            Checkbox::new(format!("predefined-filter-option:{}", filter.id))
                                .small()
                                .flex_shrink_0()
                                .w_full()
                                .px_3()
                                .py_2()
                                .rounded(popover_cx.theme().radius)
                                .checked(checked)
                                .label(filter.name.clone())
                                .tooltip(filter_value.clone())
                                .when(checked, |option| option.bg(popover_cx.theme().list_active))
                                .when(!checked, |option| {
                                    option.hover(|style| {
                                        style.bg(popover_cx.theme().tokens.list_hover)
                                    })
                                })
                                .on_click(choose)
                                .child(
                                    h_flex()
                                        .w_full()
                                        .min_w_0()
                                        .gap_2()
                                        .child(
                                            div()
                                                .min_w_0()
                                                .flex_1()
                                                .truncate()
                                                .text_xs()
                                                .text_color(popover_cx.theme().muted_foreground)
                                                .child(filter_value),
                                        )
                                        .when(filter.use_regex, |preview| {
                                            preview.child(
                                                div()
                                                    .flex_none()
                                                    .rounded_full()
                                                    .px_2()
                                                    .py_0p5()
                                                    .bg(popover_cx.theme().primary.opacity(0.12))
                                                    .text_xs()
                                                    .text_color(popover_cx.theme().primary)
                                                    .child(".*"),
                                            )
                                        }),
                                ),
                        );
                    }
                }

                let list = div()
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .child(options)
                    .overflow_y_scrollbar()
                    .id("predefined-filter-options-scroll");

                let edit_workspace = workspace.clone();
                let edit = popover_cx.listener(move |popover, _, window, cx| {
                    popover.dismiss(window, cx);
                    edit_workspace.update(cx, |workspace, cx| {
                        workspace.open_predefined_filters_dialog(window, cx);
                    });
                });

                v_flex()
                    .relative()
                    .w(menu_size.width)
                    .h(menu_size.height)
                    .min_w_0()
                    .child(list)
                    .child(
                        div()
                            .w_full()
                            .flex_shrink_0()
                            .border_t_1()
                            .border_color(popover_cx.theme().border)
                            .p_2()
                            .child(
                                Button::new("edit-predefined-filters")
                                    .small()
                                    .ghost()
                                    .w_full()
                                    .justify_start()
                                    .icon(IconName::Settings2)
                                    .label(crate::tr!(
                                        "编辑预定义过滤器…",
                                        "Edit predefined filters…"
                                    ))
                                    .on_click(edit),
                            ),
                    )
                    .child(resize_layer)
            })
            .into_any_element()
    }
}

#[derive(Default)]
pub(super) struct FilterPopoverState {
    preferred_size: Option<Size<Pixels>>,
    modified: bool,
    gesture: Option<FilterPopoverResizeGesture>,
    save_task: Option<Task<()>>,
}

#[derive(Clone, Copy)]
struct FilterPopoverResizeGesture {
    start: Point<Pixels>,
    initial_size: Size<Pixels>,
    width: bool,
    height: bool,
}

impl FilterPopoverState {
    fn size(
        preferred_size: Option<Size<Pixels>>,
        window: &Window,
        filter_count: usize,
    ) -> Size<Pixels> {
        // Keep the compact initial menu; an explicit user size owns both axes afterward.
        let default_height = if filter_count == 0 {
            10.
        } else if filter_count > 4 {
            19.
        } else {
            4. + filter_count as f32 * 3.5
        };
        Self::clamp_size(
            preferred_size.unwrap_or_else(|| {
                size(window.rem_size() * 20., window.rem_size() * default_height)
            }),
            window,
        )
    }

    fn clamp_size(requested: Size<Pixels>, window: &Window) -> Size<Pixels> {
        // Leave room for the popup border, offset and the host's window-edge margin.
        let margin = window.rem_size() * 2.;
        let available = window.viewport_size();
        let max_width = (available.width - margin).max(px(1.));
        let max_height = (available.height - margin).max(px(1.));
        size(
            requested
                .width
                .max((window.rem_size() * 16.).min(max_width))
                .min(max_width),
            requested
                .height
                .max((window.rem_size() * 8.).min(max_height))
                .min(max_height),
        )
    }
}

impl Workspace {
    pub(super) fn restore_filter_popover_size(
        &mut self,
        stored_size: Option<[f32; 2]>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.filter_popover.modified {
            if self.filter_popover.gesture.is_none() {
                self.save_filter_popover_size(window, cx);
            }
        } else {
            self.filter_popover.preferred_size = stored_size.map(|[w, h]| size(px(w), px(h)));
        }
    }

    fn save_filter_popover_size(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(menu_size) = self.filter_popover.preferred_size else {
            return;
        };
        let Some(store) = self.persistence.store.clone() else {
            return;
        };
        let previous_save = self.filter_popover.save_task.take();
        self.filter_popover.save_task = Some(cx.spawn_in(window, async move |this, cx| {
            if let Some(previous_save) = previous_save {
                previous_save.await;
            }
            let result = cx
                .background_spawn(async move {
                    store.save_filter_popover_size([
                        menu_size.width.as_f32(),
                        menu_size.height.as_f32(),
                    ])
                })
                .await;
            if let Err(error) = result {
                _ = this.update_in(cx, |_, window, cx| {
                    window.push_notification(
                        crate::tr_args!(
                            "过滤器菜单尺寸未能保存：{error}",
                            "Couldn’t save the filter menu size: {error}"
                        ),
                        cx,
                    );
                });
            }
        }));
    }

    fn finish_filter_popover_resize(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.filter_popover.gesture.take().is_none() {
            return false;
        }
        window.release_pointer();
        if self.filter_popover.modified {
            self.save_filter_popover_size(window, cx);
        }
        cx.notify();
        true
    }

    fn render_filter_popover_resize_layer(workspace: WeakEntity<Self>) -> AnyElement {
        canvas(
            |bounds, window, _| {
                let grip = window.rem_size() * 0.375;
                let corner_grip = window.rem_size() * 0.75;
                let right = Bounds::new(
                    point(bounds.right() - grip, bounds.top()),
                    size(grip, bounds.size.height),
                );
                let bottom = Bounds::new(
                    point(bounds.left(), bounds.bottom() - grip),
                    size(bounds.size.width, grip),
                );
                let corner = Bounds::new(
                    point(bounds.right() - corner_grip, bounds.bottom() - corner_grip),
                    size(corner_grip, corner_grip),
                );
                (
                    window.insert_hitbox(right, HitboxBehavior::Normal),
                    window.insert_hitbox(bottom, HitboxBehavior::Normal),
                    window.insert_hitbox(corner, HitboxBehavior::Normal),
                )
            },
            move |bounds, (right, bottom, corner), window, cx| {
                // GPUI resolves these requests again on pointer movement without
                // repainting. Register every grip even when it is not hovered yet.
                window.set_cursor_style(gpui::CursorStyle::ResizeLeftRight, &right);
                window.set_cursor_style(gpui::CursorStyle::ResizeUpDown, &bottom);
                window.set_cursor_style(gpui::CursorStyle::ResizeUpLeftDownRight, &corner);
                if let Some(gesture) = workspace
                    .upgrade()
                    .and_then(|this| this.read(cx).filter_popover.gesture)
                {
                    let cursor = match (gesture.width, gesture.height) {
                        (true, true) => gpui::CursorStyle::ResizeUpLeftDownRight,
                        (true, false) => gpui::CursorStyle::ResizeLeftRight,
                        _ => gpui::CursorStyle::ResizeUpDown,
                    };
                    window.set_window_cursor_style(cursor);
                }
                window.on_mouse_event({
                    let workspace = workspace.clone();
                    move |event: &MouseDownEvent, phase, window, cx| {
                        if phase.bubble() || event.button != MouseButton::Left {
                            return;
                        }
                        let in_corner =
                            corner.is_hovered(window) && corner.bounds.contains(&event.position);
                        let width = in_corner
                            || (right.is_hovered(window) && right.bounds.contains(&event.position));
                        let height = in_corner
                            || (bottom.is_hovered(window)
                                && bottom.bounds.contains(&event.position));
                        if !width && !height {
                            return;
                        }
                        if workspace
                            .update(cx, |this, cx| {
                                this.filter_popover.gesture = Some(FilterPopoverResizeGesture {
                                    start: event.position,
                                    initial_size: bounds.size,
                                    width,
                                    height,
                                });
                                cx.notify();
                            })
                            .is_ok()
                        {
                            window.capture_pointer(if in_corner {
                                corner.id
                            } else if width {
                                right.id
                            } else {
                                bottom.id
                            });
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
                                let Some(gesture) = this.filter_popover.gesture else {
                                    return false;
                                };
                                if !event.dragging() {
                                    return this.finish_filter_popover_resize(window, cx);
                                }
                                let delta = event.position - gesture.start;
                                let requested = size(
                                    gesture.initial_size.width
                                        + if gesture.width { delta.x } else { px(0.) },
                                    gesture.initial_size.height
                                        + if gesture.height { delta.y } else { px(0.) },
                                );
                                let next_size = FilterPopoverState::clamp_size(requested, window);
                                if next_size
                                    != this
                                        .filter_popover
                                        .preferred_size
                                        .unwrap_or(gesture.initial_size)
                                {
                                    this.filter_popover.modified = true;
                                    this.filter_popover.preferred_size = Some(next_size);
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
                        .update(cx, |this, cx| this.finish_filter_popover_resize(window, cx))
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
