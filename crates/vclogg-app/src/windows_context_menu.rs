//! Current-user Explorer integration. The registry is the persisted source of truth.
use std::{ffi::OsStr, io, os::windows::ffi::OsStrExt as _, ptr};

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _, button::Button, h_flex, switch::Switch,
    v_flex,
};
use gpui_kit::{
    App, AppContext as _, Context, Entity, Global, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, Styled as _, Window, div,
    prelude::FluentBuilder as _,
};
use windows_sys::Win32::{
    Foundation::{ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, ERROR_SUCCESS},
    System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
        RegCreateKeyExW, RegDeleteTreeW, RegOpenKeyExW, RegSetValueExW,
    },
    UI::Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify},
};

const MENU_KEY: &str = r"Software\Classes\*\shell\vclogg2.open";

pub(crate) struct ContextMenuSettings {
    enabled: Option<bool>,
    busy: bool,
    error: Option<SharedString>,
}

struct SharedSettings(Entity<ContextMenuSettings>);
impl Global for SharedSettings {}

pub(crate) fn settings(cx: &mut App) -> Entity<ContextMenuSettings> {
    if let Some(shared) = cx.try_global::<SharedSettings>() {
        let settings = shared.0.clone();
        settings.update(cx, |settings, cx| settings.update_registration(None, cx));
        return settings;
    }
    let settings = cx.new(ContextMenuSettings::new);
    cx.set_global(SharedSettings(settings.clone()));
    settings
}

impl ContextMenuSettings {
    fn new(cx: &mut Context<Self>) -> Self {
        let mut settings = Self {
            enabled: None,
            busy: false,
            error: None,
        };
        settings.update_registration(None, cx);
        settings
    }

    fn update_registration(&mut self, enabled: Option<bool>, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        // Retain the owner through completion even if the settings window closes.
        let owner = cx.entity();
        cx.spawn(async move |_, cx| {
            let (state, error) = cx
                .background_spawn(async move {
                    let error = enabled.and_then(|enabled| set_enabled(enabled).err());
                    (is_registered(), error)
                })
                .await;
            _ = owner.update(cx, |this, cx| {
                this.busy = false;
                let (state, error) = match state {
                    Ok(state) => (Some(state), error),
                    Err(read_error) => (None, error.or(Some(read_error))),
                };
                this.enabled = state;
                this.error = error.map(|error| {
                    format!(
                        "{}: {error}",
                        crate::tr!(
                            "无法读取或更新右键菜单，请重试",
                            "Could not read or update the context menu. Try again"
                        )
                    )
                    .into()
                });
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

impl Render for ContextMenuSettings {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let control = Switch::new("settings-windows-context-menu")
            .small()
            .checked(self.enabled.unwrap_or(false))
            .disabled(self.busy || self.enabled.is_none())
            .accessibility_label(crate::tr!("文件右键菜单", "File context menu"))
            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                this.update_registration(Some(*checked), cx);
            }));
        let row =
            v_flex()
                .px_3()
                .py_2()
                .gap_2()
                .child(
                    h_flex()
                        .gap_3()
                        .min_h_10()
                        .child(
                            div()
                                .flex_1()
                                .text_sm()
                                .child(crate::tr!("使用 vclogg 打开", "Open with vclogg")),
                        )
                        .child(control),
                )
                .when(self.busy, |view| {
                    view.child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(crate::tr!(
                                "正在更新右键菜单状态…",
                                "Updating context menu status…"
                            )),
                    )
                })
                .when_some(self.error.clone(), |view, error| {
                    view.child(
                        h_flex()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_sm()
                                    .text_color(cx.theme().danger)
                                    .child(error),
                            )
                            .child(
                                Button::new("settings-context-menu-retry")
                                    .small()
                                    .label(crate::tr!("重新读取", "Reload status"))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.update_registration(None, cx)
                                    })),
                            ),
                    )
                });
        crate::settings_dialog::layout::section(
            crate::tr!("文件右键菜单", "File context menu"),
            Some(crate::tr!(
                "在文件右键菜单中添加“使用 vclogg 打开”，仅对当前用户生效。Windows 11 中可在“显示更多选项”中找到。",
                "Add “使用 vclogg 打开” (Open with vclogg) to the file context menu for the current user. On Windows 11, look under Show more options.",
            ).to_owned()),
            [row.into_any_element()],
            cx,
        )
        .id("settings-windows-context-menu-section")
    }
}

struct RegistryKey(HKEY);

impl Drop for RegistryKey {
    fn drop(&mut self) {
        // SAFETY: this handle was opened by this module and is closed exactly once.
        unsafe {
            RegCloseKey(self.0);
        }
    }
}

fn wide(value: impl AsRef<OsStr>) -> Vec<u16> {
    value.as_ref().encode_wide().chain(Some(0)).collect()
}

fn check(status: u32) -> io::Result<()> {
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(status as i32))
    }
}

fn is_registered() -> io::Result<bool> {
    let path = wide(MENU_KEY);
    let mut key = ptr::null_mut();
    // SAFETY: path is NUL-terminated and the output handle is valid writable storage.
    let status = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, path.as_ptr(), 0, KEY_READ, &mut key) };
    if matches!(status, ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND) {
        return Ok(false);
    }
    check(status)?;
    let _key = RegistryKey(key);
    Ok(true)
}

fn create_key(path: &str) -> io::Result<RegistryKey> {
    let path = wide(path);
    let mut key = ptr::null_mut();
    // SAFETY: all pointers are valid for the call; optional parameters are null.
    check(unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            path.as_ptr(),
            0,
            ptr::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            ptr::null(),
            &mut key,
            ptr::null_mut(),
        )
    })?;
    Ok(RegistryKey(key))
}

fn set_string(key: &RegistryKey, name: &str, value: &[u16]) -> io::Result<()> {
    let name = wide(name);
    let bytes = u32::try_from(std::mem::size_of_val(value))
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Registry value is too long"))?;
    // SAFETY: the handle is owned and the UTF-16 buffers include their terminators.
    check(unsafe {
        RegSetValueExW(
            key.0,
            name.as_ptr(),
            0,
            REG_SZ,
            value.as_ptr().cast(),
            bytes,
        )
    })
}

fn set_enabled(enabled: bool) -> io::Result<()> {
    let result = if enabled {
        register()
    } else {
        let path = wide(MENU_KEY);
        // SAFETY: only our dedicated current-user verb subtree is removed.
        let status = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, path.as_ptr()) };
        if matches!(status, ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND) {
            Ok(())
        } else {
            check(status)
        }
    };
    // Refresh Explorer even after a partially completed operation.
    // SAFETY: association-change notifications do not carry item pointers.
    unsafe {
        SHChangeNotify(
            SHCNE_ASSOCCHANGED as i32,
            SHCNF_IDLIST,
            ptr::null(),
            ptr::null(),
        );
    }
    result
}

fn register() -> io::Result<()> {
    let executable = std::env::current_exe()?;
    // Preserve native UTF-16 paths, including spaces and non-Unicode filenames.
    let mut command = vec![b'"' as u16];
    command.extend(executable.as_os_str().encode_wide());
    command.extend("\" \"%1\"".encode_utf16());
    command.push(0);
    let command_key = create_key(&format!(r"{MENU_KEY}\command"))?;
    set_string(&command_key, "", &command)?;
    let key = create_key(MENU_KEY)?;
    set_string(&key, "", &wide("使用 vclogg 打开"))?;
    // Explorer does not infer a static verb's icon from its command.
    let mut icon = vec![b'"' as u16];
    icon.extend(executable.as_os_str().encode_wide());
    icon.extend("\",0".encode_utf16());
    icon.push(0);
    set_string(&key, "Icon", &icon)?;
    set_string(&key, "MultiSelectModel", &wide("Single"))?;
    Ok(())
}
