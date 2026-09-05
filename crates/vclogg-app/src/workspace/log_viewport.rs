use super::*;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct RowViewportPosition {
    pub(super) row_ix: usize,
    pub(super) viewport_y: Pixels,
}

#[derive(Clone, Copy)]
pub(super) struct LogWheelScrollRequest {
    pub(super) delta_y: Pixels,
    pub(super) row_count: usize,
    pub(super) row_height: Pixels,
    pub(super) line_count: usize,
    pub(super) line_scroll: bool,
    pub(super) scale: f32,
}

/// One scroll owner shared by fixed and wrapped log rows.
///
/// The vertical position is always a logical row plus an offset inside that row. Measured
/// heights improve nearby pixel scrolling only; they are never accumulated into a document-wide
/// height index.
pub(super) struct LogViewportState<K> {
    word_wrap: Cell<bool>,
    viewport: VirtualLogViewport<LogRowKey>,
    text_selections: RefCell<TextSelectionCache<K>>,
    pending_scrollbar_offset: Rc<Cell<Option<Point<Pixels>>>>,
    layout_key: RefCell<Option<WrappedLayoutKey>>,
    row_bounds: Rc<RefCell<BTreeMap<usize, Bounds<Pixels>>>>,
}

impl<K: Clone + Ord> LogViewportState<K> {
    pub(super) fn new(
        word_wrap: bool,
        viewport: VirtualLogViewport<LogRowKey>,
        row_bounds: Rc<RefCell<BTreeMap<usize, Bounds<Pixels>>>>,
    ) -> Self {
        Self {
            word_wrap: Cell::new(word_wrap),
            viewport,
            text_selections: RefCell::default(),
            pending_scrollbar_offset: Rc::default(),
            layout_key: RefCell::default(),
            row_bounds,
        }
    }

    pub(super) fn viewport(&self) -> &VirtualLogViewport<LogRowKey> {
        &self.viewport
    }

    pub(super) fn is_wrapped(&self) -> bool {
        self.word_wrap.get()
    }

    pub(super) fn set_word_wrap(&self, enabled: bool) {
        self.word_wrap.set(enabled);
        self.pending_scrollbar_offset.set(None);
        if enabled {
            self.viewport.set_horizontal_offset(px(0.));
        }
        self.viewport.invalidate_measurements();
        self.text_selections.borrow_mut().clear();
        self.row_bounds.borrow_mut().clear();
    }

    pub(super) fn capture_viewport_position(
        &self,
        row_count: usize,
        preferred_row: Option<usize>,
        row_height: Pixels,
    ) -> Option<RowViewportPosition> {
        if row_count == 0 {
            return None;
        }
        self.viewport.set_item_count(row_count, row_height);
        let position = self.viewport.position();
        let viewport_bounds = self.viewport.viewport_bounds();
        if let Some(row_ix) = preferred_row.filter(|row_ix| {
            self.row_bounds
                .borrow()
                .get(row_ix)
                .is_some_and(|bounds| bounds.intersects(&viewport_bounds))
        }) {
            let viewport_y = self
                .row_bounds
                .borrow()
                .get(&row_ix)
                .map_or(-position.offset_in_row, |bounds| {
                    bounds.top() - viewport_bounds.top()
                });
            Some(RowViewportPosition { row_ix, viewport_y })
        } else {
            Some(RowViewportPosition {
                row_ix: position.row_ix.min(row_count - 1),
                viewport_y: -position.offset_in_row,
            })
        }
    }

    pub(super) fn is_at_end(&self) -> bool {
        self.viewport.is_at_end()
    }

    pub(super) fn restore_viewport(
        &self,
        row_ix: usize,
        viewport_y: Pixels,
        at_end: bool,
        row_height: Pixels,
    ) {
        self.viewport.set_item_count(
            self.viewport.item_count().max(row_ix.saturating_add(1)),
            row_height,
        );
        if at_end {
            self.viewport.scroll_to_end();
        } else {
            self.viewport.scroll_row_to_viewport_y(row_ix, viewport_y);
        }
    }

    pub(super) fn scroll_to_end(&self) {
        VirtualLogListScrollHandle::new(&self.viewport).scroll_to_bottom();
    }

    pub(super) fn center_row(&self, row_ix: usize) {
        VirtualLogListScrollHandle::new(&self.viewport)
            .scroll_to_item(row_ix, ScrollStrategy::Center);
    }

    pub(super) fn first_visible(&self, row_count: usize, row_height: Pixels) -> usize {
        if row_count == 0 {
            0
        } else {
            self.viewport.set_item_count(row_count, row_height);
            self.viewport.position().row_ix.min(row_count - 1)
        }
    }

    pub(super) fn requested_row_range(
        &self,
        row_count: usize,
        viewport_height: Pixels,
        row_height: Pixels,
    ) -> Range<usize> {
        self.viewport.set_item_count(row_count, row_height);
        self.viewport.read_range(viewport_height)
    }

    pub(super) fn row_at_position(&self, position: Point<Pixels>) -> Option<usize> {
        self.row_bounds
            .borrow()
            .iter()
            .find_map(|(row_ix, bounds)| bounds.contains(&position).then_some(*row_ix))
    }

    pub(super) fn visible_row_edge(&self, after: bool) -> Option<usize> {
        let bounds = self.row_bounds.borrow();
        if after {
            bounds.keys().next_back().copied()
        } else {
            bounds.keys().next().copied()
        }
    }

    pub(super) fn place_at_top(&self, row_ix: usize, row_height: Pixels) {
        self.viewport.set_item_count(
            self.viewport.item_count().max(row_ix.saturating_add(1)),
            row_height,
        );
        self.viewport.scroll_to_row(row_ix, ScrollStrategy::Top);
    }

    pub(super) fn page_size(&self, fixed_visible_rows: usize, base_height: Pixels) -> usize {
        let measured = (self.viewport.viewport_bounds().size.height / base_height.max(px(1.)))
            .floor()
            .max(1.) as usize;
        fixed_visible_rows.max(measured).max(1)
    }

    pub(super) fn reveal_row(&self, row_ix: usize, strategy: ScrollStrategy) {
        VirtualLogListScrollHandle::new(&self.viewport).scroll_to_item(row_ix, strategy);
    }

    pub(super) fn wheel_scroll_target(
        &self,
        current: Point<Pixels>,
        request: LogWheelScrollRequest,
    ) -> Option<Point<Pixels>> {
        if request.row_count == 0 || request.delta_y == px(0.) {
            return None;
        }
        self.viewport
            .set_item_count(request.row_count, request.row_height);
        let row_delta = request.line_scroll.then(|| {
            let rows = request.line_count.min(isize::MAX as usize) as isize;
            if request.delta_y < px(0.) {
                rows
            } else {
                -rows
            }
        });
        let pixel_delta = (!request.line_scroll).then_some(-request.delta_y * request.scale);
        self.viewport
            .wheel_scroll_target(current, pixel_delta, row_delta)
    }

    pub(super) fn horizontal_offset(&self) -> Pixels {
        self.viewport.horizontal_offset()
    }

    pub(super) fn set_horizontal_offset(&self, offset: Pixels) {
        self.viewport.set_horizontal_offset(offset);
    }

    pub(super) fn wrapped_base_height(&self) -> Pixels {
        self.viewport.base_height()
    }

    pub(super) fn wrapped_viewport_height(&self) -> Pixels {
        self.viewport.viewport_bounds().size.height
    }

    pub(super) fn committed_viewport_height(&self) -> Pixels {
        self.viewport.viewport_bounds().size.height
    }

    pub(super) fn wrapped_scroll_handle(&self) -> VirtualLogViewport<LogRowKey> {
        self.viewport().clone()
    }

    pub(super) fn wrapped_row_bounds(&self) -> Rc<RefCell<BTreeMap<usize, Bounds<Pixels>>>> {
        self.row_bounds.clone()
    }

    pub(super) fn wrapped_selection(
        &self,
        key: K,
        text: &LogText,
        window: &Window,
        cx: &mut App,
    ) -> LogTextSelection {
        self.text_selections
            .borrow_mut()
            .handle(key, text, window, cx)
    }

    pub(super) fn wrapped_sizes(&self, count: usize, base_height: Pixels) {
        self.viewport.set_item_count(count, base_height);
    }

    pub(super) fn wrapped_logical_scroll_handle(
        &self,
        item_count: usize,
        slot_height: Pixels,
    ) -> AtomicVirtualLogScrollHandle {
        self.viewport.set_item_count(item_count, slot_height);
        AtomicVirtualLogScrollHandle {
            viewport: self.viewport.clone(),
            pending_offset: self.pending_scrollbar_offset.clone(),
        }
    }

    pub(super) fn take_pending_scrollbar_offset(&self) -> Option<Point<Pixels>> {
        self.pending_scrollbar_offset.take()
    }

    pub(super) fn committed_scroll_offset(&self) -> Point<Pixels> {
        self.viewport.offset()
    }

    pub(super) fn viewport_offset_for_target(
        &self,
        target: LogScrollFrameTarget,
        item_count: usize,
        slot_height: Pixels,
    ) -> Point<Pixels> {
        self.viewport.set_item_count(item_count, slot_height);
        target.offset()
    }

    pub(super) fn commit_scroll_frame_target(
        &self,
        target: LogScrollFrameTarget,
        item_count: usize,
        slot_height: Pixels,
    ) {
        self.viewport.set_item_count(item_count, slot_height);
        self.viewport.set_offset(target.offset());
        self.viewport.publish_native_scroll();
    }

    pub(super) fn effective_row_height(&self, row_ix: usize, base_height: Pixels) -> Pixels {
        self.viewport.row_height(row_ix).max(base_height)
    }

    #[cfg(test)]
    pub(super) fn has_known_wrapped_row_height(&self, row_ix: usize) -> bool {
        self.viewport.has_measured_height(row_ix)
    }

    pub(super) fn prime_wrapped_measured_heights(
        &self,
        count: usize,
        base_height: Pixels,
        heights: impl IntoIterator<Item = (usize, Pixels)>,
    ) {
        self.viewport.set_item_count(count, base_height);
        for (row_ix, height) in heights {
            self.viewport.record_indexed_height(row_ix, height);
        }
    }

    pub(super) fn wrapped_measured_heights_by_key(
        &self,
        _key_for_row: impl Fn(usize) -> Option<LogRowKey>,
    ) -> BTreeMap<LogRowKey, Pixels> {
        self.viewport.measured_heights()
    }

    pub(super) fn reset_wrapped_with_remapped_heights(
        &mut self,
        count: usize,
        base_height: Pixels,
        measured_heights: BTreeMap<LogRowKey, Pixels>,
        row_for_key: impl Fn(&LogRowKey) -> Option<usize>,
    ) {
        let anchor_key = self.viewport.anchor_key();
        self.viewport.invalidate_measurements();
        self.viewport.set_item_count(count, base_height);
        for (key, height) in measured_heights {
            if let Some(row_ix) = row_for_key(&key) {
                self.viewport.record_measured_height(row_ix, key, height);
            }
        }
        if let Some(anchor_key) = anchor_key {
            self.viewport
                .restore_anchor_by_key(&anchor_key, row_for_key);
        }
    }

    pub(super) fn invalidate_wrapped(&mut self) {
        self.viewport.invalidate_measurements();
        self.text_selections.borrow_mut().clear();
        self.row_bounds.borrow_mut().clear();
    }

    pub(super) fn capture_wrapped_viewport_position(
        &self,
        preferred_row: Option<usize>,
    ) -> Option<RowViewportPosition> {
        self.capture_viewport_position(
            self.viewport.item_count(),
            preferred_row,
            self.viewport.base_height(),
        )
    }

    pub(super) fn ensure_wrapped_measurement_anchor(&self, preferred_row: Option<usize>) {
        let Some(position) = self.capture_wrapped_viewport_position(preferred_row) else {
            return;
        };
        if !self.is_at_end() || preferred_row == Some(position.row_ix) {
            self.viewport
                .preserve_row_at_viewport_y(position.row_ix, position.viewport_y);
        }
    }

    pub(super) fn invalidate_wrapped_layout_preserving_position(
        &self,
        key: WrappedLayoutKey,
        preferred_row: Option<usize>,
    ) -> bool {
        if key.width <= px(0.)
            || self
                .layout_key
                .borrow()
                .as_ref()
                .is_some_and(|current| current.is_equivalent_to(&key))
        {
            return false;
        }
        self.ensure_wrapped_measurement_anchor(preferred_row);
        self.layout_key.replace(Some(key));
        self.viewport.invalidate_measurements();
        self.text_selections.borrow_mut().clear();
        self.row_bounds.borrow_mut().clear();
        true
    }

    pub(super) fn begin_row_layout(&self) {
        // Only rows prepainted in this frame may participate in hit testing and anchoring.
        self.row_bounds.borrow_mut().clear();
    }
}

#[derive(Clone, Debug)]
pub(super) struct WrappedLayoutKey {
    pub(super) content_revision: u64,
    pub(super) width: Pixels,
    pub(super) rem_size: Pixels,
    pub(super) font_family: SharedString,
    pub(super) font_size: u16,
    pub(super) base_height: Pixels,
    pub(super) horizontal_padding: Pixels,
}

impl WrappedLayoutKey {
    pub(super) fn is_equivalent_to(&self, other: &Self) -> bool {
        self.content_revision == other.content_revision
            && (self.width - other.width).abs() < px(0.5)
            && self.rem_size == other.rem_size
            && self.font_family == other.font_family
            && self.font_size == other.font_size
            && self.base_height == other.base_height
            && self.horizontal_padding == other.horizontal_padding
    }
}

#[derive(Clone)]
pub(super) struct AtomicVirtualLogScrollHandle {
    viewport: VirtualLogViewport<LogRowKey>,
    pending_offset: Rc<Cell<Option<Point<Pixels>>>>,
}

impl ScrollbarHandle for AtomicVirtualLogScrollHandle {
    fn viewport_bounds(&self) -> Bounds<Pixels> {
        self.viewport.viewport_bounds()
    }

    fn offset(&self) -> Point<Pixels> {
        self.viewport.offset()
    }

    fn set_offset(&self, offset: Point<Pixels>) {
        let current = self.viewport.offset();
        if (current.x - offset.x).abs() >= px(0.5) || (current.y - offset.y).abs() >= px(0.5) {
            self.pending_offset.set(Some(offset));
        }
    }

    fn content_size(&self) -> Size<Pixels> {
        self.viewport.content_size()
    }
}
