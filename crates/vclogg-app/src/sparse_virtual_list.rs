use std::{
    cell::RefCell,
    collections::BTreeMap,
    ops::{Deref, Range},
    rc::Rc,
};

use gpui::{
    AnyElement, App, AvailableSpace, Bounds, ContentMask, Context, DeferredScrollToItem, Div,
    Element, ElementId, Entity, GlobalElementId, Hitbox, InteractiveElement as _, IntoElement,
    Pixels, Point, Render, ScrollHandle, ScrollStrategy, Size, Stateful,
    StatefulInteractiveElement as _, StyleRefinement, Styled, Window, div, point, px, size,
};
use gpui_component::InteractiveElementExt as _;

/// Sparse row heights shared by the wrapped-list element and its scroll presenters.
#[derive(Clone)]
pub(crate) struct SparseListMeasurements {
    pub(crate) item_count: usize,
    pub(crate) base_height: Pixels,
    pub(crate) measured_heights: Rc<RefCell<BTreeMap<usize, Pixels>>>,
    pub(crate) cumulative_corrections: Rc<RefCell<Vec<(usize, Pixels)>>>,
}

impl SparseListMeasurements {
    pub(crate) fn item_height(&self, row_ix: usize) -> Pixels {
        self.measured_heights
            .borrow()
            .get(&row_ix)
            .copied()
            .unwrap_or(self.base_height)
            .max(self.base_height)
    }

    pub(crate) fn prefix_height(&self, row_ix: usize) -> Pixels {
        prefix_height_for(
            self.base_height,
            &self.cumulative_corrections.borrow(),
            row_ix,
        )
    }

    pub(crate) fn row_for_y(&self, target: Pixels) -> usize {
        row_for_absolute_y(
            self.item_count,
            self.base_height,
            &self.cumulative_corrections.borrow(),
            target,
        )
    }
}

struct SparseVirtualListScrollState {
    deferred_scroll_to_item: Option<DeferredScrollToItem>,
}

/// Scroll handle for [`SparseVirtualList`].
#[derive(Clone)]
pub(crate) struct SparseVirtualListScrollHandle {
    state: Rc<RefCell<SparseVirtualListScrollState>>,
    base_handle: ScrollHandle,
}

impl Default for SparseVirtualListScrollHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl SparseVirtualListScrollHandle {
    pub(crate) fn new() -> Self {
        Self {
            state: Rc::new(RefCell::new(SparseVirtualListScrollState {
                deferred_scroll_to_item: None,
            })),
            base_handle: ScrollHandle::new(),
        }
    }

    pub(crate) fn base_handle(&self) -> &ScrollHandle {
        &self.base_handle
    }

    pub(crate) fn scroll_to_item(&self, item_index: usize, strategy: ScrollStrategy) {
        self.state.borrow_mut().deferred_scroll_to_item = Some(DeferredScrollToItem {
            item_index,
            strategy,
            offset: 0,
            scroll_strict: false,
        });
    }

    pub(crate) fn scroll_to_bottom(&self) {
        self.scroll_to_item(usize::MAX, ScrollStrategy::Bottom);
    }
}

impl Deref for SparseVirtualListScrollHandle {
    type Target = ScrollHandle;

    fn deref(&self) -> &Self::Target {
        &self.base_handle
    }
}

impl gpui_base::ScrollbarHandle for SparseVirtualListScrollHandle {
    fn viewport_bounds(&self) -> Bounds<Pixels> {
        self.base_handle.bounds()
    }

    fn offset(&self) -> Point<Pixels> {
        self.base_handle.offset()
    }

    fn set_offset(&self, offset: Point<Pixels>) {
        self.base_handle.set_offset(offset);
    }

    fn content_size(&self) -> Size<Pixels> {
        self.base_handle.content_size()
    }
}

/// Build a vertical virtual list whose variable heights are stored only for measured rows.
pub(crate) fn sparse_v_virtual_list<R, V>(
    view: Entity<V>,
    id: impl Into<ElementId>,
    measurements: SparseListMeasurements,
    render: impl 'static + Fn(&mut V, Range<usize>, &mut Window, &mut Context<V>) -> Vec<R>,
) -> SparseVirtualList
where
    R: IntoElement,
    V: Render,
{
    let id = id.into();
    let render_items = move |range: Range<usize>, window: &mut Window, cx: &mut App| {
        view.update(cx, |view, cx| {
            render(view, range, window, cx)
                .into_iter()
                .map(IntoElement::into_any_element)
                .collect()
        })
    };
    let scroll_handle = SparseVirtualListScrollHandle::new();
    SparseVirtualList {
        id: id.clone(),
        base: div()
            .id(id)
            .size_full()
            .overflow_scroll()
            .lock_scroll_axis()
            .track_scroll(scroll_handle.base_handle()),
        scroll_handle,
        measurements,
        render_items: Box::new(render_items),
    }
}

pub(crate) struct SparseVirtualList {
    id: ElementId,
    base: Stateful<Div>,
    scroll_handle: SparseVirtualListScrollHandle,
    measurements: SparseListMeasurements,
    render_items: Box<RenderItems>,
}

type RenderItems = dyn for<'a> Fn(Range<usize>, &'a mut Window, &'a mut App) -> Vec<AnyElement>;

impl SparseVirtualList {
    pub(crate) fn track_scroll(mut self, handle: &SparseVirtualListScrollHandle) -> Self {
        self.base = self.base.track_scroll(handle.base_handle());
        self.scroll_handle = handle.clone();
        self
    }

    fn apply_deferred_scroll(&self, viewport_height: Pixels, deferred: DeferredScrollToItem) {
        if self.measurements.item_count == 0 {
            return;
        }
        let row_ix = deferred
            .item_index
            .saturating_add(deferred.offset)
            .min(self.measurements.item_count - 1);
        let row_top = self.measurements.prefix_height(row_ix);
        let row_height = self.measurements.item_height(row_ix);
        let row_bottom = row_top + row_height;
        let current_top = (-self.scroll_handle.offset().y).max(px(0.));
        let is_above = row_top < current_top;
        let is_below = row_bottom > current_top + viewport_height;
        if !deferred.scroll_strict && !is_above && !is_below {
            return;
        }
        let strategy = if deferred.strategy == ScrollStrategy::Nearest {
            if is_above {
                ScrollStrategy::Top
            } else if is_below {
                ScrollStrategy::Bottom
            } else {
                return;
            }
        } else {
            deferred.strategy
        };
        let content_height = self
            .measurements
            .prefix_height(self.measurements.item_count);
        let max_top = (content_height - viewport_height).max(px(0.));
        let top = match strategy {
            ScrollStrategy::Top => row_top,
            ScrollStrategy::Center => row_top + row_height / 2. - viewport_height / 2.,
            ScrollStrategy::Bottom => row_bottom - viewport_height,
            ScrollStrategy::Nearest => current_top,
        }
        .clamp(px(0.), max_top);
        self.scroll_handle
            .set_offset(point(self.scroll_handle.offset().x, -top));
    }
}

impl Styled for SparseVirtualList {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

pub(crate) struct SparseVirtualListFrameState {
    items: Vec<AnyElement>,
}

impl Element for SparseVirtualList {
    type RequestLayoutState = SparseVirtualListFrameState;
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
        (layout_id, SparseVirtualListFrameState { items: Vec::new() })
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
        let content_height = self
            .measurements
            .prefix_height(self.measurements.item_count);
        let content_size = size(content_bounds.size.width, content_height);

        let deferred = {
            let mut state = self.scroll_handle.state.borrow_mut();
            state.deferred_scroll_to_item.take()
        };
        if let Some(deferred) = deferred {
            self.apply_deferred_scroll(content_bounds.size.height, deferred);
        }

        let max_top = (content_height - content_bounds.size.height).max(px(0.));
        let mut scroll_offset = self.scroll_handle.offset();
        scroll_offset.y = scroll_offset.y.clamp(-max_top, px(0.));
        scroll_offset.x = px(0.);
        if scroll_offset != self.scroll_handle.offset() {
            self.scroll_handle.set_offset(scroll_offset);
        }

        self.base.interactivity().prepaint(
            global_id,
            inspector_id,
            bounds,
            content_size,
            window,
            cx,
            |_style, _, hitbox, window, cx| {
                if self.measurements.item_count == 0 {
                    return hitbox;
                }
                let top = (-scroll_offset.y).max(px(0.));
                let first = self.measurements.row_for_y(top);
                let last = self
                    .measurements
                    .row_for_y(top + content_bounds.size.height)
                    .saturating_add(2)
                    .min(self.measurements.item_count);
                let visible_range = first..last.max(first.saturating_add(1));
                let items = (self.render_items)(visible_range.clone(), window, cx);
                window.with_content_mask(Some(ContentMask { bounds }), |window| {
                    for (mut item, row_ix) in items.into_iter().zip(visible_range) {
                        let origin = content_bounds.origin
                            + point(
                                px(0.),
                                self.measurements.prefix_height(row_ix) + scroll_offset.y,
                            );
                        let available_space = size(
                            AvailableSpace::Definite(content_bounds.size.width),
                            AvailableSpace::Definite(self.measurements.item_height(row_ix)),
                        );
                        item.layout_as_root(available_space, window, cx);
                        item.prepaint_at(origin, window, cx);
                        frame.items.push(item);
                    }
                });
                hitbox
            },
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
                for item in &mut frame.items {
                    item.paint(window, cx);
                }
            },
        )
    }
}

impl IntoElement for SparseVirtualList {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

pub(crate) fn prefix_height_for(
    base_height: Pixels,
    corrections: &[(usize, Pixels)],
    row_ix: usize,
) -> Pixels {
    let base = base_height * row_ix as f32;
    let correction_ix = corrections.partition_point(|(measured_row, _)| *measured_row < row_ix);
    base + correction_ix
        .checked_sub(1)
        .and_then(|ix| corrections.get(ix).map(|(_, correction)| *correction))
        .unwrap_or(px(0.))
}

pub(crate) fn row_for_absolute_y(
    count: usize,
    base_height: Pixels,
    corrections: &[(usize, Pixels)],
    target: Pixels,
) -> usize {
    let mut low = 0usize;
    let mut high = count;
    while low < high {
        let middle = low + (high - low) / 2;
        if prefix_height_for(base_height, corrections, middle.saturating_add(1)) > target {
            high = middle;
        } else {
            low = middle.saturating_add(1);
        }
    }
    low.min(count.saturating_sub(1))
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, collections::BTreeMap, ops::Range, rc::Rc};

    use gpui::{
        Context, IntoElement, Render, ScrollStrategy, Styled as _, TestAppContext, Window, div, px,
    };

    use super::{SparseListMeasurements, SparseVirtualListScrollHandle, sparse_v_virtual_list};

    struct Harness {
        measurements: SparseListMeasurements,
        scroll_handle: SparseVirtualListScrollHandle,
        visible_ranges: Rc<RefCell<Vec<Range<usize>>>>,
    }

    impl Render for Harness {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let visible_ranges = self.visible_ranges.clone();
            sparse_v_virtual_list(
                cx.entity(),
                "sparse-list-test",
                self.measurements.clone(),
                move |_, range, _, _| {
                    visible_ranges.borrow_mut().push(range.clone());
                    range.map(|_| div()).collect::<Vec<_>>()
                },
            )
            .track_scroll(&self.scroll_handle)
            .w(px(80.))
            .h(px(60.))
        }
    }

    #[gpui::test]
    fn renders_only_visible_rows_and_scrolls_to_distant_rows(cx: &mut TestAppContext) {
        let scroll_handle = SparseVirtualListScrollHandle::new();
        let visible_ranges = Rc::new(RefCell::new(Vec::new()));
        let measurements = SparseListMeasurements {
            item_count: 1_000_000,
            base_height: px(20.),
            measured_heights: Rc::new(RefCell::new(BTreeMap::new())),
            cumulative_corrections: Rc::new(RefCell::new(Vec::new())),
        };
        let (_, cx) = cx.add_window_view({
            let scroll_handle = scroll_handle.clone();
            let visible_ranges = visible_ranges.clone();
            move |_, _| Harness {
                measurements,
                scroll_handle,
                visible_ranges,
            }
        });

        cx.update(|window, cx| window.draw(cx).clear(cx));
        let initial = visible_ranges.borrow().last().cloned().unwrap();
        assert_eq!(initial.start, 0);
        assert!(initial.end < 10);

        scroll_handle.scroll_to_item(800_000, ScrollStrategy::Top);
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let scrolled = visible_ranges.borrow().last().cloned().unwrap();
        assert!(scrolled.contains(&800_000));
        assert!(scrolled.len() < 10);
        assert!(scroll_handle.offset().y < px(0.));
    }

    #[gpui::test]
    fn scroll_to_bottom_can_be_requested_before_first_layout(cx: &mut TestAppContext) {
        let scroll_handle = SparseVirtualListScrollHandle::new();
        scroll_handle.scroll_to_bottom();
        let visible_ranges = Rc::new(RefCell::new(Vec::new()));
        let measurements = SparseListMeasurements {
            item_count: 100,
            base_height: px(20.),
            measured_heights: Rc::new(RefCell::new(BTreeMap::new())),
            cumulative_corrections: Rc::new(RefCell::new(Vec::new())),
        };
        let (_, cx) = cx.add_window_view({
            let scroll_handle = scroll_handle.clone();
            let visible_ranges = visible_ranges.clone();
            move |_, _| Harness {
                measurements,
                scroll_handle,
                visible_ranges,
            }
        });

        cx.update(|window, cx| window.draw(cx).clear(cx));

        assert!(visible_ranges.borrow().last().unwrap().contains(&99));
    }
}
