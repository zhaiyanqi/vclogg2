use std::{cell::Cell, rc::Rc};

use gpui_kit::{
    AnyElement, App, AvailableSpace, Bounds, Context, Div, Element, ElementId, GlobalElementId,
    InspectorElementId, InteractiveElement as _, IntoElement, LayoutId, ParentElement as _, Pixels,
    Point, Render, Stateful, Styled as _, Window, div, point, px, size,
};

#[derive(Clone, Copy)]
pub(crate) struct TagGeometry {
    pub(crate) content: Bounds<Pixels>,
    pub(crate) tag: Bounds<Pixels>,
    visible: Bounds<Pixels>,
    vertical_center: bool,
}

impl TagGeometry {
    pub(crate) fn drag_position(
        self,
        pointer: Point<Pixels>,
        grab_offset: Point<Pixels>,
    ) -> Point<Pixels> {
        self.constrain_position(pointer - self.content.origin - grab_offset)
    }

    fn constrain_position(self, position: Point<Pixels>) -> Point<Pixels> {
        let vertical_space = (self.content.size.height - self.tag.size.height).max(px(0.));
        let left = (self.visible.left() - self.content.left()).max(px(0.));
        let right = (self.visible.right() - self.content.left() - self.tag.size.width).max(left);
        point(
            position.x.clamp(left, right),
            if self.vertical_center {
                vertical_space / 2.
            } else {
                position.y.clamp(px(0.), vertical_space)
            },
        )
    }
}

pub(crate) struct TagDragPreview;

impl Render for TagDragPreview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        // The original tag moves in its row; no detached duplicate follows the pointer.
        div()
    }
}

pub(crate) type TagGeometryHandle = Rc<Cell<Option<TagGeometry>>>;

pub(crate) struct PositionedTag {
    pub(crate) position: Point<Pixels>,
    pub(crate) vertical_center: bool,
    pub(crate) geometry: TagGeometryHandle,
    pub(crate) element: Option<AnyElement>,
}

/// Out-of-flow row annotations. Layout is resolved here so clamping uses this
/// frame's actual wrapped row bounds and measured tag dimensions.
pub(crate) struct LogTagLayer {
    id: ElementId,
    base: Stateful<Div>,
    tags: Vec<PositionedTag>,
}

impl LogTagLayer {
    pub(crate) fn new(id: impl Into<ElementId>, tags: Vec<PositionedTag>) -> Self {
        let id = id.into();
        Self {
            base: div().id(id.clone()).absolute().inset_0(),
            id,
            tags,
        }
    }
}

impl IntoElement for LogTagLayer {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for LogTagLayer {
    type RequestLayoutState = Vec<AnyElement>;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let layout = self.base.interactivity().request_layout(
            id,
            inspector,
            window,
            cx,
            |style, window, cx| window.request_layout(style, None, cx),
        );
        (layout, Vec::new())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        elements: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) {
        // Horizontal scrolling can place the content bounds outside the viewport.
        // Keep coordinates relative to the content so dragging and persistence agree.
        let mask = window.content_mask().bounds;
        let left = bounds.left().max(mask.left());
        let right = bounds.right().min(mask.right()).max(left);
        let visible = Bounds::new(
            point(left, bounds.top()),
            size(right - left, bounds.size.height),
        );
        // The layer has no hitbox: empty space belongs to SelectableLogText.
        for tag in &mut self.tags {
            let Some(content) = tag.element.take() else {
                continue;
            };
            let mut element = div()
                .max_w(visible.size.width)
                .max_h(bounds.size.height)
                .overflow_hidden()
                .child(content)
                .into_any_element();
            let measured = element.layout_as_root(
                size(AvailableSpace::MaxContent, AvailableSpace::MinContent),
                window,
                cx,
            );
            let mut geometry = TagGeometry {
                content: bounds,
                visible,
                tag: Bounds::new(bounds.origin, measured),
                vertical_center: tag.vertical_center,
            };
            // Append placement is chosen when a mark is created. Rendering only
            // clamps to the viewport, so saved and dragged positions can overlap.
            geometry.tag.origin = bounds.origin + geometry.constrain_position(tag.position);
            tag.geometry.set(Some(geometry));
            element.prepaint_at(geometry.tag.origin, window, cx);
            elements.push(element);
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        elements: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        for element in elements {
            element.paint(window, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TagGeometry;
    use gpui_kit::{Bounds, point, px, size};

    #[test]
    fn line_end_placement_keeps_the_whole_tag_in_the_visible_row() {
        let geometry = TagGeometry {
            content: Bounds::new(point(px(-200.), px(0.)), size(px(2000.), px(24.))),
            visible: Bounds::new(point(px(100.), px(0.)), size(px(500.), px(24.))),
            tag: Bounds::new(point(px(0.), px(0.)), size(px(80.), px(20.))),
            vertical_center: true,
        };
        // A visible line end remains the anchor, including after horizontal scrolling.
        assert_eq!(
            geometry.constrain_position(point(px(400.), px(0.))),
            point(px(400.), px(2.))
        );
        // An offscreen line end leaves room for the complete tag at the right edge.
        assert_eq!(
            geometry.constrain_position(point(px(1800.), px(0.))),
            point(px(720.), px(2.))
        );
        // A line end scrolled past the left edge remains reachable.
        assert_eq!(
            geometry.constrain_position(point(px(50.), px(0.))),
            point(px(300.), px(2.))
        );
    }

    #[gpui_kit::test]
    fn rendered_marks_preserve_overlapping_positions(cx: &mut gpui_kit::TestAppContext) {
        use super::{LogTagLayer, PositionedTag, TagGeometryHandle};
        use gpui_kit::test::TestWindowExt as _;
        use gpui_kit::{
            AppContext as _, Context, IntoElement, ParentElement as _, Render, Styled as _, Window,
            div,
        };

        struct Marks([TagGeometryHandle; 2]);
        impl Render for Marks {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div()
                    .relative()
                    .w(px(500.))
                    .h(px(24.))
                    .child(LogTagLayer::new(
                        "overlapping-marks",
                        self.0
                            .iter()
                            .map(|geometry| PositionedTag {
                                position: point(px(200.), px(0.)),
                                vertical_center: true,
                                geometry: geometry.clone(),
                                element: Some(div().w(px(80.)).h(px(20.)).into_any_element()),
                            })
                            .collect(),
                    ))
            }
        }
        cx.update(gpui_kit::init);
        let geometry: [TagGeometryHandle; 2] = Default::default();
        let handle = cx.open_window(size(px(600.), px(100.)), |_, _| Marks(geometry.clone()));
        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            let first = geometry[0].get().unwrap();
            let second = geometry[1].get().unwrap();
            assert_eq!(first.tag.origin, second.tag.origin);
            assert_eq!(first.tag.origin.x - first.content.origin.x, px(200.));
        })
        .unwrap();
    }
}
