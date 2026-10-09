//! Disable AppKit's extra window entrance; GPUI owns the shared presentation.
use gpui_kit::Window;
use objc2::rc::Retained;
use objc2_app_kit::{NSView, NSWindow, NSWindowAnimationBehavior};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

fn native_window(window: &Window) -> Option<Retained<NSWindow>> {
    let RawWindowHandle::AppKit(handle) = HasWindowHandle::window_handle(window).ok()?.as_raw()
    else {
        return None;
    };
    // SAFETY: GPUI supplies a live NSView; all callers run on the main thread.
    let view = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
    view.window()
}

pub(super) fn prepare(window: &Window) {
    if let Some(native) = native_window(window) {
        native.setAnimationBehavior(NSWindowAnimationBehavior::None);
    }
}
