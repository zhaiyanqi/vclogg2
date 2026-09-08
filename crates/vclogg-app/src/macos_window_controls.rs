//! Persistent AppKit window buttons, independent of the auto-hiding titlebar.
#![deny(deprecated)]

use std::cell::{Cell, RefCell};

use anyhow::{Context as _, Result};
use gpui::{Pixels, Point, Window, WindowOptions};
use objc2::{
    AnyThread, DefinedClass, MainThreadOnly, define_class, msg_send,
    rc::{Retained, Weak},
    runtime::AnyObject,
    sel,
};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSButton, NSEvent, NSEventModifierFlags, NSTrackingArea,
    NSTrackingAreaOptions, NSView, NSWindow, NSWindowButton, NSWindowDidBecomeKeyNotification,
    NSWindowDidChangeBackingPropertiesNotification, NSWindowDidDeminiaturizeNotification,
    NSWindowDidEndSheetNotification, NSWindowDidEnterFullScreenNotification,
    NSWindowDidExitFullScreenNotification, NSWindowDidResignKeyNotification,
    NSWindowDidResizeNotification, NSWindowDidUpdateNotification, NSWindowOrderingMode,
    NSWindowStyleMask, NSWindowWillBeginSheetNotification, NSWindowWillCloseNotification,
};
use objc2_foundation::{
    NSNotification, NSNotificationCenter, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString,
};
use raw_window_handle::RawWindowHandle;

struct ControlsState {
    window: Weak<NSWindow>,
    buttons: [Retained<NSButton>; 3],
    position: NSPoint,
    gap: f64,
    syncing: Cell<bool>,
    hovered: Cell<bool>,
    tracking_area: RefCell<Option<Retained<NSTrackingArea>>>,
    tooltip_state: Cell<Option<(crate::i18n::Language, bool, bool)>>,
}

define_class!(
    // SAFETY: NSView is subclassed only on the AppKit main thread. Native
    // ownership retains this view; the window and button targets are weak.
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[ivars = ControlsState]
    #[name = "VCLoggWindowControls"]
    struct WindowControls;

    unsafe impl NSObjectProtocol for WindowControls {}

    impl WindowControls {
        // AppKit's standard window buttons ask their parent for group rollover
        // state before drawing native glyphs. This is an undocumented AppKit
        // callback, isolated to our own class: no system method is swizzled.
        // NSTrackingArea alone cannot make those glyphs appear. If AppKit stops
        // using this callback, tracking/clicks still work but glyphs need review.
        #[unsafe(method(_mouseInGroup:))]
        fn mouse_in_group(&self, button: &NSButton) -> bool {
            self.ivars().hovered.get() && button.isEnabled()
        }

        #[unsafe(method(mouseEntered:))]
        fn mouse_entered(&self, _event: &NSEvent) {
            self.update_hover_from_pointer();
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _event: &NSEvent) {
            self.set_hovered(false);
        }

        #[unsafe(method(updateTrackingAreas))]
        fn update_tracking_areas(&self) {
            // SAFETY: Match NSView's no-argument tracking lifecycle method.
            unsafe { let _: () = msg_send![super(self), updateTrackingAreas]; }
            self.update_hover_tracking();
        }

        #[unsafe(method(closeWindow:))]
        fn close_window(&self, sender: &AnyObject) {
            if let Some(window) = self.ivars().window.load() {
                self.sync();
                // performClose preserves GPUI's windowShouldClose/close hooks.
                window.performClose(Some(sender));
            }
        }

        #[unsafe(method(minimizeWindow:))]
        fn minimize_window(&self, sender: &AnyObject) {
            if let Some(window) = self.ivars().window.load()
                && !window.styleMask().contains(NSWindowStyleMask::FullScreen)
            {
                self.sync();
                window.performMiniaturize(Some(sender));
            }
        }

        #[unsafe(method(toggleWindowFullscreen:))]
        fn toggle_fullscreen(&self, sender: &AnyObject) {
            if let Some(window) = self.ivars().window.load() {
                self.sync();
                let fullscreen = window.styleMask().contains(NSWindowStyleMask::FullScreen);
                let option = window.currentEvent().is_some_and(|event| {
                    event.modifierFlags().contains(NSEventModifierFlags::Option)
                });
                if option && !fullscreen {
                    window.performZoom(Some(sender));
                } else {
                    window.toggleFullScreen(Some(sender));
                }
                // Native fullscreen is asynchronous. No manual disabled latch
                // is set here; every update derives state from NSWindow.
                self.sync();
            }
        }

        #[unsafe(method(windowStateChanged:))]
        fn window_state_changed(&self, _notification: &NSNotification) {
            self.sync();
            self.update_hover_from_pointer();
            for button in &self.ivars().buttons {
                NSView::setNeedsDisplay(button, true);
            }
        }

        #[unsafe(method(windowUpdated:))]
        fn window_updated(&self, _notification: &NSNotification) {
            // AppKit can replace system buttons after the fullscreen delegate
            // callbacks. This native lifecycle hook performs only changed
            // writes; it must never unconditionally schedule another redraw.
            self.sync();
        }

        #[unsafe(method(windowWillClose:))]
        fn window_will_close(&self, _notification: &NSNotification) {
            self.set_hovered(false);
            // SAFETY: This exact observer was registered below.
            unsafe { NSNotificationCenter::defaultCenter().removeObserver(self) };
        }

        #[unsafe(method(viewDidMoveToWindow))]
        fn view_did_move_to_window(&self) {
            // SAFETY: Match NSView's no-argument lifecycle method.
            unsafe { let _: () = msg_send![super(self), viewDidMoveToWindow]; }
            self.sync();
            self.update_hover_from_pointer();
        }

        #[unsafe(method(viewDidChangeEffectiveAppearance))]
        fn view_did_change_effective_appearance(&self) {
            // SAFETY: Match NSView's no-argument lifecycle method.
            unsafe { let _: () = msg_send![super(self), viewDidChangeEffectiveAppearance]; }
            self.sync();
        }

        #[unsafe(method_id(hitTest:))]
        fn hit_test(&self, point: NSPoint) -> Option<Retained<NSView>> {
            // SAFETY: NSView's hitTest: returns an optional NSView.
            let hit: Option<Retained<NSView>> = unsafe { msg_send![super(self), hitTest: point] };
            // Empty space in the native lane belongs to GPUI's titlebar drag
            // handler; only real AppKit buttons consume pointer input.
            let own: &NSView = self;
            hit.filter(|view| !std::ptr::eq::<NSView>(&**view, own))
        }
    }
);

impl WindowControls {
    fn set_hovered(&self, hovered: bool) {
        if self.ivars().hovered.replace(hovered) != hovered {
            // Rollover is shared by all three controls, including the space
            // between them. Do not turn hover into NSButton's pressed state.
            for button in &self.ivars().buttons {
                NSView::setNeedsDisplay(button, true);
            }
        }
    }

    fn button_group_bounds(&self) -> NSRect {
        let frames = self.ivars().buttons.each_ref().map(|button| button.frame());
        let left = frames
            .iter()
            .map(|frame| frame.origin.x)
            .fold(f64::INFINITY, f64::min);
        let bottom = frames
            .iter()
            .map(|frame| frame.origin.y)
            .fold(f64::INFINITY, f64::min);
        let right = frames
            .iter()
            .map(|frame| frame.origin.x + frame.size.width)
            .fold(f64::NEG_INFINITY, f64::max);
        let top = frames
            .iter()
            .map(|frame| frame.origin.y + frame.size.height)
            .fold(f64::NEG_INFINITY, f64::max);
        NSRect::new(
            NSPoint::new(left, bottom),
            NSSize::new(right - left, top - bottom),
        )
    }

    fn update_hover_from_pointer(&self) {
        let hovered = self.window().is_some_and(|window| {
            if self.isHiddenOrHasHiddenAncestor() || !window.isVisible() || window.isMiniaturized()
            {
                return false;
            }
            let pointer =
                self.convertPoint_fromView(window.mouseLocationOutsideOfEventStream(), None);
            let bounds = self.button_group_bounds();
            pointer.x >= bounds.origin.x
                && pointer.x < bounds.origin.x + bounds.size.width
                && pointer.y >= bounds.origin.y
                && pointer.y < bounds.origin.y + bounds.size.height
        });
        self.set_hovered(hovered);
    }

    fn update_hover_tracking(&self) {
        let bounds = self.button_group_bounds();
        let unchanged = self
            .ivars()
            .tracking_area
            .borrow()
            .as_ref()
            .is_some_and(|area| area.rect() == bounds);
        if !unchanged {
            // Drop the RefCell borrow before calling AppKit. Keep one tracking
            // area for the complete group across layout/fullscreen changes.
            let previous = self.ivars().tracking_area.borrow_mut().take();
            if let Some(previous) = previous {
                self.removeTrackingArea(&previous);
            }
            // SAFETY: Our main-thread NSView implements both requested tracking
            // selectors with NSEvent arguments. NSTrackingArea does not retain
            // its owner; NSView retains the area and releases it on teardown.
            let area = unsafe {
                NSTrackingArea::initWithRect_options_owner_userInfo(
                    NSTrackingArea::alloc(),
                    bounds,
                    NSTrackingAreaOptions::MouseEnteredAndExited
                        | NSTrackingAreaOptions::ActiveAlways
                        | NSTrackingAreaOptions::EnabledDuringMouseDrag,
                    Some(self),
                    None,
                )
            };
            self.addTrackingArea(&area);
            *self.ivars().tracking_area.borrow_mut() = Some(area);
        }
        // Resizing or moving into fullscreen can place the group under a
        // stationary pointer; don't wait for a fresh mouse-enter event.
        self.update_hover_from_pointer();
    }

    fn sync(&self) {
        if self.ivars().syncing.replace(true) {
            return;
        }
        struct SyncGuard<'a>(&'a Cell<bool>);
        impl Drop for SyncGuard<'_> {
            fn drop(&mut self) {
                self.0.set(false);
            }
        }
        let _guard = SyncGuard(&self.ivars().syncing);
        let Some(window) = self.ivars().window.load() else {
            return;
        };
        let Some(content) = window.contentView() else {
            return;
        };
        let style = window.styleMask();
        let fullscreen = style.contains(NSWindowStyleMask::FullScreen);
        let option = NSEvent::modifierFlags_class().contains(NSEventModifierFlags::Option);
        let tooltip_state = (crate::i18n::current_language(), fullscreen, option);
        if self.ivars().tooltip_state.replace(Some(tooltip_state)) != Some(tooltip_state) {
            let zoom_tooltip = if fullscreen {
                crate::tr!("退出全屏幕", "Exit Full Screen")
            } else if option {
                crate::tr!("缩放", "Zoom")
            } else {
                crate::tr!("进入全屏幕", "Enter Full Screen")
            };
            for (button, tooltip) in self.ivars().buttons.iter().zip([
                crate::tr!("关闭", "Close"),
                crate::tr!("最小化", "Minimize"),
                zoom_tooltip,
            ]) {
                button.setToolTip(Some(&NSString::from_str(tooltip)));
            }
        }
        let has_sheet = window.attachedSheet().is_some();
        let enabled = [
            style.contains(NSWindowStyleMask::Closable) && !has_sheet,
            style.contains(NSWindowStyleMask::Miniaturizable) && !fullscreen && !has_sheet,
            style.contains(NSWindowStyleMask::Resizable) && !has_sheet,
        ];
        // GPUI changes this at native fullscreen entry. Our controls belong to
        // content, so keep the empty system titlebar transparent as well.
        if !window.titlebarAppearsTransparent() {
            window.setTitlebarAppearsTransparent(true);
        }
        for (kind, enabled) in [
            NSWindowButton::CloseButton,
            NSWindowButton::MiniaturizeButton,
            NSWindowButton::ZoomButton,
        ]
        .into_iter()
        .zip(enabled)
        {
            if let Some(original) = window.standardWindowButton(kind) {
                // Never read enabled state from the auto-hidden titlebar. Also
                // keep native performClose/performMiniaturize validation valid.
                if original.isEnabled() != enabled {
                    original.setEnabled(enabled);
                }
                if !original.isHidden() {
                    original.setHidden(true);
                }
            }
        }
        let state = self.ivars();
        let height = state
            .buttons
            .iter()
            .map(|b| b.frame().size.height)
            .fold(0., f64::max)
            + state.position.y * 2.;
        let width = state
            .buttons
            .iter()
            .map(|b| b.frame().size.width)
            .sum::<f64>()
            + state.gap * 2.
            + state.position.x * 2.;
        let bounds = content.bounds();
        let frame = NSRect::new(
            NSPoint::new(
                bounds.origin.x,
                bounds.origin.y + bounds.size.height - height,
            ),
            NSSize::new(width, height),
        );
        let geometry_changed = self.frame() != frame;
        if geometry_changed {
            self.setFrame(frame);
        }
        let mut x = state.position.x;
        for (button, enabled) in state.buttons.iter().zip(enabled) {
            let size = button.frame().size;
            let origin = NSPoint::new(x, height - state.position.y - size.height);
            if button.frame().origin != origin {
                button.setFrameOrigin(origin);
            }
            if button.isEnabled() != enabled {
                button.setEnabled(enabled);
            }
            if button.isHidden() {
                button.setHidden(false);
            }
            x += size.width + state.gap;
        }
        if geometry_changed || state.tracking_area.borrow().is_none() {
            self.update_hover_tracking();
        }
    }
}

pub(crate) fn configure(mut options: WindowOptions) -> (WindowOptions, Option<Point<Pixels>>) {
    // A single owner positions the controls. GPUI must not move the hidden
    // system titlebar buttons on resize or after leaving fullscreen.
    let position = options
        .titlebar
        .as_mut()
        .filter(|bar| bar.appears_transparent)
        .and_then(|bar| bar.traffic_light_position.take());
    (options, position)
}

pub(crate) fn attach(window: &Window, position: Point<Pixels>) -> Result<()> {
    let RawWindowHandle::AppKit(handle) =
        raw_window_handle::HasWindowHandle::window_handle(window)?.as_raw()
    else {
        anyhow::bail!("expected an AppKit window handle");
    };
    // SAFETY: GPUI supplies a live NSView handle. This callback runs on the
    // AppKit main thread; the borrowed pointer is never retained in Rust state.
    let view = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
    let native = view.window().context("GPUI view has no native window")?;
    let content = native
        .contentView()
        .context("native window has no content view")?;
    let make = |kind| {
        NSWindow::standardWindowButton_forStyleMask(kind, native.styleMask(), native.mtm())
            .context("AppKit could not create a standard window button")
    };
    let buttons = [
        make(NSWindowButton::CloseButton)?,
        make(NSWindowButton::MiniaturizeButton)?,
        make(NSWindowButton::ZoomButton)?,
    ];
    let close = native
        .standardWindowButton(NSWindowButton::CloseButton)
        .context("missing native close button")?;
    let minimize = native
        .standardWindowButton(NSWindowButton::MiniaturizeButton)
        .context("missing native minimize button")?;
    let gap = minimize.frame().origin.x - close.frame().origin.x - close.frame().size.width;
    let allocated = WindowControls::alloc(native.mtm()).set_ivars(ControlsState {
        window: Weak::from_retained(&native),
        buttons,
        position: NSPoint::new(position.x.to_f64(), position.y.to_f64()),
        gap,
        syncing: Cell::new(false),
        hovered: Cell::new(false),
        tracking_area: RefCell::new(None),
        tooltip_state: Cell::new(None),
    });
    // SAFETY: Initialize the NSView superclass after setting our Rust ivars.
    let controls: Retained<WindowControls> =
        unsafe { msg_send![super(allocated), initWithFrame: NSRect::ZERO] };
    controls.setWantsLayer(true);
    controls.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinYMargin);
    for (button, action) in controls.ivars().buttons.iter().zip([
        sel!(closeWindow:),
        sel!(minimizeWindow:),
        sel!(toggleWindowFullscreen:),
    ]) {
        // SAFETY: Each selector above is implemented on this retained NSView
        // with NSControl's expected one-object action signature. Target is weak.
        unsafe {
            button.setTarget(Some(&controls));
            button.setAction(Some(action));
        }
        controls.addSubview(button);
    }
    content.addSubview_positioned_relativeTo(&controls, NSWindowOrderingMode::Above, None);
    let center = NSNotificationCenter::defaultCenter();
    // SAFETY: AppKit notification names are immutable process-lifetime values;
    // delivery is restricted to this NSWindow on the main thread. These
    // selector observers are weak and automatically unregistered on dealloc.
    unsafe {
        for name in [
            NSWindowDidResizeNotification,
            NSWindowDidEnterFullScreenNotification,
            NSWindowDidExitFullScreenNotification,
            NSWindowDidBecomeKeyNotification,
            NSWindowDidResignKeyNotification,
            NSWindowDidChangeBackingPropertiesNotification,
            NSWindowDidDeminiaturizeNotification,
            NSWindowWillBeginSheetNotification,
            NSWindowDidEndSheetNotification,
        ] {
            center.addObserver_selector_name_object(
                &controls,
                sel!(windowStateChanged:),
                Some(name),
                Some(&native),
            );
        }
        center.addObserver_selector_name_object(
            &controls,
            sel!(windowUpdated:),
            Some(NSWindowDidUpdateNotification),
            Some(&native),
        );
        center.addObserver_selector_name_object(
            &controls,
            sel!(windowWillClose:),
            Some(NSWindowWillCloseNotification),
            Some(&native),
        );
    }
    controls.sync();
    Ok(())
}
