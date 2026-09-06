//! Built-in application identity and native icon adapters. Preferences own the selection;
//! this process-wide snapshot only coordinates native windows and late settings restores.

use std::sync::{Arc, OnceLock};

use gpui::{App, Global, Image, ImageFormat, Window};
use gpui_component::WindowExt as _;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum AppIcon {
    #[default]
    Soft,
    Compact,
    Illustration,
    Sticker,
}

impl AppIcon {
    pub(crate) const ALL: [Self; 4] =
        [Self::Soft, Self::Compact, Self::Illustration, Self::Sticker];

    pub(crate) fn storage_value(self) -> &'static str {
        match self {
            Self::Soft => "soft",
            Self::Compact => "compact",
            Self::Illustration => "illustration",
            Self::Sticker => "sticker",
        }
    }

    pub(crate) fn from_storage_value(value: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|icon| icon.storage_value() == value)
            .unwrap_or_default()
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Soft => crate::tr!("A · 软萌立体（默认）", "A · Soft 3D (default)"),
            Self::Compact => crate::tr!("A · 圆润坐姿", "A · Rounded sitting pose"),
            Self::Illustration => crate::tr!("B · 清爽插画", "B · Clean illustration"),
            Self::Sticker => crate::tr!("C · 可爱贴纸", "C · Cute sticker"),
        }
    }

    fn png(self) -> &'static [u8] {
        match self {
            Self::Soft => include_bytes!("../resources/icons/soft.png"),
            Self::Compact => include_bytes!("../resources/icons/compact.png"),
            Self::Illustration => include_bytes!("../resources/icons/illustration.png"),
            Self::Sticker => include_bytes!("../resources/icons/sticker.png"),
        }
    }

    pub(crate) fn image(self) -> Arc<Image> {
        static IMAGES: OnceLock<[Arc<Image>; 4]> = OnceLock::new();
        IMAGES.get_or_init(|| {
            Self::ALL.map(|icon| Arc::new(Image::from_bytes(ImageFormat::Png, icon.png().to_vec())))
        })[self.index()]
        .clone()
    }

    pub(crate) fn index(self) -> usize {
        Self::ALL.iter().position(|icon| *icon == self).unwrap_or(0)
    }

    #[cfg(target_os = "linux")]
    fn desktop_id(self) -> &'static str {
        match self {
            Self::Soft => "com.vclogg2.desktop",
            Self::Compact => "com.vclogg2.desktop.compact",
            Self::Illustration => "com.vclogg2.desktop.illustration",
            Self::Sticker => "com.vclogg2.desktop.sticker",
        }
    }
}

#[derive(Default)]
struct IconRuntime {
    current: Option<AppIcon>,
}

impl Global for IconRuntime {}

pub(crate) fn init(cx: &mut App) {
    cx.set_global(IconRuntime::default());
}

/// A later window's database bootstrap must not overwrite a live preview or newer choice.
pub(crate) fn restored_icon(saved: AppIcon, cx: &App) -> AppIcon {
    cx.try_global::<IconRuntime>()
        .and_then(|runtime| runtime.current)
        .unwrap_or(saved)
}

pub(crate) fn attach_window(window: &mut Window, cx: &mut App) {
    let icon = cx.global::<IconRuntime>().current.unwrap_or_default();
    if let Err(error) = platform::apply_window(icon, window) {
        log::warn!("Could not initialize window icon: {error:#}");
    }
    if cx.global::<IconRuntime>().current.is_none()
        && let Err(error) = platform::apply_application(icon)
    {
        log::warn!("Could not initialize application icon: {error:#}");
    }
}

/// Called by the existing preview/commit/cancel path, never from a renderer.
pub(crate) fn apply(icon: AppIcon, window: &mut Window, cx: &mut App) {
    if cx.global::<IconRuntime>().current == Some(icon) {
        return;
    }
    cx.global_mut::<IconRuntime>().current = Some(icon);
    let mut errors = Vec::new();
    if let Err(error) = platform::apply_application(icon) {
        errors.push(error.to_string());
    }
    if let Err(error) = platform::apply_window(icon, window) {
        errors.push(error.to_string());
    }
    let source = window.window_handle();
    for handle in cx.windows().into_iter().filter(|handle| *handle != source) {
        // Updating only the native window avoids re-entering any Workspace entity.
        if let Ok(Err(error)) =
            handle.update(cx, |_, window, _| platform::apply_window(icon, window))
        {
            errors.push(error.to_string());
        }
    }
    if !errors.is_empty() {
        log::warn!("Could not apply application icon: {}", errors.join("; "));
        window.push_notification(
            crate::tr!(
                "部分系统图标未能更新，请切换到其他图标后重试。",
                "Some system icons couldn’t be updated. Switch to another icon and try again."
            ),
            cx,
        );
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use anyhow::Context as _;
    use objc2::{AnyThread as _, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;

    pub(super) fn apply_application(icon: AppIcon) -> anyhow::Result<()> {
        let mtm = MainThreadMarker::new().context("Application icon requires the main thread")?;
        let data = NSData::with_bytes(icon.png());
        let image = NSImage::initWithData(NSImage::alloc(), &data)
            .context("Could not decode application icon")?;
        // GPUI invokes this on the AppKit main thread. AppKit retains the image;
        // no signed .app bundle or Finder metadata is modified.
        unsafe {
            NSApplication::sharedApplication(mtm).setApplicationIconImage(Some(&image));
        }
        Ok(())
    }

    pub(super) fn apply_window(_: AppIcon, _: &mut Window) -> anyhow::Result<()> {
        Ok(())
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use super::*;
    use raw_window_handle::RawWindowHandle;
    use windows_sys::Win32::{
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::{
            GetSystemMetrics, ICON_BIG, ICON_SMALL, IMAGE_ICON, LR_SHARED, LoadImageW, SM_CXICON,
            SM_CXSMICON, SM_CYICON, SM_CYSMICON, SendMessageW, WM_SETICON,
        },
    };

    pub(super) fn apply_application(_: AppIcon) -> anyhow::Result<()> {
        Ok(())
    }

    pub(super) fn apply_window(icon: AppIcon, window: &mut Window) -> anyhow::Result<()> {
        let RawWindowHandle::Win32(handle) =
            raw_window_handle::HasWindowHandle::window_handle(window)?.as_raw()
        else {
            anyhow::bail!("Expected a Win32 window");
        };
        // Resource IDs 1–4 match vclogg2.rc; shared resource handles belong to Windows
        // and must not be destroyed when a window switches icons or closes.
        unsafe {
            let module = GetModuleHandleW(std::ptr::null());
            if module.is_null() {
                return Err(std::io::Error::last_os_error().into());
            }
            for (kind, width, height) in [
                (ICON_SMALL, SM_CXSMICON, SM_CYSMICON),
                (ICON_BIG, SM_CXICON, SM_CYICON),
            ] {
                let handle_icon = LoadImageW(
                    module,
                    (icon.index() + 1) as *const u16,
                    IMAGE_ICON,
                    GetSystemMetrics(width),
                    GetSystemMetrics(height),
                    LR_SHARED,
                );
                if handle_icon.is_null() {
                    return Err(std::io::Error::last_os_error().into());
                }
                SendMessageW(
                    handle.hwnd.get() as _,
                    WM_SETICON,
                    kind as usize,
                    handle_icon as isize,
                );
            }
        }
        Ok(())
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use super::*;
    use raw_window_handle::RawWindowHandle;
    use x11rb::{
        connection::Connection as _,
        protocol::xproto::{AtomEnum, ConnectionExt as _, PropMode},
        wrapper::ConnectionExt as _,
    };

    pub(super) fn apply_application(_: AppIcon) -> anyhow::Result<()> {
        Ok(())
    }

    pub(super) fn apply_window(icon: AppIcon, window: &mut Window) -> anyhow::Result<()> {
        let handle = raw_window_handle::HasWindowHandle::window_handle(window)?.as_raw();
        let xid = match handle {
            RawWindowHandle::Xcb(handle) => Some(handle.window.get()),
            RawWindowHandle::Xlib(handle) => Some(u32::try_from(handle.window)?),
            RawWindowHandle::Wayland(_) => None,
            _ => anyhow::bail!("Unsupported Linux window backend"),
        };
        if let Some(xid) = xid {
            let (connection, _) = x11rb::connect(None)?;
            let atom = connection
                .intern_atom(false, b"_NET_WM_ICON")?
                .reply()?
                .atom;
            // Raster dimensions are the EWMH boundary, independent of interface zoom.
            let pixels = image::load_from_memory_with_format(icon.png(), image::ImageFormat::Png)?
                .resize_exact(128, 128, image::imageops::FilterType::Lanczos3)
                .to_rgba8();
            let mut data = Vec::with_capacity(2 + 128 * 128);
            data.extend([128, 128]);
            data.extend(
                pixels
                    .pixels()
                    .map(|pixel| u32::from_be_bytes([pixel[3], pixel[0], pixel[1], pixel[2]])),
            );
            connection
                .change_property32(PropMode::REPLACE, xid, atom, AtomEnum::CARDINAL, &data)?
                .check()?;
            connection.flush()?;
        }
        // The Linux installer provides matching hidden desktop entries and icon names.
        // Wayland compositors resolve this ID instead of accepting a window bitmap.
        window.set_app_id(icon.desktop_id());
        Ok(())
    }
}
