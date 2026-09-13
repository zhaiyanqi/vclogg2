use super::overview::{COLOR_TRACKS_PER_PAGE, OVERVIEW_BINS, OverviewSnapshot};
use super::*;

#[derive(Default)]
pub(super) struct OverviewState {
    pub(super) job: Option<SidebarJob>,
    pub(super) request: u64,
    pub(super) dirty: bool,
    pub(super) snapshot: Option<Arc<OverviewSnapshot>>,
    pub(super) search: CompressedRows,
    pub(super) color_page: usize,
    pub(super) zoom: Option<Range<usize>>,
    pub(super) hover: Option<OverviewHover>,
    pub(super) anchor: Option<usize>,
    pub(super) drag: Option<(f64, f64)>,
    pub(super) bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
}

#[derive(Clone, PartialEq)]
pub(super) struct OverviewHover {
    pub(super) lane: usize,
    pub(super) bins: Range<usize>,
}

impl OverviewState {
    pub(super) fn reset_source(&mut self) {
        self.zoom = None;
        self.color_page = 0;
        self.anchor = None;
        self.hover = None;
        self.drag = None;
        self.bounds.set(None);
    }

    pub(super) fn range(&self, count: usize) -> Range<usize> {
        self.zoom.as_ref().map_or(0..count, |range| {
            range.start.min(count.saturating_sub(1))..range.end.min(count)
        })
    }
}

impl SidebarState {
    pub(super) fn overview_state(&self, panel: SidebarPanelId) -> &OverviewState {
        match panel {
            SidebarPanelId::Minimap => &self.minimap,
            SidebarPanelId::Colors => &self.color_overview,
            _ => unreachable!("panel has no overview"),
        }
    }

    pub(super) fn overview_state_mut(&mut self, panel: SidebarPanelId) -> &mut OverviewState {
        match panel {
            SidebarPanelId::Minimap => &mut self.minimap,
            SidebarPanelId::Colors => &mut self.color_overview,
            _ => unreachable!("panel has no overview"),
        }
    }

    pub(super) fn overview_source_count(&self) -> usize {
        self.document
            .as_ref()
            .map_or(0, |(_, document)| document.source_line_count())
    }

    pub(super) fn overview_range(&self, panel: SidebarPanelId) -> Range<usize> {
        let total = self.overview_source_count();
        self.overview_state(panel).range(total)
    }

    pub(super) fn overview_current_row(&self) -> usize {
        self.viewport
            .as_ref()
            .map_or(0, |viewport| {
                overview_viewport(
                    viewport,
                    self.document
                        .as_ref()
                        .map(|(_, document)| document.as_ref()),
                )
                .center
                .floor() as usize
            })
            .min(self.overview_source_count().saturating_sub(1))
    }

    pub(super) fn zoom_overview(
        &mut self,
        panel: SidebarPanelId,
        zoom_in: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if panel != SidebarPanelId::Minimap {
            return;
        }
        let total = self.overview_source_count();
        if total == 0 {
            return;
        }
        let range = self.overview_range(panel);
        let length = if zoom_in {
            if range.len() <= OVERVIEW_BINS {
                return;
            }
            range.len().div_ceil(4).max(OVERVIEW_BINS).min(total)
        } else {
            range.len().saturating_mul(4).min(total)
        };
        let center = self
            .overview_state(panel)
            .anchor
            .filter(|row| range.contains(row))
            .unwrap_or_else(|| {
                self.overview_current_row()
                    .clamp(range.start, range.end - 1)
            });
        let start = center.saturating_sub(length / 2).min(total - length);
        self.overview_state_mut(panel).zoom = (length < total).then_some(start..start + length);
        self.refresh_overview_range(panel, window, cx);
    }

    pub(super) fn reset_overview_zoom(
        &mut self,
        panel: SidebarPanelId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.overview_state_mut(panel).zoom = None;
        self.overview_state_mut(panel).anchor = None;
        self.refresh_overview_range(panel, window, cx);
    }

    pub(super) fn scroll_overview(
        &mut self,
        panel: SidebarPanelId,
        event: &ScrollWheelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !event.modifiers.control {
            return;
        }
        let Some(bounds) = self
            .overview_state(panel)
            .bounds
            .get()
            .filter(|bounds| bounds.contains(&event.position))
        else {
            return;
        };
        cx.stop_propagation();
        let delta = event.delta.pixel_delta(window.line_height()).y;
        let total = self.overview_source_count();
        let range = self.overview_range(panel);
        if delta == px(0.) || range.is_empty() {
            return;
        }
        // Use the wheel distance so high-resolution scrolling does not zoom one
        // whole level per event. Keep the source point at the same canvas ratio.
        let steps = (delta / window.line_height().max(px(1.))) as f64;
        let factor = 2_f64.powf((-steps / 4.).clamp(-4., 4.));
        let length =
            ((range.len() as f64 * factor).round() as usize).clamp(total.min(OVERVIEW_BINS), total);
        if length == range.len() {
            return;
        }
        let ratio = pointer_fraction(event.position.y, bounds.top(), bounds.size.height);
        let anchor = range.start as f64 + ratio * range.len() as f64;
        let start = (anchor - ratio * length as f64)
            .round()
            .clamp(0., (total - length) as f64) as usize;
        let state = self.overview_state_mut(panel);
        state.zoom = (length < total).then_some(start..start + length);
        state.anchor = Some((anchor.floor() as usize).min(total - 1));
        self.refresh_overview_range(panel, window, cx);
    }

    fn refresh_overview_range(
        &mut self,
        panel: SidebarPanelId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let state = self.overview_state_mut(panel);
        state.dirty = true;
        state.hover = None;
        state.drag = None;
        self.prepare_overview(panel, cx);
        self.focus[&panel].focus(window, cx);
        cx.notify();
    }

    pub(super) fn page_overview_colors(
        &mut self,
        panel: SidebarPanelId,
        next: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let last = self.colors.len().saturating_sub(1) / COLOR_TRACKS_PER_PAGE;
        self.overview_state_mut(panel).color_page = if next {
            self.overview_state(panel)
                .color_page
                .saturating_add(1)
                .min(last)
        } else {
            self.overview_state(panel).color_page.saturating_sub(1)
        };
        self.refresh_overview_range(panel, window, cx);
    }

    pub(super) fn hover_overview(
        &mut self,
        panel: SidebarPanelId,
        position: Point<Pixels>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let Some(snapshot) = &self.overview_state(panel).snapshot else {
            return;
        };
        let Some(bounds) = self
            .overview_state(panel)
            .bounds
            .get()
            .filter(|bounds| bounds.contains(&position))
        else {
            return;
        };
        let y = pointer_fraction(position.y, bounds.top(), bounds.size.height);
        let x = pointer_fraction(position.x, bounds.left(), bounds.size.width);
        let lane = ((x * snapshot.lane_count() as f64) as usize).min(snapshot.lane_count() - 1);
        let stride = snapshot.stride(bounds, window.scale_factor());
        let bin = ((y * snapshot.bins as f64) as usize).min(snapshot.bins - 1) / stride * stride;
        let hover = OverviewHover {
            lane,
            bins: bin..(bin + stride).min(snapshot.bins),
        };
        let anchor = pointer_row(&snapshot.range, y).0;
        self.overview_state_mut(panel).anchor = Some(anchor);
        if self.overview_state(panel).hover.as_ref() != Some(&hover) {
            self.overview_state_mut(panel).hover = Some(hover);
            cx.notify();
        }
    }

    pub(super) fn press_overview(
        &mut self,
        panel: SidebarPanelId,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.hover_overview(panel, event.position, window, cx);
        self.focus[&panel].focus(window, cx);
        if panel == SidebarPanelId::Minimap && event.click_count == 2 {
            self.zoom_overview(panel, true, window, cx);
            return;
        }
        let Some(snapshot) = &self.overview_state(panel).snapshot else {
            return;
        };
        let Some(hover) = &self.overview_state(panel).hover else {
            return;
        };
        let Some(bounds) = self.overview_state(panel).bounds.get() else {
            return;
        };
        let ratio = pointer_fraction(event.position.y, bounds.top(), bounds.size.height);
        let (row, fraction) = pointer_row(&snapshot.range, ratio);
        if snapshot.position_track && hover.lane == 0 {
            self.jump_centered(row, fraction, window, cx);
            self.overview_state_mut(panel).drag = Some((ratio, row as f64 + fraction as f64));
        } else {
            let rows = snapshot.rows(hover.bins.clone());
            let row = row.clamp(rows.start, rows.end - 1);
            let target = snapshot.tracks[hover.lane - usize::from(snapshot.position_track)]
                .target(rows, row);
            self.jump_centered(target, 0.5, window, cx);
            if panel == SidebarPanelId::Colors {
                // Continue from the actual click target, then move freely through
                // source rows instead of snapping to sparse color hits on every move.
                self.overview_state_mut(panel).drag = Some((ratio, target as f64 + 0.5));
            }
        }
        cx.notify();
    }

    pub(super) fn drag_overview(
        &mut self,
        panel: SidebarPanelId,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.hover_overview(panel, event.position, window, cx);
        if event.pressed_button != Some(MouseButton::Left) {
            self.overview_state_mut(panel).drag = None;
            return;
        }
        let Some((origin, anchor)) = self.overview_state(panel).drag else {
            return;
        };
        let Some(bounds) = self.overview_state(panel).bounds.get() else {
            return;
        };
        let range = self.overview_range(panel);
        if range.is_empty() {
            return;
        }
        let ratio = pointer_fraction(event.position.y, bounds.top(), bounds.size.height);
        let target = (anchor + (ratio - origin) * range.len() as f64)
            .clamp(range.start as f64, (range.end - 1) as f64);
        self.jump_centered(target.floor() as usize, target.fract() as f32, window, cx);
    }

    pub(super) fn overview_key(
        &mut self,
        panel: SidebarPanelId,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = self.overview_range(panel);
        if range.is_empty() {
            return;
        }
        let current = self
            .overview_current_row()
            .clamp(range.start, range.end - 1);
        let page = self
            .viewport
            .as_ref()
            .map_or(1, |viewport| viewport.visible_range().len().max(1));
        let row = match event.keystroke.key.as_str() {
            "up" => current.saturating_sub(1),
            "down" => current.saturating_add(1),
            "pageup" => current.saturating_sub(page),
            "pagedown" => current.saturating_add(page),
            "home" => range.start,
            "end" => range.end - 1,
            "escape" => {
                if self.overview_state(panel).zoom.is_some() {
                    self.reset_overview_zoom(panel, window, cx);
                } else {
                    self.overview_state_mut(panel).drag = None;
                    self.overview_state_mut(panel).hover = None;
                    cx.notify();
                }
                cx.stop_propagation();
                return;
            }
            _ => return,
        };
        let row = row.clamp(range.start, range.end - 1);
        self.overview_state_mut(panel).anchor = Some(row);
        self.jump_centered(row, 0.5, window, cx);
        cx.stop_propagation();
    }
}

fn pointer_fraction(position: Pixels, start: Pixels, extent: Pixels) -> f64 {
    // One pixel is a numeric geometry guard, not a UI spacing value.
    (f32::from(position - start) as f64 / f32::from(extent.max(px(1.))) as f64).clamp(0., 1.)
}

fn pointer_row(range: &Range<usize>, ratio: f64) -> (usize, f32) {
    let offset = ratio * range.len() as f64;
    let row = range.start + (offset.floor() as usize).min(range.len() - 1);
    (row, offset.fract() as f32)
}

pub(super) struct OverviewViewport {
    first: f64,
    end: f64,
    center: f64,
}

/// Measured viewport in source coordinates, including its actual screen center.
pub(super) fn overview_viewport(
    viewport: &VirtualLogViewport<LogRowKey>,
    document: Option<&LogDocument>,
) -> OverviewViewport {
    let position = viewport.position();
    let source = |row| {
        document
            .and_then(|document| document.source_row(row))
            .unwrap_or(row)
    };
    let first = source(position.row_ix) as f64
        + (position.offset_in_row / viewport.row_height(position.row_ix).max(px(1.))) as f64;
    let mut remaining = viewport.viewport_bounds().size.height;
    let mut to_center = remaining / 2.;
    let mut center = None;
    let mut end = first;
    for row in position.row_ix..viewport.visible_range().end {
        let height = viewport.row_height(row).max(px(1.));
        let offset = if row == position.row_ix {
            position.offset_in_row
        } else {
            px(0.)
        };
        let used = remaining.min((height - offset).max(px(0.)));
        if center.is_none() && to_center <= used {
            center = Some(source(row) as f64 + ((offset + to_center) / height) as f64);
        }
        to_center -= used;
        end = source(row) as f64 + ((offset + used) / height).min(1.) as f64;
        remaining -= used;
        if remaining <= px(0.) {
            break;
        }
    }
    OverviewViewport {
        first,
        end,
        center: center.unwrap_or((first + end) / 2.),
    }
}

pub(super) fn overview_thumb(
    rows: OverviewViewport,
    range: &Range<usize>,
    bounds: Bounds<Pixels>,
    pixel: Pixels,
) -> Option<(Pixels, Pixels)> {
    let first = rows.first.max(range.start as f64);
    let end = rows.end.min(range.end as f64);
    if first >= end || range.is_empty() {
        return None;
    }
    // Project both visible edges independently. With wrapped rows, the screen
    // center is not necessarily halfway between their source coordinates.
    let top = bounds.size.height * ((first - range.start as f64) / range.len() as f64) as f32;
    let height = bounds.size.height * ((end - first) / range.len() as f64) as f32;
    // A subpixel viewport is a locator line, never a minimum-sized selection box.
    let painted_height = height.max(pixel).min(bounds.size.height);
    let top = (top - (painted_height - height) / 2.)
        .clamp(px(0.), (bounds.size.height - painted_height).max(px(0.)));
    Some((top, painted_height))
}
