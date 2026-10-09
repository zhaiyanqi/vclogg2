//! Presentation-layer entrance without resizing or reflowing the settings UI.
use gpui_kit::Window;
use objc2::rc::Retained;
use objc2_app_kit::{NSColor, NSView, NSWindow, NSWindowAnimationBehavior};
use objc2_foundation::{NSNumber, NSValue, ns_string};
use objc2_quartz_core::{
    CABasicAnimation, CALayer, CAMediaTiming, CAMediaTimingFunction, NSValueCATransform3DAdditions,
};
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

pub(super) fn prepare(window: &Window, opacity: f32) {
    if let Some(native) = native_window(window) {
        native.setAnimationBehavior(NSWindowAnimationBehavior::None);
        native.setAlphaValue(opacity.into());
    }
}

/// Restores native presentation even if the settings window closes mid-animation.
/// Owned and dropped exclusively by the GPUI foreground task.
pub(super) struct Entrance {
    native: Retained<NSWindow>,
    layer: Retained<CALayer>,
    background: Retained<NSColor>,
    opaque: bool,
}

impl Drop for Entrance {
    fn drop(&mut self) {
        self.layer
            .removeAnimationForKey(ns_string!("vclogg-settings-scale"));
        self.layer
            .removeAnimationForKey(ns_string!("vclogg-settings-fade"));
        self.native.setBackgroundColor(Some(&self.background));
        self.native.setOpaque(self.opaque);
    }
}

pub(super) fn animate(window: &Window) -> Option<Entrance> {
    let native = native_window(window)?;
    // Never leave an invisible window if the platform cannot provide a layer.
    native.setAlphaValue(1.);
    let content = native.contentView()?;
    content.setWantsLayer(true);
    let layer = content.layer()?;
    let entrance = Entrance {
        background: native.backgroundColor(),
        opaque: native.isOpaque(),
        native,
        layer,
    };
    let layer = &entrance.layer;
    let final_transform = layer.transform();
    let mut initial_transform = final_transform.scale(super::ENTER_SCALE, super::ENTER_SCALE, 1.);
    // AppKit layers need not use a centered anchor. Compensate without changing
    // their model geometry, so window controls and text scale around the center.
    let bounds = layer.bounds();
    let anchor = layer.anchorPoint();
    initial_transform.m41 += (1. - super::ENTER_SCALE) * (0.5 - anchor.x) * bounds.size.width;
    initial_transform.m42 += (1. - super::ENTER_SCALE) * (0.5 - anchor.y) * bounds.size.height;
    let easing = CAMediaTimingFunction::functionWithControlPoints(0.22, 1., 0.36, 1.);
    let scale = CABasicAnimation::animationWithKeyPath(Some(ns_string!("transform")));
    // SAFETY: The transform key path requires NSValue-wrapped CATransform3D values.
    unsafe {
        scale.setFromValue(Some(&NSValue::valueWithCATransform3D(initial_transform)));
        scale.setToValue(Some(&NSValue::valueWithCATransform3D(final_transform)));
    }
    scale.setDuration(super::ENTER_SCALE_DURATION.as_secs_f64());
    scale.setTimingFunction(Some(&easing));
    let fade = CABasicAnimation::animationWithKeyPath(Some(ns_string!("opacity")));
    // SAFETY: The opacity key path requires numeric values.
    unsafe {
        fade.setFromValue(Some(&NSNumber::new_f32(0.)));
        fade.setToValue(Some(&NSNumber::new_f32(layer.opacity())));
    }
    fade.setDuration(super::ENTER_FADE_DURATION.as_secs_f64());
    fade.setTimingFunction(Some(&easing));
    entrance.native.setOpaque(false);
    entrance
        .native
        .setBackgroundColor(Some(&NSColor::clearColor()));
    layer.addAnimation_forKey(&scale, Some(ns_string!("vclogg-settings-scale")));
    layer.addAnimation_forKey(&fade, Some(ns_string!("vclogg-settings-fade")));
    Some(entrance)
}
