use std::{
    cell::RefCell,
    collections::{BTreeMap, VecDeque},
    marker::PhantomData,
    ops::{Deref, Range},
    rc::Rc,
};

use gpui::{
    AnyElement, App, AvailableSpace, Bounds, ContentMask, Context, Div, Element, ElementId, Entity,
    EventEmitter, GlobalElementId, Hitbox, InteractiveElement as _, IntoElement, Pixels, Point,
    Render, ScrollHandle, ScrollStrategy, Size, Stateful, StatefulInteractiveElement as _,
    StyleRefinement, Styled, Window, div, point, px, size,
};
use gpui_base::ScrollbarHandle as _;
use gpui_component::InteractiveElementExt as _;

const DEFAULT_MEASURED_HEIGHT_LIMIT: usize = 4096;

/// At most one unwrapped screen, including a partially clipped row. The last window is
/// filled backwards so layout can find the real bottom without reading the whole document.
pub(crate) fn viewport_read_range(
    first_row: usize,
    item_count: usize,
    viewport_height: Pixels,
    base_height: Pixels,
) -> Range<usize> {
    let count = (viewport_height / base_height.max(px(1.))).ceil().max(1.) as usize + 1;
    let start = first_row.min(item_count.saturating_sub(count));
    start..start.saturating_add(count).min(item_count)
}

/// Data contract shared by local logs, projected local results, and global results.
///
/// Rendering stays in the workspace presentation layer because it owns selection, focus, menus,
/// and navigation. The delegate supplies the stable projection, sizing inputs, and row model.
/// The renderer synchronously loads its bounded source window before querying those models.
pub(crate) trait VirtualLogListDelegate {
    type Key: Clone + Ord + 'static;
    type Row;

    fn row_count(&self) -> usize;
    fn stable_row_key(&self, row_ix: usize) -> Option<Self::Key>;
    fn minimum_row_height(&self) -> Pixels;
    fn row(&self, row_ix: usize) -> Option<Self::Row>;
    fn unwrapped_content_width(&self, cx: &App) -> Pixels;
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct VirtualLogListPosition {
    pub(crate) row_ix: usize,
    pub(crate) offset_in_row: Pixels,
}

#[derive(Clone, Copy, Debug)]
enum PendingScroll {
    Row {
        row_ix: usize,
        strategy: ScrollStrategy,
    },
    RowAtViewportY {
        row_ix: usize,
        viewport_y: Pixels,
    },
    End,
}

struct VirtualLogViewportInner<K> {
    item_count: usize,
    base_height: Pixels,
    position: VirtualLogListPosition,
    anchor_key: Option<K>,
    at_end: bool,
    pending_scroll: Option<PendingScroll>,
    visible_range: Range<usize>,
    viewport_bounds: Bounds<Pixels>,
    content_width: Pixels,
    horizontal_offset: Pixels,
    synced_native_offset: Point<Pixels>,
    measured_heights: BTreeMap<K, Pixels>,
    measured_order: VecDeque<K>,
    indexed_heights: BTreeMap<usize, (Option<K>, Pixels)>,
    indexed_order: VecDeque<usize>,
    measured_height_limit: usize,
}

impl<K> Default for VirtualLogViewportInner<K> {
    fn default() -> Self {
        Self {
            item_count: 0,
            base_height: px(0.),
            position: VirtualLogListPosition::default(),
            anchor_key: None,
            at_end: false,
            pending_scroll: None,
            visible_range: 0..0,
            viewport_bounds: Bounds::default(),
            content_width: px(0.),
            horizontal_offset: px(0.),
            synced_native_offset: Point::default(),
            measured_heights: BTreeMap::new(),
            measured_order: VecDeque::new(),
            indexed_heights: BTreeMap::new(),
            indexed_order: VecDeque::new(),
            measured_height_limit: DEFAULT_MEASURED_HEIGHT_LIMIT,
        }
    }
}

/// Retained viewport state for a virtual log list.
///
/// Position is represented by a row index and an offset inside that row. The state never builds
/// a prefix-height index, so jumping to a distant row is independent of the rows before it.
pub(crate) struct VirtualLogViewport<K> {
    inner: Rc<RefCell<VirtualLogViewportInner<K>>>,
    native_scroll: ScrollHandle,
}

impl<K> Clone for VirtualLogViewport<K> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            native_scroll: self.native_scroll.clone(),
        }
    }
}

impl<K: 'static> Default for VirtualLogViewport<K> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: 'static> VirtualLogViewport<K> {
    pub(crate) fn new() -> Self {
        Self {
            inner: Rc::new(RefCell::new(VirtualLogViewportInner::default())),
            native_scroll: ScrollHandle::new(),
        }
    }

    pub(crate) fn native_scroll_handle(&self) -> &ScrollHandle {
        &self.native_scroll
    }

    pub(crate) fn position(&self) -> VirtualLogListPosition {
        self.inner.borrow().position
    }

    pub(crate) fn anchor_key(&self) -> Option<K>
    where
        K: Clone,
    {
        self.inner.borrow().anchor_key.clone()
    }

    pub(crate) fn visible_range(&self) -> Range<usize> {
        self.inner.borrow().visible_range.clone()
    }

    pub(crate) fn viewport_bounds(&self) -> Bounds<Pixels> {
        self.inner.borrow().viewport_bounds
    }

    pub(crate) fn item_count(&self) -> usize {
        self.inner.borrow().item_count
    }

    pub(crate) fn base_height(&self) -> Pixels {
        self.inner.borrow().base_height
    }

    pub(crate) fn set_item_count(&self, item_count: usize, base_height: Pixels) {
        let mut inner = self.inner.borrow_mut();
        let base_height = base_height.max(px(1.));
        if inner.base_height != base_height {
            inner.base_height = base_height;
            inner.measured_heights.clear();
            inner.measured_order.clear();
            inner.indexed_heights.clear();
            inner.indexed_order.clear();
        }
        let previous_count = inner.item_count;
        inner.item_count = item_count;
        if item_count == 0 {
            inner.position = VirtualLogListPosition::default();
            inner.anchor_key = None;
            inner.at_end = false;
            inner.visible_range = 0..0;
        } else {
            inner.position.row_ix = inner.position.row_ix.min(item_count - 1);
            inner
                .indexed_heights
                .retain(|row_ix, _| *row_ix < item_count);
            inner.indexed_order.retain(|row_ix| *row_ix < item_count);
            if inner.at_end && item_count > previous_count {
                inner.pending_scroll = Some(PendingScroll::End);
            }
        }
    }

    pub(crate) fn set_content_width(&self, content_width: Pixels) {
        let mut inner = self.inner.borrow_mut();
        inner.content_width = content_width.max(inner.viewport_bounds.size.width);
        let max = (inner.content_width - inner.viewport_bounds.size.width).max(px(0.));
        inner.horizontal_offset = inner.horizontal_offset.clamp(px(0.), max);
    }

    pub(crate) fn horizontal_offset(&self) -> Pixels {
        self.inner.borrow().horizontal_offset
    }

    pub(crate) fn set_horizontal_offset(&self, offset: Pixels) {
        let mut inner = self.inner.borrow_mut();
        let max = (inner.content_width - inner.viewport_bounds.size.width).max(px(0.));
        inner.horizontal_offset = offset.clamp(px(0.), max);
    }

    pub(crate) fn scroll_to_row(&self, row_ix: usize, strategy: ScrollStrategy) {
        self.inner.borrow_mut().pending_scroll = Some(PendingScroll::Row { row_ix, strategy });
    }

    pub(crate) fn scroll_row_to_viewport_y(&self, row_ix: usize, viewport_y: Pixels) {
        self.inner.borrow_mut().pending_scroll =
            Some(PendingScroll::RowAtViewportY { row_ix, viewport_y });
    }

    pub(crate) fn preserve_row_at_viewport_y(&self, row_ix: usize, viewport_y: Pixels) {
        self.inner
            .borrow_mut()
            .pending_scroll
            .get_or_insert(PendingScroll::RowAtViewportY { row_ix, viewport_y });
    }

    pub(crate) fn scroll_to_end(&self) {
        self.inner.borrow_mut().pending_scroll = Some(PendingScroll::End);
    }

    pub(crate) fn is_at_end(&self) -> bool {
        self.inner.borrow().at_end
    }

    #[cfg(test)]
    pub(crate) fn set_position(&self, position: VirtualLogListPosition) {
        let mut inner = self.inner.borrow_mut();
        inner.pending_scroll = None;
        inner.at_end = false;
        inner.position = position;
        normalize_position(&mut inner);
    }

    pub(crate) fn row_height(&self, row_ix: usize) -> Pixels {
        indexed_height(&self.inner.borrow(), row_ix)
    }

    #[cfg(test)]
    pub(crate) fn has_measured_height(&self, row_ix: usize) -> bool {
        self.inner.borrow().indexed_heights.contains_key(&row_ix)
    }

    pub(crate) fn record_measured_height(&self, row_ix: usize, key: K, height: Pixels)
    where
        K: Clone + Ord,
    {
        let visible_range = self.visible_range();
        let bounds = self.viewport_bounds();
        self.record_layout(row_ix, key, height, visible_range, bounds);
    }

    pub(crate) fn record_indexed_height(&self, row_ix: usize, height: Pixels)
    where
        K: Ord,
    {
        let mut inner = self.inner.borrow_mut();
        if row_ix < inner.item_count {
            let height = height.max(inner.base_height);
            if let Some(old_ix) = inner.indexed_order.iter().position(|ix| *ix == row_ix) {
                inner.indexed_order.remove(old_ix);
            }
            inner.indexed_order.push_back(row_ix);
            inner.indexed_heights.insert(row_ix, (None, height));
            evict_indexed_heights(&mut inner);
        }
    }

    pub(crate) fn restore_anchor_by_key(
        &self,
        key: &K,
        row_for_key: impl FnOnce(&K) -> Option<usize>,
    ) where
        K: Clone,
    {
        let mut inner = self.inner.borrow_mut();
        if let Some(row_ix) = row_for_key(key) {
            inner.position.row_ix = row_ix.min(inner.item_count.saturating_sub(1));
            inner.anchor_key = Some(key.clone());
            normalize_position(&mut inner);
        }
    }

    #[cfg(test)]
    pub(crate) fn scroll_by_pixels(&self, delta: Pixels) -> bool
    where
        K: Clone,
    {
        if delta == px(0.) {
            return false;
        }
        let mut inner = self.inner.borrow_mut();
        if inner.item_count == 0 {
            return false;
        }
        let before = inner.position;
        inner.pending_scroll = None;
        inner.at_end = false;
        let mut position = inner.position;
        inner.at_end = scroll_position_by_pixels(&inner, &mut position, delta);
        inner.position = position;
        before != inner.position
    }

    /// Computes a future logical scrollbar offset without mutating the committed viewport.
    /// Input samples coalesce into the latest target; the next UI frame reads and lays it out.
    pub(crate) fn wheel_scroll_target(
        &self,
        current: Point<Pixels>,
        pixel_delta: Option<Pixels>,
        row_delta: Option<isize>,
    ) -> Option<Point<Pixels>> {
        let inner = self.inner.borrow();
        if inner.item_count == 0 {
            return None;
        }
        if inner.at_end
            && current == self.offset()
            && (pixel_delta.is_some_and(|delta| delta > px(0.))
                || row_delta.is_some_and(|delta| delta > 0))
        {
            return None;
        }
        let logical_top = (-current.y).max(px(0.));
        let (mut position, _) = position_for_logical_top(&inner, logical_top);
        let at_end = if let Some(delta) = row_delta.filter(|delta| *delta != 0) {
            position.row_ix = if delta.is_negative() {
                position.row_ix.saturating_sub(delta.unsigned_abs())
            } else {
                position
                    .row_ix
                    .saturating_add(delta as usize)
                    .min(inner.item_count - 1)
            };
            position.offset_in_row = px(0.);
            false
        } else {
            let delta = pixel_delta.filter(|delta| *delta != px(0.))?;
            scroll_position_by_pixels(&inner, &mut position, delta)
        };
        let bottom = bottom_position(&inner);
        let at_end = at_end || position_is_after(position, bottom);
        if at_end {
            position = bottom;
        }
        let target = point(
            current.x,
            -logical_top_for_position(&inner, position, at_end),
        );
        (target != current).then_some(target)
    }

    pub(crate) fn invalidate_measurements(&self) {
        let mut inner = self.inner.borrow_mut();
        inner.measured_heights.clear();
        inner.measured_order.clear();
        inner.indexed_heights.clear();
        inner.indexed_order.clear();
    }

    #[cfg(test)]
    pub(crate) fn measured_height_count(&self) -> usize {
        self.inner.borrow().measured_heights.len()
    }

    pub(crate) fn measured_heights(&self) -> BTreeMap<K, Pixels>
    where
        K: Clone,
    {
        self.inner.borrow().measured_heights.clone()
    }

    #[cfg(test)]
    fn set_measured_height_limit(&self, limit: usize) {
        self.inner.borrow_mut().measured_height_limit = limit;
    }

    fn record_layout(
        &self,
        row_ix: usize,
        key: K,
        height: Pixels,
        visible_range: Range<usize>,
        bounds: Bounds<Pixels>,
    ) where
        K: Clone + Ord,
    {
        let mut inner = self.inner.borrow_mut();
        let height = height.max(inner.base_height);
        if let Some(old_ix) = inner
            .measured_order
            .iter()
            .position(|candidate| candidate == &key)
        {
            inner.measured_order.remove(old_ix);
        }
        inner.measured_order.push_back(key.clone());
        inner.measured_heights.insert(key.clone(), height);
        inner.indexed_heights.insert(row_ix, (Some(key), height));
        if let Some(old_ix) = inner.indexed_order.iter().position(|ix| *ix == row_ix) {
            inner.indexed_order.remove(old_ix);
        }
        inner.indexed_order.push_back(row_ix);
        if row_ix == inner.position.row_ix {
            inner.anchor_key = inner
                .indexed_heights
                .get(&row_ix)
                .and_then(|(key, _)| key.clone());
        }
        while inner.measured_order.len() > inner.measured_height_limit {
            if let Some(evicted) = inner.measured_order.pop_front() {
                inner.measured_heights.remove(&evicted);
                inner
                    .indexed_heights
                    .retain(|_, (key, _)| key.as_ref().is_none_or(|key| key != &evicted));
            }
        }
        evict_indexed_heights(&mut inner);
        inner.visible_range = visible_range;
        inner.viewport_bounds = bounds;
    }

    fn update_frame(
        &self,
        position: VirtualLogListPosition,
        at_end: bool,
        visible_range: Range<usize>,
        bounds: Bounds<Pixels>,
    ) where
        K: Clone,
    {
        let mut inner = self.inner.borrow_mut();
        inner.position = position;
        inner.anchor_key = inner
            .indexed_heights
            .get(&position.row_ix)
            .and_then(|(key, _)| key.as_ref().cloned());
        inner.at_end = at_end;
        inner.visible_range = visible_range;
        inner.viewport_bounds = bounds;
        let max = (inner.content_width - bounds.size.width).max(px(0.));
        inner.horizontal_offset = inner.horizontal_offset.clamp(px(0.), max);
        normalize_position(&mut inner);
    }

    fn take_pending_scroll(&self) -> Option<PendingScroll> {
        self.inner.borrow_mut().pending_scroll.take()
    }

    pub(crate) fn read_range(&self, viewport_height: Pixels) -> Range<usize> {
        let pending = self.inner.borrow().pending_scroll;
        self.candidate_range(pending, viewport_height)
    }

    fn candidate_range(
        &self,
        pending: Option<PendingScroll>,
        viewport_height: Pixels,
    ) -> Range<usize> {
        let inner = self.inner.borrow();
        if inner.item_count == 0 {
            return 0..0;
        }
        let visible_slots = (viewport_height / inner.base_height.max(px(1.)))
            .ceil()
            .max(1.) as usize;
        let target = match pending {
            Some(PendingScroll::Row { row_ix, .. })
            | Some(PendingScroll::RowAtViewportY { row_ix, .. }) => {
                row_ix.min(inner.item_count - 1)
            }
            Some(PendingScroll::End) => inner.item_count - 1,
            None => inner.position.row_ix.min(inner.item_count - 1),
        };
        let start = match pending {
            Some(PendingScroll::Row {
                strategy: ScrollStrategy::Center,
                ..
            }) => target.saturating_sub(visible_slots / 2),
            Some(PendingScroll::Row {
                strategy: ScrollStrategy::Bottom,
                ..
            })
            | Some(PendingScroll::End) => target.saturating_sub(visible_slots),
            Some(PendingScroll::RowAtViewportY { viewport_y, .. }) if viewport_y > px(0.) => {
                let rows_before = (viewport_y / inner.base_height).ceil().max(0.) as usize;
                target.saturating_sub(rows_before)
            }
            _ => target,
        };
        viewport_read_range(start, inner.item_count, viewport_height, inner.base_height)
    }

    fn sync_native_scroll(&self) {
        let native = self.native_scroll.offset();
        let (synced, has_pending_scroll) = {
            let inner = self.inner.borrow();
            (inner.synced_native_offset, inner.pending_scroll.is_some())
        };
        // Geometry changes alter the logical offset without scrolling the native handle.
        // Only a change since our last write is input, and explicit navigation wins over it.
        if !has_pending_scroll {
            if (synced.x - native.x).abs() >= px(0.5) {
                self.set_horizontal_offset((-native.x).max(px(0.)));
            }
            if (synced.y - native.y).abs() >= px(0.5) {
                self.set_logical_top((-native.y).max(px(0.)));
            }
        }
        self.publish_native_scroll();
    }

    pub(crate) fn publish_native_scroll(&self) {
        let offset = self.offset();
        self.native_scroll.set_offset(offset);
        self.inner.borrow_mut().synced_native_offset = offset;
    }

    fn set_logical_top(&self, top: Pixels) {
        let mut inner = self.inner.borrow_mut();
        inner.pending_scroll = None;
        let (position, at_end) = position_for_logical_top(&inner, top);
        inner.position = position;
        inner.at_end = at_end;
        if at_end {
            inner.pending_scroll = Some(PendingScroll::End);
        }
    }
}

fn position_for_logical_top<K>(
    inner: &VirtualLogViewportInner<K>,
    top: Pixels,
) -> (VirtualLogListPosition, bool) {
    if inner.item_count == 0 {
        return (VirtualLogListPosition::default(), false);
    }
    let bottom = bottom_position(inner);
    let distance = logical_top_for_position(inner, bottom, false);
    let top = top.clamp(px(0.), distance.max(px(0.)));
    if top >= distance - px(0.5) {
        return (bottom, true);
    }
    let row_ix = ((top / inner.base_height).floor().max(0.) as usize).min(inner.item_count - 1);
    let fraction = ((top - inner.base_height * row_ix as f32) / inner.base_height).clamp(0., 1.);
    (
        VirtualLogListPosition {
            row_ix,
            offset_in_row: indexed_height(inner, row_ix) * fraction,
        },
        false,
    )
}

fn logical_top_for_position<K>(
    inner: &VirtualLogViewportInner<K>,
    position: VirtualLogListPosition,
    at_end: bool,
) -> Pixels {
    if inner.item_count == 0 {
        return px(0.);
    }
    let position = if at_end {
        bottom_position(inner)
    } else {
        position
    };
    let row_ix = position.row_ix.min(inner.item_count - 1);
    let height = indexed_height(inner, row_ix);
    let fraction = if height > px(0.) {
        (position.offset_in_row / height).clamp(0., 1.)
    } else {
        0.
    };
    inner.base_height * row_ix as f32 + inner.base_height * fraction
}

fn position_is_after(position: VirtualLogListPosition, other: VirtualLogListPosition) -> bool {
    position.row_ix > other.row_ix
        || (position.row_ix == other.row_ix && position.offset_in_row >= other.offset_in_row)
}

fn bottom_position<K>(inner: &VirtualLogViewportInner<K>) -> VirtualLogListPosition {
    let mut remaining = inner.viewport_bounds.size.height.max(px(0.));
    for row_ix in (0..inner.item_count).rev() {
        let height = indexed_height(inner, row_ix);
        if remaining <= height {
            return VirtualLogListPosition {
                row_ix,
                offset_in_row: height - remaining,
            };
        }
        remaining -= height;
    }
    VirtualLogListPosition::default()
}

fn scroll_position_by_pixels<K>(
    inner: &VirtualLogViewportInner<K>,
    position: &mut VirtualLogListPosition,
    delta: Pixels,
) -> bool {
    let mut at_end = false;
    if delta > px(0.) {
        let mut remaining = delta;
        while remaining > px(0.) {
            let height = indexed_height(inner, position.row_ix);
            let available = (height - position.offset_in_row).max(px(0.));
            if remaining < available || position.row_ix + 1 >= inner.item_count {
                position.offset_in_row = (position.offset_in_row + remaining).min(height);
                at_end =
                    position.row_ix + 1 >= inner.item_count && position.offset_in_row >= height;
                break;
            }
            remaining -= available;
            position.row_ix += 1;
            position.offset_in_row = px(0.);
        }
    } else {
        let mut remaining = -delta;
        while remaining > px(0.) {
            if remaining <= position.offset_in_row {
                position.offset_in_row -= remaining;
                break;
            }
            remaining -= position.offset_in_row;
            if position.row_ix == 0 {
                position.offset_in_row = px(0.);
                break;
            }
            position.row_ix -= 1;
            position.offset_in_row = indexed_height(inner, position.row_ix);
        }
    }
    at_end
}

fn indexed_height<K>(inner: &VirtualLogViewportInner<K>, row_ix: usize) -> Pixels {
    inner
        .indexed_heights
        .get(&row_ix)
        .map(|(_, height)| *height)
        .unwrap_or(inner.base_height)
        .max(inner.base_height)
}

fn evict_indexed_heights<K: Ord>(inner: &mut VirtualLogViewportInner<K>) {
    while inner.indexed_order.len() > inner.measured_height_limit {
        let Some(evicted_ix) = inner.indexed_order.pop_front() else {
            break;
        };
        if let Some((Some(key), _)) = inner.indexed_heights.remove(&evicted_ix) {
            inner.measured_heights.remove(&key);
            if let Some(key_ix) = inner
                .measured_order
                .iter()
                .position(|candidate| candidate == &key)
            {
                inner.measured_order.remove(key_ix);
            }
        }
    }
}

fn normalize_position<K>(inner: &mut VirtualLogViewportInner<K>) {
    if inner.item_count == 0 {
        inner.position = VirtualLogListPosition::default();
        inner.at_end = false;
        return;
    }
    inner.position.row_ix = inner.position.row_ix.min(inner.item_count - 1);
    loop {
        let height = indexed_height(inner, inner.position.row_ix);
        if inner.position.offset_in_row < height || inner.position.row_ix + 1 >= inner.item_count {
            inner.position.offset_in_row = inner.position.offset_in_row.clamp(px(0.), height);
            break;
        }
        inner.position.offset_in_row -= height;
        inner.position.row_ix += 1;
    }
}

/// Scrollbar adapter exposing logical row progress instead of accumulated physical row heights.
#[derive(Clone)]
pub(crate) struct VirtualLogListScrollHandle<K> {
    viewport: VirtualLogViewport<K>,
}

impl<K: 'static> VirtualLogListScrollHandle<K> {
    pub(crate) fn new(viewport: &VirtualLogViewport<K>) -> Self {
        Self {
            viewport: viewport.clone(),
        }
    }

    pub(crate) fn scroll_to_item(&self, row_ix: usize, strategy: ScrollStrategy) {
        self.viewport.scroll_to_row(row_ix, strategy);
    }

    pub(crate) fn scroll_to_bottom(&self) {
        self.viewport.scroll_to_end();
    }
}

impl<K: 'static> Deref for VirtualLogListScrollHandle<K> {
    type Target = ScrollHandle;

    fn deref(&self) -> &Self::Target {
        self.viewport.native_scroll_handle()
    }
}

impl<K: 'static> gpui_base::ScrollbarHandle for VirtualLogListScrollHandle<K> {
    fn viewport_bounds(&self) -> Bounds<Pixels> {
        self.viewport.viewport_bounds()
    }

    fn offset(&self) -> Point<Pixels> {
        let inner = self.viewport.inner.borrow();
        let logical_top = logical_top_for_position(&inner, inner.position, inner.at_end);
        point(-inner.horizontal_offset, -logical_top)
    }

    fn set_offset(&self, offset: Point<Pixels>) {
        self.viewport.set_horizontal_offset((-offset.x).max(px(0.)));
        self.viewport.set_logical_top((-offset.y).max(px(0.)));
    }

    fn content_size(&self) -> Size<Pixels> {
        let inner = self.viewport.inner.borrow();
        size(
            inner.content_width.max(inner.viewport_bounds.size.width),
            inner.viewport_bounds.size.height
                + logical_top_for_position(&inner, bottom_position(&inner), false),
        )
    }
}

impl<K: 'static> gpui_base::ScrollbarHandle for VirtualLogViewport<K> {
    fn viewport_bounds(&self) -> Bounds<Pixels> {
        VirtualLogListScrollHandle::new(self).viewport_bounds()
    }

    fn offset(&self) -> Point<Pixels> {
        VirtualLogListScrollHandle::new(self).offset()
    }

    fn set_offset(&self, offset: Point<Pixels>) {
        VirtualLogListScrollHandle::new(self).set_offset(offset);
    }

    fn content_size(&self) -> Size<Pixels> {
        VirtualLogListScrollHandle::new(self).content_size()
    }
}

#[derive(Clone, Debug)]
pub(crate) enum VirtualLogListEvent {
    SelectRow(usize),
    ClearSelection,
}

#[derive(Debug, Default)]
pub(crate) struct VirtualLogVisibleRange {
    rows: Range<usize>,
}

impl VirtualLogVisibleRange {
    pub(crate) fn rows(&self) -> &Range<usize> {
        &self.rows
    }
}

/// Entity-owned delegate and interaction state for a virtual log list.
pub(crate) struct VirtualLogListState<M, K> {
    delegate: M,
    viewport: VirtualLogViewport<K>,
    visible_range: VirtualLogVisibleRange,
    selected_row: Option<usize>,
    _key: PhantomData<K>,
}

impl<M: 'static, K: 'static> VirtualLogListState<M, K> {
    pub(crate) fn new(delegate: M, viewport: VirtualLogViewport<K>) -> Self {
        Self {
            delegate,
            viewport,
            visible_range: VirtualLogVisibleRange::default(),
            selected_row: None,
            _key: PhantomData,
        }
    }

    pub(crate) fn delegate(&self) -> &M {
        &self.delegate
    }

    pub(crate) fn delegate_mut(&mut self) -> &mut M {
        &mut self.delegate
    }

    pub(crate) fn viewport(&self) -> &VirtualLogViewport<K> {
        &self.viewport
    }

    pub(crate) fn visible_range(&self) -> &VirtualLogVisibleRange {
        &self.visible_range
    }

    pub(crate) fn set_visible_range(&mut self, range: Range<usize>) {
        self.visible_range.rows = range;
    }

    pub(crate) fn selected_row(&self) -> Option<usize> {
        self.selected_row
    }

    pub(crate) fn set_selected_row(&mut self, row_ix: usize, cx: &mut Context<Self>)
    where
        M: 'static,
        K: 'static,
    {
        self.selected_row = Some(row_ix);
        cx.emit(VirtualLogListEvent::SelectRow(row_ix));
        cx.notify();
    }

    pub(crate) fn clear_selection(&mut self, cx: &mut Context<Self>)
    where
        M: 'static,
        K: 'static,
    {
        self.selected_row = None;
        cx.emit(VirtualLogListEvent::ClearSelection);
        cx.notify();
    }

    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        cx.notify();
    }

    pub(crate) fn scroll_to_row(&mut self, row_ix: usize, cx: &mut Context<Self>)
    where
        K: Clone + Ord,
    {
        self.viewport.scroll_to_row(row_ix, ScrollStrategy::Nearest);
        cx.notify();
    }
}

impl<M: 'static, K: 'static> EventEmitter<VirtualLogListEvent> for VirtualLogListState<M, K> {}

impl<M: 'static, K: 'static> Render for VirtualLogListState<M, K> {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

pub(crate) struct VirtualLogRow<K> {
    pub(crate) row_ix: usize,
    pub(crate) key: K,
    pub(crate) element: AnyElement,
}

impl<K> VirtualLogRow<K> {
    pub(crate) fn new(row_ix: usize, key: K, element: impl IntoElement) -> Self {
        Self {
            row_ix,
            key,
            element: element.into_any_element(),
        }
    }
}

type RenderRows<K> =
    dyn for<'a> Fn(Range<usize>, &'a mut Window, &'a mut App) -> Vec<VirtualLogRow<K>>;

pub(crate) fn v_virtual_log_list<V, K>(
    view: Entity<V>,
    id: impl Into<ElementId>,
    viewport: VirtualLogViewport<K>,
    item_count: usize,
    base_height: Pixels,
    content_width: Pixels,
    render: impl 'static
    + Fn(&mut V, Range<usize>, &mut Window, &mut Context<V>) -> Vec<VirtualLogRow<K>>,
) -> VirtualLogList<K>
where
    V: Render,
    K: Clone + Ord + 'static,
{
    let id = id.into();
    viewport.set_item_count(item_count, base_height);
    viewport.set_content_width(content_width);
    let render_rows = move |range: Range<usize>, window: &mut Window, cx: &mut App| {
        view.update(cx, |view, cx| render(view, range, window, cx))
    };
    VirtualLogList {
        id: id.clone(),
        base: div()
            .id(id)
            .size_full()
            .overflow_scroll()
            .lock_scroll_axis()
            .track_scroll(viewport.native_scroll_handle()),
        viewport,
        render_rows: Box::new(render_rows),
    }
}

pub(crate) struct VirtualLogList<K> {
    id: ElementId,
    base: Stateful<Div>,
    viewport: VirtualLogViewport<K>,
    render_rows: Box<RenderRows<K>>,
}

impl<K> Styled for VirtualLogList<K> {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

pub(crate) struct VirtualLogListFrame<K> {
    rows: Vec<(usize, K, AnyElement, Size<Pixels>)>,
}

impl<K: Clone + Ord + 'static> Element for VirtualLogList<K> {
    type RequestLayoutState = VirtualLogListFrame<K>;
    type PrepaintState = Option<Hitbox>;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui::LayoutId, Self::RequestLayoutState) {
        let layout_id = self.base.interactivity().request_layout(
            global_id,
            inspector_id,
            window,
            cx,
            |style, window, cx| window.request_layout(style, None, cx),
        );
        (layout_id, VirtualLogListFrame { rows: Vec::new() })
    }

    fn prepaint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        frame: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.viewport.sync_native_scroll();
        let style = self
            .base
            .interactivity()
            .compute_style(global_id, None, window, cx);
        let borders = style.border_widths.to_pixels(window.rem_size());
        let padding = style
            .padding
            .to_pixels(bounds.size.into(), window.rem_size());
        let content_bounds = Bounds::from_corners(
            bounds.origin + point(borders.left + padding.left, borders.top + padding.top),
            bounds.bottom_right()
                - point(
                    borders.right + padding.right,
                    borders.bottom + padding.bottom,
                ),
        );
        let pending = self
            .viewport
            .take_pending_scroll()
            .or_else(|| self.viewport.is_at_end().then_some(PendingScroll::End));
        let candidate_range = self
            .viewport
            .candidate_range(pending, content_bounds.size.height);
        let rows = (self.render_rows)(candidate_range.clone(), window, cx);
        let available_space = size(
            AvailableSpace::Definite(content_bounds.size.width),
            AvailableSpace::MinContent,
        );
        let mut measured = Vec::with_capacity(candidate_range.len());
        for VirtualLogRow {
            row_ix,
            key,
            mut element,
        } in rows
        {
            let measured_size = element.layout_as_root(available_space, window, cx);
            measured.push((row_ix, key, element, measured_size));
        }

        let (position, at_end) = resolve_position(
            self.viewport.position(),
            pending,
            &measured,
            content_bounds.size.height,
            self.viewport.item_count(),
        );
        let position_prefix = measured
            .iter()
            .take_while(|(row_ix, _, _, _)| *row_ix < position.row_ix)
            .fold(px(0.), |height, (_, _, _, size)| height + size.height);
        let mut y = content_bounds.top() - position.offset_in_row - position_prefix;
        let mut first_visible = None;
        let mut last_visible = candidate_range.start;

        window.with_content_mask(
            Some(ContentMask {
                bounds: content_bounds,
            }),
            |window| {
                for (row_ix, key, row, measured_size) in &mut measured {
                    let origin = point(content_bounds.left(), y);
                    let row_bounds = Bounds::new(origin, *measured_size);
                    if row_bounds.bottom() > content_bounds.top()
                        && row_bounds.top() < content_bounds.bottom()
                    {
                        first_visible.get_or_insert(*row_ix);
                        last_visible = row_ix.saturating_add(1);
                        row.prepaint_at(origin, window, cx);
                    }
                    y += measured_size.height;
                    self.viewport.record_layout(
                        *row_ix,
                        key.clone(),
                        measured_size.height,
                        first_visible.unwrap_or(*row_ix)..last_visible,
                        content_bounds,
                    );
                }
            },
        );

        let visible_range = first_visible.unwrap_or(candidate_range.start)..last_visible;
        self.viewport
            .update_frame(position, at_end, visible_range.clone(), content_bounds);
        self.viewport.publish_native_scroll();
        measured.retain(|(row_ix, _, _, _)| visible_range.contains(row_ix));
        frame.rows = measured;

        let logical_content_size = self.viewport.content_size();
        self.base.interactivity().prepaint(
            global_id,
            inspector_id,
            bounds,
            logical_content_size,
            window,
            cx,
            |_, _, hitbox, _, _| hitbox,
        )
    }

    fn paint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        frame: &mut Self::RequestLayoutState,
        hitbox: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.base.interactivity().paint(
            global_id,
            inspector_id,
            bounds,
            hitbox.as_ref(),
            window,
            cx,
            |_, window, cx| {
                for (_, _, row, _) in &mut frame.rows {
                    row.paint(window, cx);
                }
            },
        )
    }
}

impl<K: Clone + Ord + 'static> IntoElement for VirtualLogList<K> {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

fn resolve_position<K>(
    current: VirtualLogListPosition,
    pending: Option<PendingScroll>,
    rows: &[(usize, K, AnyElement, Size<Pixels>)],
    viewport_height: Pixels,
    item_count: usize,
) -> (VirtualLogListPosition, bool) {
    if item_count == 0 || rows.is_empty() {
        return (VirtualLogListPosition::default(), false);
    }
    let first_ix = rows[0].0;
    let last_ix = rows.last().map(|row| row.0).unwrap_or(first_ix);
    let height_before = |target: usize| {
        rows.iter()
            .take_while(|(row_ix, _, _, _)| *row_ix < target)
            .fold(px(0.), |height, (_, _, _, size)| height + size.height)
    };
    let total_height = rows
        .iter()
        .fold(px(0.), |height, (_, _, _, size)| height + size.height);
    let target_height = |target: usize| {
        rows.iter()
            .find(|(row_ix, _, _, _)| *row_ix == target)
            .map(|(_, _, _, size)| size.height)
            .unwrap_or(px(1.))
    };

    let desired_top = match pending {
        Some(PendingScroll::End) => (total_height - viewport_height).max(px(0.)),
        Some(PendingScroll::Row { row_ix, strategy }) => {
            let row_ix = row_ix.clamp(first_ix, last_ix);
            let top = height_before(row_ix);
            let height = target_height(row_ix);
            match strategy {
                ScrollStrategy::Top => top,
                ScrollStrategy::Center => top + height / 2. - viewport_height / 2.,
                ScrollStrategy::Bottom => top + height - viewport_height,
                ScrollStrategy::Nearest => {
                    if row_ix < current.row_ix {
                        top
                    } else if row_ix > current.row_ix {
                        top + height - viewport_height
                    } else {
                        height_before(current.row_ix) + current.offset_in_row
                    }
                }
            }
        }
        Some(PendingScroll::RowAtViewportY { row_ix, viewport_y }) => {
            let row_ix = row_ix.clamp(first_ix, last_ix);
            // A collapsed wrapped row may be shorter than its old clipped portion.
            // Keep that source row at the top instead of skipping to subsequent rows.
            let viewport_y = if -viewport_y >= target_height(row_ix) {
                px(0.)
            } else {
                viewport_y
            };
            height_before(row_ix) - viewport_y
        }
        None => {
            let current_row = current.row_ix.clamp(first_ix, last_ix);
            height_before(current_row) + current.offset_in_row
        }
    };
    // The read window is not the document. Clamp to its bottom only when it contains EOF.
    let max_top = (total_height - viewport_height).max(px(0.));
    let reaches_end = last_ix.saturating_add(1) == item_count;
    let desired_top = if reaches_end {
        desired_top.clamp(px(0.), max_top)
    } else {
        desired_top.max(px(0.))
    };
    let at_end = reaches_end && desired_top >= max_top;
    let mut remaining = desired_top;
    for (row_ix, _, _, size) in rows {
        if remaining < size.height || *row_ix == last_ix {
            return (
                VirtualLogListPosition {
                    row_ix: *row_ix,
                    offset_in_row: remaining.clamp(px(0.), size.height),
                },
                at_end,
            );
        }
        remaining -= size.height;
    }
    (
        VirtualLogListPosition {
            row_ix: last_ix,
            offset_in_row: px(0.),
        },
        at_end,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    struct Harness {
        viewport: VirtualLogViewport<usize>,
        visible_ranges: Rc<RefCell<Vec<Range<usize>>>>,
    }

    impl Render for Harness {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let visible_ranges = self.visible_ranges.clone();
            v_virtual_log_list(
                cx.entity(),
                "virtual-log-list-test",
                self.viewport.clone(),
                1_000_000,
                px(20.),
                px(80.),
                move |_, range, _, _| {
                    visible_ranges.borrow_mut().push(range.clone());
                    range
                        .map(|row_ix| {
                            VirtualLogRow::new(
                                row_ix,
                                row_ix,
                                div().w(px(80.)).h(if row_ix == 800_000 {
                                    px(100.)
                                } else {
                                    px(20.)
                                }),
                            )
                        })
                        .collect()
                },
            )
            .w(px(80.))
            .h(px(60.))
        }
    }

    #[gpui::test]
    fn element_measures_only_the_distant_visible_window(cx: &mut TestAppContext) {
        let viewport = VirtualLogViewport::new();
        let visible_ranges = Rc::new(RefCell::new(Vec::new()));
        let (_, cx) = cx.add_window_view({
            let viewport = viewport.clone();
            let visible_ranges = visible_ranges.clone();
            move |_, _| Harness {
                viewport,
                visible_ranges,
            }
        });

        cx.update(|window, cx| window.draw(cx).clear(cx));
        viewport.scroll_to_row(800_000, ScrollStrategy::Top);
        cx.update(|window, cx| window.draw(cx).clear(cx));

        let rendered = visible_ranges.borrow().last().cloned().unwrap();
        assert!(rendered.contains(&800_000));
        assert!(rendered.len() < 16);
        assert_eq!(viewport.measured_heights().get(&800_000), Some(&px(100.)));
        assert!(viewport.measured_height_count() < 32);
    }

    #[test]
    fn distant_row_jump_is_indexed_without_preceding_height_state() {
        let viewport = VirtualLogViewport::<usize>::new();
        viewport.set_item_count(10_000_000, px(20.));
        viewport.scroll_to_row(8_000_000, ScrollStrategy::Top);
        let pending = viewport.take_pending_scroll();
        let range = viewport.candidate_range(pending, px(400.));

        assert!(range.contains(&8_000_000));
        assert!(range.len() < 32);
        assert_eq!(viewport.measured_height_count(), 0);
    }

    #[test]
    fn measured_height_cache_is_bounded_independently_of_item_count() {
        let viewport = VirtualLogViewport::<usize>::new();
        viewport.set_item_count(10_000_000, px(20.));
        viewport.set_measured_height_limit(4);
        for row_ix in 0..10 {
            viewport.record_layout(
                row_ix,
                row_ix,
                px(20. + row_ix as f32),
                row_ix..row_ix + 1,
                Bounds::default(),
            );
        }

        assert_eq!(viewport.measured_height_count(), 4);
        assert_eq!(viewport.inner.borrow().indexed_heights.len(), 4);
        assert_eq!(
            viewport
                .measured_heights()
                .keys()
                .copied()
                .collect::<Vec<_>>(),
            vec![6, 7, 8, 9]
        );
    }

    #[test]
    fn temporary_indexed_measurements_use_the_same_fixed_capacity() {
        let viewport = VirtualLogViewport::<usize>::new();
        viewport.set_item_count(10_000_000, px(20.));
        viewport.set_measured_height_limit(4);
        for row_ix in 0..10 {
            viewport.record_indexed_height(row_ix, px(40.));
        }

        let inner = viewport.inner.borrow();
        assert_eq!(inner.indexed_heights.len(), 4);
        assert_eq!(
            inner.indexed_order.iter().copied().collect::<Vec<_>>(),
            vec![6, 7, 8, 9]
        );
        assert!(inner.measured_heights.is_empty());
    }

    #[test]
    fn pixel_scroll_consumes_the_current_rows_real_height() {
        let viewport = VirtualLogViewport::<usize>::new();
        viewport.set_item_count(3, px(20.));
        viewport.record_layout(0, 0, px(100.), 0..1, Bounds::default());

        assert!(viewport.scroll_by_pixels(px(70.)));
        assert_eq!(
            viewport.position(),
            VirtualLogListPosition {
                row_ix: 0,
                offset_in_row: px(70.)
            }
        );
        assert!(viewport.scroll_by_pixels(px(40.)));
        assert_eq!(
            viewport.position(),
            VirtualLogListPosition {
                row_ix: 1,
                offset_in_row: px(10.)
            }
        );
    }

    #[test]
    fn logical_scrollbar_maps_directly_to_a_row_and_fraction() {
        let viewport = VirtualLogViewport::<usize>::new();
        viewport.set_item_count(1_000, px(20.));
        viewport.record_layout(400, 400, px(100.), 400..401, Bounds::default());
        let handle = VirtualLogListScrollHandle::new(&viewport);

        gpui_base::ScrollbarHandle::set_offset(&handle, point(px(0.), -px(8_010.)));

        assert_eq!(viewport.position().row_ix, 400);
        assert_eq!(viewport.position().offset_in_row, px(50.));
    }
}
