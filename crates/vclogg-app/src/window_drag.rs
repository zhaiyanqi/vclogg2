//! Move a detached workspace without surrendering its tab drag to a native modal loop.
use gpui_kit::{App, Pixels, Point, Window};

pub(crate) fn move_to(window: &Window, origin: Point<Pixels>, cx: &App) -> anyhow::Result<()> {
    // The headless platform has no native window. The gesture owns its requested bounds.
    if cx.is_test() {
        return Ok(());
    }
    platform_move_to(window, origin, cx)
}

/// GPUI's macOS backend reports display-relative window bounds. Use one global
/// top-left coordinate space for hit testing and moving across displays.
pub(crate) fn screen_origin(window: &Window, cx: &App) -> Point<Pixels> {
    #[cfg(target_os = "macos")]
    if !cx.is_test()
        && let Ok(native) = native_window(window)
    {
        let frame = native.frame();
        let height = cx
            .primary_display()
            .map_or(0., |display| display.bounds().size.height.as_f32());
        return gpui_kit::point(
            gpui_kit::px(frame.origin.x as f32),
            gpui_kit::px(height - (frame.origin.y + frame.size.height) as f32),
        );
    }
    let _ = cx;
    window.bounds().origin
}

pub(crate) fn initial_bounds(
    bounds: gpui_kit::Bounds<Pixels>,
    display: Option<gpui_kit::DisplayId>,
    cx: &App,
) -> gpui_kit::Bounds<Pixels> {
    #[cfg(target_os = "macos")]
    if !cx.is_test()
        && let Some(display) = cx
            .displays()
            .into_iter()
            .find(|candidate| Some(candidate.id()) == display)
    {
        return gpui_kit::Bounds::new(bounds.origin - display.bounds().origin, bounds.size);
    }
    let _ = (display, cx);
    bounds
}

#[cfg(target_os = "macos")]
fn native_window(window: &Window) -> anyhow::Result<objc2::rc::Retained<objc2_app_kit::NSWindow>> {
    use anyhow::Context as _;
    use objc2_app_kit::NSView;
    use raw_window_handle::RawWindowHandle;
    let RawWindowHandle::AppKit(handle) =
        raw_window_handle::HasWindowHandle::window_handle(window)?.as_raw()
    else {
        anyhow::bail!("expected an AppKit window");
    };
    // SAFETY: GPUI owns the live NSView and this is called on its foreground thread.
    let view = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
    view.window().context("GPUI view has no native window")
}

#[cfg(target_os = "macos")]
fn platform_move_to(window: &Window, origin: Point<Pixels>, cx: &App) -> anyhow::Result<()> {
    use objc2_foundation::NSPoint;
    let native = native_window(window)?;
    let height = cx
        .primary_display()
        .map_or(0., |display| display.bounds().size.height.as_f32());
    // Moving can synchronously invoke GPUI's bounds/scale callbacks. Like GPUI's
    // native resize implementation, run outside the current App/Window borrow.
    cx.foreground_executor()
        .spawn(async move {
            if native.isVisible() {
                let frame = native.frame();
                native.setFrameOrigin(NSPoint::new(
                    origin.x.as_f32() as f64,
                    (height - origin.y.as_f32()) as f64 - frame.size.height,
                ));
            }
        })
        .detach();
    Ok(())
}

#[cfg(target_os = "windows")]
fn platform_move_to(window: &Window, origin: Point<Pixels>, _cx: &App) -> anyhow::Result<()> {
    use raw_window_handle::RawWindowHandle;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos,
    };
    let RawWindowHandle::Win32(handle) =
        raw_window_handle::HasWindowHandle::window_handle(window)?.as_raw()
    else {
        anyhow::bail!("expected a Win32 window");
    };
    let scale = window.scale_factor();
    // SAFETY: the HWND is owned by GPUI; no ownership transfer or size change occurs.
    let result = unsafe {
        SetWindowPos(
            handle.hwnd.get() as _,
            std::ptr::null_mut(),
            (origin.x.as_f32() * scale).round() as i32,
            (origin.y.as_f32() * scale).round() as i32,
            0,
            0,
            SWP_NOACTIVATE | SWP_NOSIZE | SWP_NOZORDER,
        )
    };
    if result == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn platform_move_to(window: &Window, origin: Point<Pixels>, _cx: &App) -> anyhow::Result<()> {
    use raw_window_handle::RawWindowHandle;
    use x11rb::{
        connection::Connection as _,
        protocol::xproto::{ConfigureWindowAux, ConnectionExt as _},
    };
    let id = match raw_window_handle::HasWindowHandle::window_handle(window)?.as_raw() {
        RawWindowHandle::Xcb(handle) => handle.window.get(),
        RawWindowHandle::Xlib(handle) => handle.window as u32,
        _ => anyhow::bail!("the compositor does not allow positioning a window"),
    };
    let (connection, _) = x11rb::connect(None)?;
    let scale = window.scale_factor();
    connection
        .configure_window(
            id,
            &ConfigureWindowAux::new()
                .x((origin.x.as_f32() * scale).round() as i32)
                .y((origin.y.as_f32() * scale).round() as i32),
        )?
        .check()?;
    connection.flush()?;
    Ok(())
}
