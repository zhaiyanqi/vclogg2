use super::minimap::{overview_thumb, overview_viewport};
use super::overview::{COLOR_TRACKS_PER_PAGE, OVERVIEW_BINS};
use super::*;

impl SidebarState {
    pub(super) fn render_overview(
        &mut self,
        panel: SidebarPanelId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let total = self.overview_source_count();
        if total == 0 {
            return self.empty(panel, crate::tr!("文件为空", "The file is empty"), None, cx);
        }
        if panel == SidebarPanelId::Colors && self.colors.is_empty() {
            if let Some(error) = self.color_error.clone() {
                return self.empty(panel, error, Some(panel), cx);
            }
            let message = if !self.color_jobs.is_empty() {
                crate::tr!("正在加载颜色标签…", "Loading color labels…")
            } else {
                crate::tr!(
                    "当前文件未应用颜色标签",
                    "No color labels applied to this file"
                )
            };
            return self.empty(panel, message, None, cx);
        }
        let range = self.overview_range(panel);
        let displayed_range = self
            .overview_state(panel)
            .snapshot
            .as_ref()
            .map_or(&range, |snapshot| &snapshot.range);
        let first = displayed_range.start + 1;
        let last = displayed_range.end;
        let range_label = crate::tr_args!("行 {first}–{last}", "Lines {first}–{last}");
        let toolbar = (panel == SidebarPanelId::Minimap).then(|| {
            h_flex()
                .flex_wrap()
                .flex_shrink_0()
                .gap_1()
                .p_1()
                .child(
                    Button::new(SharedString::from(format!("overview-zoom-in-{panel:?}")))
                        .small()
                        .ghost()
                        .label(crate::tr!("放大", "Zoom in"))
                        .tooltip(crate::tr!(
                            "放大当前位置或最近悬停区段",
                            "Zoom in at the current or last hovered position"
                        ))
                        .disabled(range.len() <= OVERVIEW_BINS)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.zoom_overview(panel, true, window, cx)
                        })),
                )
                .child(
                    Button::new(SharedString::from(format!("overview-zoom-out-{panel:?}")))
                        .small()
                        .ghost()
                        .label(crate::tr!("缩小", "Zoom out"))
                        .tooltip(crate::tr!("缩小概览范围", "Zoom out"))
                        .disabled(self.overview_state(panel).zoom.is_none())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.zoom_overview(panel, false, window, cx)
                        })),
                )
                .child(
                    Button::new(SharedString::from(format!("overview-reset-{panel:?}")))
                        .small()
                        .ghost()
                        .label(crate::tr!("全文", "Full file"))
                        .tooltip(crate::tr!(
                            "返回全文概览（Esc）",
                            "Return to full file overview (Esc)"
                        ))
                        .disabled(self.overview_state(panel).zoom.is_none())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.reset_overview_zoom(panel, window, cx)
                        })),
                )
        });
        let mut body = v_flex().id(SharedString::from(format!("sidebar-minimap-{panel:?}"))).flex_1().min_h_0().min_w_0()
            .track_focus(&self.focus[&panel]).tab_index(0)
            .aria_label(if panel == SidebarPanelId::Colors {
                crate::tr!(
                    "颜色轨概览，悬停查看命中行数，点击跳转，按住左键拖动定位，Ctrl+滚轮围绕鼠标缩放，方向键定位，Esc 返回全文",
                    "Color tracks; hover for matching line counts, click to navigate, hold left button and drag to move, Ctrl+wheel to zoom at pointer, arrow keys to navigate, Escape for full file"
                )
            } else {
                crate::tr!(
                    "分轨概览，悬停查看命中行数，点击跳转，双击放大，Ctrl+滚轮围绕鼠标缩放，方向键定位，Esc 返回全文",
                    "Track overview; hover for matching line counts, click to navigate, double-click to zoom, Ctrl+wheel to zoom at pointer, arrow keys to navigate, Escape for full file"
                )
            })
            .border_1().border_color(cx.theme().transparent)
            .focus_visible(|style| style.border_color(cx.theme().ring))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| this.overview_key(panel, event, window, cx)))
            .when_some(toolbar, |this, toolbar| this.child(toolbar))
            .child(div().px_2().pb_1().flex_shrink_0().text_xs().child(range_label));

        if panel == SidebarPanelId::Colors && self.colors.len() > COLOR_TRACKS_PER_PAGE {
            let page = self.overview_state(panel).color_page + 1;
            let pages = self.colors.len().div_ceil(COLOR_TRACKS_PER_PAGE);
            body = body.child(
                h_flex()
                    .flex_wrap()
                    .p_1()
                    .gap_1()
                    .flex_shrink_0()
                    .child(
                        Button::new(SharedString::from(format!(
                            "overview-previous-labels-{panel:?}"
                        )))
                        .small()
                        .ghost()
                        .label(crate::tr!("上组", "Previous"))
                        .tooltip(crate::tr!("上一组颜色轨", "Previous color tracks"))
                        .disabled(page == 1)
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.page_overview_colors(panel, false, window, cx)
                            },
                        )),
                    )
                    .child(div().flex_1().min_w_0().text_xs().child(crate::tr_args!(
                        "标签 {page}/{pages}",
                        "Labels {page}/{pages}"
                    )))
                    .child(
                        Button::new(SharedString::from(format!(
                            "overview-next-labels-{panel:?}"
                        )))
                        .small()
                        .ghost()
                        .label(crate::tr!("下组", "Next"))
                        .tooltip(crate::tr!("下一组颜色轨", "Next color tracks"))
                        .disabled(page == pages)
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.page_overview_colors(panel, true, window, cx)
                            },
                        )),
                    ),
            );
        }
        if let Some(snapshot) = self.overview_state(panel).snapshot.clone() {
            let labels = std::iter::once((
                "position".to_owned(),
                crate::tr!("位置", "Position").to_owned(),
            ))
            .filter(|_| snapshot.position_track)
            .chain(snapshot.tracks.iter().map(|track| {
                (
                    track
                        .group
                        .as_ref()
                        .map_or_else(|| "search".to_owned(), |group| group.id.clone()),
                    track.label(),
                )
            }));
            body = body
                .child(
                    h_flex()
                        .items_stretch()
                        .flex_shrink_0()
                        .children(labels.map(|(id, label)| {
                            div()
                                .id(SharedString::from(format!(
                                    "overview-heading-{panel:?}-{id}"
                                )))
                                .flex_1()
                                .min_w_0()
                                .px_1()
                                .py_1()
                                .text_xs()
                                .truncate()
                                .tooltip({
                                    let label = label.clone();
                                    move |window, cx| {
                                        gpui_kit::component::tooltip::Tooltip::new(label.clone())
                                            .build(window, cx)
                                    }
                                })
                                .child(label)
                        })),
                )
                .child(self.render_overview_canvas(panel, window, cx));
            let details = if let Some(hover) = &self.overview_state(panel).hover {
                let rows = snapshot.rows(hover.bins.clone());
                let first = rows.start + 1;
                let last = rows.end;
                let (label, count) = if snapshot.position_track && hover.lane == 0 {
                    let count = rows.len();
                    (
                        crate::tr!("位置", "Position").to_owned(),
                        crate::tr_args!("{count} 行", "{count} lines"),
                    )
                } else {
                    let track = &snapshot.tracks[hover.lane - usize::from(snapshot.position_track)];
                    let count: usize = track.counts[hover.bins.clone()].iter().sum();
                    (
                        track.label(),
                        crate::tr_args!("命中 {count} 行", "{count} matching lines"),
                    )
                };
                v_flex()
                    .child(div().truncate().child(label))
                    .child(
                        div()
                            .truncate()
                            .child(crate::tr_args!("起始行：{first}", "First line: {first}")),
                    )
                    .child(
                        div()
                            .truncate()
                            .child(crate::tr_args!("结束行：{last}", "Last line: {last}")),
                    )
                    .child(div().truncate().child(count))
            } else {
                let row = self.overview_current_row() + 1;
                v_flex()
                    .child(div().truncate().child(crate::tr_args!(
                        "当前位置：第 {row} 行",
                        "Current position: line {row}"
                    )))
                    .child(div().truncate().child(crate::tr!(
                        "悬停查看区段与命中行数",
                        "Hover for range and matching lines"
                    )))
                    .child(
                        div()
                            .truncate()
                            .child(crate::tr!("点击跳转", "Click to navigate")),
                    )
                    .when(panel == SidebarPanelId::Minimap, |this| {
                        this.child(
                            div()
                                .truncate()
                                .child(crate::tr!("双击放大", "Double-click to zoom")),
                        )
                    })
            };
            body = body.child(
                details
                    .child(div().truncate().child(crate::tr!(
                        "Ctrl+滚轮：围绕鼠标缩放",
                        "Ctrl+wheel: zoom at pointer"
                    )))
                    .child(div().truncate().child(if panel == SidebarPanelId::Colors {
                        crate::tr!(
                            "左键拖动定位 · Esc 返回全文",
                            "Drag to move · Esc: full file"
                        )
                    } else {
                        crate::tr!("Esc：返回全文", "Esc: full file")
                    }))
                    .flex_shrink_0()
                    .h(rems(8.))
                    .min_w_0()
                    .p_2()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .border_t_1()
                    .border_color(cx.theme().border),
            );
        } else {
            body = body.child(
                div()
                    .flex_1()
                    .min_h_0()
                    .p_2()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(crate::tr!("正在汇总分布…", "Preparing overview…")),
            );
        }
        if panel == SidebarPanelId::Colors {
            let error = self.color_error.clone();
            let message = if let Some(error) = &error {
                crate::tr_args!("颜色加载失败：{error}", "Could not load color: {error}")
            } else if !self.color_jobs.is_empty() {
                crate::tr!("正在更新颜色轨…", "Updating color tracks…").to_owned()
            } else {
                let count = self.colors.len();
                crate::tr_args!("{count} 种颜色", "{count} colors")
            };
            // Reserve status space so an incremental scan does not resize all existing tracks.
            body = body.child(
                h_flex()
                    .h(rems(2.))
                    .px_1()
                    .gap_1()
                    .flex_shrink_0()
                    .min_w_0()
                    .child(
                        div()
                            .id("color-track-status")
                            .flex_1()
                            .min_w_0()
                            .text_xs()
                            .truncate()
                            .text_color(cx.theme().muted_foreground)
                            .tooltip({
                                let message = message.clone();
                                move |window, cx| {
                                    gpui_kit::component::tooltip::Tooltip::new(message.clone())
                                        .build(window, cx)
                                }
                            })
                            .child(message),
                    )
                    .when(error.is_some(), |this| {
                        this.child(
                            Button::new("overview-retry-colors")
                                .small()
                                .label(crate::tr!("重试", "Retry"))
                                .on_click(cx.listener(|this, _, _, cx| this.retry_colors(cx))),
                        )
                    }),
            );
        }
        body.into_any_element()
    }

    fn render_overview_canvas(
        &self,
        panel: SidebarPanelId,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let snapshot = self
            .overview_state(panel)
            .snapshot
            .clone()
            .expect("installed overview");
        let bounds_cell = self.overview_state(panel).bounds.clone();
        let viewport = self.viewport.clone();
        let document = self.document.as_ref().map(|(_, document)| document.clone());
        let hover = self.overview_state(panel).hover.clone();
        let active = cx.theme().primary;
        let muted = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let inset = window.rem_size() * 0.25;
        let overview = canvas(
            move |bounds, _, _| bounds_cell.set(Some(bounds)),
            move |bounds, _, window, _| {
                if bounds.size.width <= px(0.) || bounds.size.height <= px(0.) {
                    return;
                }
                let lanes = snapshot.lane_count();
                let position_lanes = usize::from(snapshot.position_track);
                let width = bounds.size.width / lanes as f32;
                // Raster separators and aggregation use actual display pixels; UI spacing uses rem.
                let pixel = px(1. / window.scale_factor());
                let lane_width = (width - inset * 2.).max(pixel);
                for lane in 0..lanes {
                    let left = bounds.left() + width * lane as f32;
                    window.paint_quad(gpui_kit::fill(
                        Bounds::new(
                            point(left + inset, bounds.top()),
                            size(lane_width, bounds.size.height),
                        ),
                        muted.opacity(0.08),
                    ));
                    if lane > 0 {
                        window.paint_quad(gpui_kit::fill(
                            Bounds::new(point(left, bounds.top()), size(pixel, bounds.size.height)),
                            border,
                        ));
                    }
                }
                let stride = snapshot.stride(bounds, window.scale_factor());
                for (ix, track) in snapshot.tracks.iter().enumerate() {
                    let color = track
                        .group
                        .as_ref()
                        .map_or(active, |group| group.paint_color());
                    let left = bounds.left() + width * (ix + position_lanes) as f32 + inset;
                    for first in (0..snapshot.bins).step_by(stride) {
                        let last = (first + stride).min(snapshot.bins);
                        let count: usize = track.counts[first..last].iter().sum();
                        if count == 0 {
                            continue;
                        }
                        let rows = snapshot.rows(first..last).len().max(1);
                        let density = count as f32 / rows as f32;
                        let top = bounds.size.height * (first as f32 / snapshot.bins as f32);
                        let bottom = bounds.size.height * (last as f32 / snapshot.bins as f32);
                        window.paint_quad(gpui_kit::fill(
                            Bounds::new(
                                point(left, bounds.top() + top),
                                size(
                                    lane_width,
                                    (bottom - top).max(pixel).min(bounds.size.height - top),
                                ),
                            ),
                            color.opacity(0.4 + 0.6 * density.sqrt()),
                        ));
                    }
                }
                if let Some(viewport) = &viewport {
                    let rows = overview_viewport(viewport, document.as_deref());
                    if let Some((top, height)) =
                        overview_thumb(rows, &snapshot.range, bounds, pixel)
                    {
                        if snapshot.position_track {
                            window.paint_quad(gpui_kit::fill(
                                Bounds::new(
                                    point(bounds.left() + inset, bounds.top() + top),
                                    size(lane_width, height),
                                ),
                                active,
                            ));
                        }
                        let left = bounds.left() + width * position_lanes as f32;
                        let mark_width = bounds.size.width - width * position_lanes as f32;
                        for edge in [top, top + height - pixel] {
                            window.paint_quad(gpui_kit::fill(
                                Bounds::new(
                                    point(left, bounds.top() + edge),
                                    size(mark_width, pixel),
                                ),
                                active,
                            ));
                        }
                    }
                }
                if let Some(hover) = &hover {
                    let top = bounds.size.height * (hover.bins.start as f32 / snapshot.bins as f32);
                    let height = (bounds.size.height
                        * (hover.bins.len() as f32 / snapshot.bins as f32))
                        .max(pixel);
                    window.paint_quad(gpui_kit::fill(
                        Bounds::new(
                            point(
                                bounds.left() + width * hover.lane as f32,
                                bounds.top() + top,
                            ),
                            size(width, height),
                        ),
                        active.opacity(0.25),
                    ));
                }
            },
        )
        .size_full();
        div()
            .id(SharedString::from(format!(
                "sidebar-overview-canvas-{panel:?}"
            )))
            .flex_1()
            .min_h_0()
            .min_w_0()
            .on_scroll_wheel(
                cx.listener(move |this, event: &ScrollWheelEvent, window, cx| {
                    this.scroll_overview(panel, event, window, cx)
                }),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.press_overview(panel, event, window, cx);
                    cx.stop_propagation();
                }),
            )
            .on_mouse_move(
                cx.listener(move |this, event: &MouseMoveEvent, window, cx| {
                    this.drag_overview(panel, event, window, cx)
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(move |this, _, _, _| this.overview_state_mut(panel).drag = None),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(move |this, _, _, _| this.overview_state_mut(panel).drag = None),
            )
            .on_hover(cx.listener(move |this, hovered, window, cx| {
                if *hovered {
                    this.hover_overview(panel, window.mouse_position(), window, cx);
                } else if this.overview_state_mut(panel).hover.take().is_some() {
                    cx.notify();
                }
            }))
            .child(overview)
            .into_any_element()
    }
}
