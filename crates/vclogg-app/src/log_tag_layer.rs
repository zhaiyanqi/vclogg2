use std::{cell::Cell, rc::Rc};

use gpui::{
    AnyElement, App, AvailableSpace, Bounds, Div, Element, ElementId, GlobalElementId,
    InspectorElementId, InteractiveElement as _, IntoElement, LayoutId, ParentElement as _, Pixels,
    Point, Stateful, Styled as _, Window, div, point, px, size,
};

#[derive(Clone, Copy)]
pub(crate) struct TagGeometry {
    pub(crate) content: Bounds<Pixels>,
    pub(crate) tag: Bounds<Pixels>,
}

pub(crate) type TagGeometryHandle = Rc<Cell<Option<TagGeometry>>>;

pub(crate) struct PositionedTag {
    pub(crate) position: Point<Pixels>,
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
        // The layer has no hitbox: empty space belongs to SelectableLogText.
        for tag in &mut self.tags {
            let Some(content) = tag.element.take() else {
                continue;
            };
            let mut element = div()
                .max_w(bounds.size.width)
                .max_h(bounds.size.height)
                .overflow_hidden()
                .child(content)
                .into_any_element();
            let measured = element.layout_as_root(
                size(AvailableSpace::MaxContent, AvailableSpace::MinContent),
                window,
                cx,
            );
            let offset = point(
                tag.position
                    .x
                    .clamp(px(0.), (bounds.size.width - measured.width).max(px(0.))),
                tag.position
                    .y
                    .clamp(px(0.), (bounds.size.height - measured.height).max(px(0.))),
            );
            let tag_bounds = Bounds::new(bounds.origin + offset, measured);
            tag.geometry.set(Some(TagGeometry {
                content: bounds,
                tag: tag_bounds,
            }));
            element.prepaint_at(tag_bounds.origin, window, cx);
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
