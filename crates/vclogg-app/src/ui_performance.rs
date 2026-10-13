pub(crate) mod trace;

use gpui_kit::{
    AnyElement, App, Bounds, Element, ElementId, GlobalElementId, InspectorElementId, IntoElement,
    LayoutId, Pixels, Window,
};

pub(crate) struct UiPerformanceElement {
    child: AnyElement,
    request_layout_name: &'static str,
    prepaint_name: &'static str,
    paint_name: &'static str,
}

pub(crate) fn element(
    request_layout_name: &'static str,
    prepaint_name: &'static str,
    paint_name: &'static str,
    child: impl IntoElement,
) -> AnyElement {
    if !trace::is_recording() {
        return child.into_any_element();
    }
    UiPerformanceElement {
        child: child.into_any_element(),
        request_layout_name,
        prepaint_name,
        paint_name,
    }
    .into_any_element()
}

impl IntoElement for UiPerformanceElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for UiPerformanceElement {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let _performance_scope = scope(self.request_layout_name);
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let _performance_scope = scope(self.prepaint_name);
        self.child.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let _performance_scope = scope(self.paint_name);
        self.child.paint(window, cx);
    }
}

struct OverlayPreference(bool);
impl gpui_kit::Global for OverlayPreference {}

/// Keep in-session changes ahead of late settings loads from other windows.
pub(crate) fn restore_overlay(
    saved: bool,
    window: &mut gpui_kit::Window,
    cx: &mut gpui_kit::App,
) -> bool {
    let enabled = cx
        .try_global::<OverlayPreference>()
        .map_or(saved, |value| value.0);
    cx.set_global(OverlayPreference(enabled));
    apply_overlay(enabled, window);
    enabled
}

/// Use GPUI's scene overlay without introducing a redraw timer or view invalidations.
pub(crate) fn apply_overlay(enabled: bool, window: &mut gpui_kit::Window) {
    let mode = if enabled {
        gpui_kit::DebugFrameOverlayMode::Full
    } else {
        gpui_kit::DebugFrameOverlayMode::Hidden
    };
    if window.debug_frame_overlay_mode() != mode {
        window.set_debug_frame_overlay_mode(mode);
    }
}

pub(crate) fn sync_overlay(enabled: bool, window: &mut gpui_kit::Window, cx: &mut gpui_kit::App) {
    cx.set_global(OverlayPreference(enabled));
    apply_overlay(enabled, window);
    let current = window.window_handle();
    for handle in cx.windows().into_iter().filter(|handle| *handle != current) {
        // Touch only the Window so settings changes cannot re-enter Workspace entities.
        _ = handle.update(cx, |_, window, _| apply_overlay(enabled, window));
    }
}

pub(crate) use trace::scope;
