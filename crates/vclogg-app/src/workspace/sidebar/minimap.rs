use super::*;

impl SidebarState {
    fn minimap_position(&self, bounds: Bounds<Pixels>, minimum_height: Pixels) -> (f32, f32) {
        let count = self
            .document
            .as_ref()
            .map_or(0, |(_, document)| document.source_line_count())
            .max(1) as f32;
        let Some(viewport) = &self.viewport else {
            return (0., 1.);
        };
        let (first, extent) = overview_viewport(viewport, count);
        let height = extent.max((minimum_height / bounds.size.height.max(px(1.))).min(1.));
        (first.min(1. - height), height)
    }

    fn move_minimap(
        &mut self,
        position: Point<Pixels>,
        start: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(bounds) = self.minimap_bounds.get() else {
            return;
        };
        let (first, height) = self.minimap_position(bounds, window.rem_size());
        let ratio = ((position.y - bounds.top()) / bounds.size.height.max(px(1.))).clamp(0., 1.);
        if start {
            self.minimap_drag = Some(if ratio >= first && ratio <= first + height {
                ratio - first
            } else {
                height / 2.
            });
        }
        let Some(offset) = self.minimap_drag else {
            return;
        };
        let count = self
            .document
            .as_ref()
            .map_or(0, |(_, document)| document.source_line_count());
        if count == 0 {
            return;
        }
        let target = (ratio - offset).clamp(0., 1.) * count as f32;
        let row = (target.floor() as usize).min(count - 1);
        self.jump_fraction(row, target.fract(), false, window, cx);
        cx.notify();
    }

    pub(super) fn render_minimap(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let summary = self.summary.clone().expect("loaded overview");
        if summary.line_count() == 0 {
            return self.empty(
                SidebarPanelId::Minimap,
                crate::tr!("文件为空", "The file is empty"),
                None,
                cx,
            );
        }
        let viewport = self.viewport.clone();
        let bounds_cell = self.minimap_bounds.clone();
        let foreground = cx.theme().muted_foreground;
        let active = cx.theme().primary;
        let minimum_height = window.rem_size();
        let marks = self
            .overview_marks
            .iter()
            .map(|group| {
                group
                    .and_then(|ix| self.colors.get(ix))
                    .map(|group| group.paint_color())
            })
            .collect::<Vec<_>>();
        let overview = canvas(
            move |bounds, _, _| {
                bounds_cell.set(Some(bounds));
            },
            move |bounds, _, window, _| {
                let total = summary.line_count().max(1) as f32;
                let strip = bounds.size.height * (summary.bucket_rows() as f32 / total);
                for (ix, bucket) in summary.buckets().iter().enumerate() {
                    let y = bounds.top() + strip * ix as f32;
                    let height = strip.min(bounds.bottom() - y).max(px(0.));
                    if height <= px(0.) {
                        continue;
                    }
                    let color = marks
                        .get(ix)
                        .copied()
                        .flatten()
                        .unwrap_or(foreground.opacity(0.15 + bucket.density() * 0.5));
                    let width = bounds.size.width * bucket.width_fraction().max(0.02);
                    window.paint_quad(gpui::fill(
                        Bounds::new(point(bounds.left(), y), size(width, height * 0.7)),
                        color,
                    ));
                }
                if let Some(viewport) = &viewport {
                    let (first, extent) = overview_viewport(viewport, total);
                    let height = (bounds.size.height * extent)
                        .max(minimum_height)
                        .min(bounds.size.height);
                    let top =
                        (bounds.size.height * first).min((bounds.size.height - height).max(px(0.)));
                    let rect = Bounds::new(
                        point(bounds.left(), bounds.top() + top),
                        size(bounds.size.width, height),
                    );
                    window.paint_quad(gpui::fill(rect, active.opacity(0.18)));
                    window.paint_quad(gpui::fill(
                        Bounds::new(rect.origin, size(rect.size.width, px(1.))),
                        active,
                    ));
                    window.paint_quad(gpui::fill(
                        Bounds::new(
                            point(rect.left(), rect.bottom() - px(1.)),
                            size(rect.size.width, px(1.)),
                        ),
                        active,
                    ));
                }
            },
        )
        .size_full();
        div()
            .id("sidebar-minimap")
            .flex_1()
            .min_h_0()
            .min_w_0()
            .relative()
            .track_focus(&self.focus[&SidebarPanelId::Minimap])
            .tab_index(0)
            .aria_label(crate::tr!(
                "文件缩略图，拖动或使用方向键定位",
                "File minimap; drag or use arrow keys to navigate"
            ))
            .border_1()
            .border_color(cx.theme().transparent)
            .focus_visible(|style| style.border_color(cx.theme().ring))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    this.focus[&SidebarPanelId::Minimap].focus(window, cx);
                    this.move_minimap(event.position, true, window, cx);
                    cx.stop_propagation();
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                if event.pressed_button == Some(MouseButton::Left) {
                    this.move_minimap(event.position, false, window, cx);
                } else {
                    this.minimap_drag = None;
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.minimap_drag = None),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.minimap_drag = None),
            )
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                let count = this
                    .document
                    .as_ref()
                    .map_or(0, |(_, document)| document.source_line_count());
                let current = this
                    .viewport
                    .as_ref()
                    .map_or(0, |viewport| viewport.position().row_ix);
                let page = this
                    .viewport
                    .as_ref()
                    .map_or(1, |viewport| viewport.visible_range().len().max(1));
                if count == 0 {
                    return;
                }
                let row = match event.keystroke.key.as_str() {
                    "up" => current.saturating_sub(1),
                    "down" => current.saturating_add(1),
                    "pageup" => current.saturating_sub(page),
                    "pagedown" => current.saturating_add(page),
                    "home" => 0,
                    "end" => count - 1,
                    "escape" => {
                        this.minimap_drag = None;
                        return;
                    }
                    _ => return,
                };
                this.jump(row.min(count - 1), false, window, cx);
                cx.stop_propagation();
            }))
            .child(overview)
            .into_any_element()
    }
}

/// Map only the measured screen to source-row fractions, including partially visible wrapped rows.
fn overview_viewport(viewport: &VirtualLogViewport<LogRowKey>, total: f32) -> (f32, f32) {
    let position = viewport.position();
    let first = position.row_ix as f32
        + position.offset_in_row / viewport.row_height(position.row_ix).max(px(1.));
    let mut remaining = viewport.viewport_bounds().size.height;
    let mut end = first;
    for row in position.row_ix..viewport.visible_range().end {
        let height = viewport.row_height(row).max(px(1.));
        let offset = if row == position.row_ix {
            position.offset_in_row
        } else {
            px(0.)
        };
        let used = remaining.min((height - offset).max(px(0.)));
        end = row as f32 + ((offset + used) / height).min(1.);
        remaining -= used;
        if remaining <= px(0.) {
            break;
        }
    }
    (
        (first / total).clamp(0., 1.),
        ((end - first).max(1.) / total).min(1.),
    )
}
