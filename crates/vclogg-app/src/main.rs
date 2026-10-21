#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

mod actions;
mod app_assets;
mod app_icon;
mod app_log;
mod app_paths;
mod build_info;
mod button_accessibility;
mod cloud_filters;
mod color_labels;
mod color_labels_dialog;
mod crash_report;
mod dialog_focus;
mod directory_search_dialog;
mod file_watch;
mod global_search_files_dialog;
mod global_search_table;
mod history_dialog;
mod i18n;
mod keyword_match_style;
mod keyword_match_style_section;
mod log_coloring;
mod log_coloring_row;
mod log_table;
mod log_tag_layer;
mod log_tags;
#[cfg(target_os = "macos")]
mod macos_window_controls;
mod modal_event_layer;
mod notifications;
mod open_directory;
mod open_log;
mod path_identity;
mod pending_tasks;
mod predefined_filters;
mod predefined_filters_dialog;
mod rename_tab_dialog;
mod result_export;
mod row_tag_dialog;
mod search_autocomplete;
mod search_color_exclusions;
mod search_context;
mod selectable_log_text;
mod selection_style;
mod selection_style_section;
mod settings_dialog;
mod single_instance;
mod state_store;
mod system_fonts;
mod tab_resume;
mod ui_performance;
mod ui_theme;
mod update_check;
mod virtual_log_lines;
mod virtual_log_list;
mod window_drag;
mod workspace;
mod workspace_state;

use std::path::PathBuf;

use gpui_kit::component::{TitleBar, theme::ThemeMode};
use gpui_kit::*;

use crate::workspace::{InitialDocument, Workspace};

struct SingleInstanceRuntime {
    _request_tasks: Vec<Task<()>>,
}

impl Global for SingleInstanceRuntime {}

pub(crate) fn open_workspace_window(
    cx: &mut App,
    primary: bool,
    initial_documents: Vec<InitialDocument>,
) -> anyhow::Result<()> {
    open_workspace_window_with_options(
        cx,
        primary,
        initial_documents,
        WindowBounds::Maximized(Bounds::centered(None, size(px(1280.), px(800.)), cx)),
        None,
    )
    .map(|_| ())
}

pub(crate) fn open_workspace_window_at(
    cx: &mut App,
    primary: bool,
    initial_documents: Vec<InitialDocument>,
    bounds: Bounds<Pixels>,
    display_id: Option<DisplayId>,
) -> anyhow::Result<gpui_kit::AnyWindowHandle> {
    open_workspace_window_with_options(
        cx,
        primary,
        initial_documents,
        WindowBounds::Windowed(bounds),
        display_id,
    )
}

fn open_workspace_window_with_options(
    cx: &mut App,
    primary: bool,
    initial_documents: Vec<InitialDocument>,
    window_bounds: WindowBounds,
    display_id: Option<DisplayId>,
) -> anyhow::Result<gpui_kit::AnyWindowHandle> {
    let _scope = ui_performance::trace::scope("Startup.open_workspace_window");
    let _open_stage = crate::open_log::Stage::new("Startup.open_workspace_window", "");
    let window_options = WindowOptions {
        window_bounds: Some(window_bounds),
        display_id,
        ..TitleBar::window_options()
    };
    #[cfg(target_os = "macos")]
    let (window_options, traffic_light_position) = macos_window_controls::configure(window_options);
    // GPUI creates the native window and renderer before invoking the content builder.
    // Keep that cost separate from application-owned setup without patching GPUI.
    let native_window_log = crate::open_log::Stage::new("Startup.prepare_native_window", "");
    let native_window_scope = ui_performance::trace::scope("Startup.prepare_native_window");
    let (handle, _) = gpui_kit::open_window(window_options, cx, |window, cx| {
        drop(native_window_scope);
        native_window_log.event("native_window_ready");
        drop(native_window_log);
        let _scope = ui_performance::trace::scope("Startup.build_workspace_window");
        let _open_stage = crate::open_log::Stage::new("Startup.build_workspace_window", "");
        window.set_window_title("VCLogg2");
        {
            let _scope = ui_performance::trace::scope("Startup.attach_window_icon");
            let _open_stage = crate::open_log::Stage::new("Startup.attach_window_icon", "");
            app_icon::attach_window(window, cx);
        }
        #[cfg(target_os = "macos")]
        {
            let _scope = ui_performance::trace::scope("Startup.attach_window_controls");
            let _open_stage = crate::open_log::Stage::new("Startup.attach_window_controls", "");
            if let Some(position) = traffic_light_position
                && let Err(error) = macos_window_controls::attach(window, position)
            {
                log::error!("Could not initialize native window controls: {error:#}");
            }
        }
        let workspace = cx.new(|cx| Workspace::new(primary, Vec::new(), window, cx));
        {
            let _scope = ui_performance::trace::scope("Startup.register_window");
            let _open_stage = crate::open_log::Stage::new("Startup.register_window", "");
            Workspace::register_window(&workspace, window, cx);
        }
        if !initial_documents.is_empty() {
            workspace.update(cx, |workspace, cx| {
                workspace.open_initial_documents_after_activation(initial_documents, window, cx);
            });
        }
        workspace
    })?;
    let _scope = ui_performance::trace::scope("Startup.activate_window");
    let _open_stage = crate::open_log::Stage::new("Startup.activate_window", "");
    cx.activate(true);
    _ = handle.update(cx, |_, window, _| window.activate_window());
    Ok(handle)
}

fn handle_external_open_request(request: single_instance::OpenRequest, cx: &mut App) {
    let paths = request.into_paths();
    let _trace = ui_performance::trace::scope("ExternalOpen.handle_request");
    let _open_stage = crate::open_log::Stage::new("ExternalOpen.handle_request", "");
    if Workspace::activate_for_external_open(paths.clone(), cx) {
        return;
    }
    if paths.is_empty() {
        if let Err(error) = open_workspace_window(cx, false, Vec::new()) {
            log::error!("外部启动未能创建 VCLogg2 窗口：{error:#}");
        }
        return;
    }

    let initial_documents = paths.into_iter().map(InitialDocument::from_path).collect();
    if let Err(error) = open_workspace_window(cx, false, initial_documents) {
        log::error!("外部文件未能在 VCLogg2 中打开：{error:#}");
    }
}

fn external_request_listener(
    receiver: async_channel::Receiver<single_instance::OpenRequest>,
    cx: &mut App,
) -> Task<()> {
    cx.spawn(async move |cx| {
        while let Ok(request) = receiver.recv().await {
            cx.update(|cx| handle_external_open_request(request, cx));
        }
    })
}

#[cfg(windows)]
fn report_windows_data_directory_error(error: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};

    log::error!("VCLogg2 数据目录初始化失败：{error}");
    let message = format!(
        "无法使用 VCLogg2 数据目录。\n\n{error}\n\n请检查目录权限，或重新运行安装程序选择数据目录。"
    )
    .encode_utf16()
    .chain(std::iter::once(0))
    .collect::<Vec<_>>();
    let title = windows_sys::w!("VCLogg2");
    // Both strings are NUL-terminated and remain alive for this modal call.
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            message.as_ptr(),
            title,
            MB_OK | MB_ICONERROR,
        );
    }
}

fn main() {
    app_log::init();
    let _diagnostic_flush = open_log::FlushOnExit;
    log::info!(target: open_log::TARGET, "process_entry launch={} args={:?}", std::process::id(), std::env::args_os().skip(1).collect::<Vec<_>>());
    let arguments = match ui_performance::trace::startup::Arguments::parse() {
        Ok(arguments) => arguments,
        Err(error) => {
            eprintln!("{error:#}");
            crate::open_log::flush();
            std::process::exit(2);
        }
    };
    let cold_trace = match arguments.begin() {
        Ok(trace) => trace,
        Err(error) => {
            eprintln!("Could not start startup trace: {error:#}");
            crate::open_log::flush();
            std::process::exit(1);
        }
    };
    let startup_scope = ui_performance::trace::scope("Startup.bootstrap");
    #[cfg(windows)]
    if let Err(error) = {
        let _trace = ui_performance::trace::scope("Startup.data_directory");
        let _open_stage = crate::open_log::Stage::new("Startup.data_directory", "");
        app_paths::initialize_windows_data_directory()
    } {
        report_windows_data_directory_error(&error);
        drop(cold_trace);
        crate::open_log::flush();
        std::process::exit(1);
    }
    app_paths::log_development_override();
    log::info!("VCLogg2 {} starting", build_info::VERSION);

    let initial_paths = single_instance::command_line_paths(arguments.paths);
    let startup_result = {
        let _scope = ui_performance::trace::scope("Startup.single_instance");
        let _open_stage = crate::open_log::Stage::new("Startup.single_instance", "");
        single_instance::acquire_or_forward(&initial_paths)
    };
    let primary_instance = match startup_result {
        Ok(single_instance::Startup::Primary(instance)) => {
            log::info!(target: open_log::TARGET, "launch={} role=primary", std::process::id());
            instance
        }
        Ok(single_instance::Startup::Forwarded) => {
            log::info!(target: open_log::TARGET, "launch={} forward_complete", std::process::id());
            if cold_trace.is_some() {
                eprintln!(
                    "An existing VCLogg2 instance received the request; quit it first to capture a cold start."
                );
            }
            return;
        }
        Err(error) => {
            log::info!(target: open_log::TARGET, "launch={} forward_failed error={error:#}", std::process::id());
            log::error!("VCLogg2 单实例初始化失败：{error:#}");
            return;
        }
    };
    #[cfg(windows)]
    if let Err(error) = {
        let _trace = ui_performance::trace::scope("Startup.validate_data_directory");
        let _open_stage = crate::open_log::Stage::new("Startup.validate_data_directory", "");
        app_paths::validate_windows_data_directory()
    } {
        report_windows_data_directory_error(&format!("{error:#}"));
        drop(cold_trace);
        crate::open_log::flush();
        std::process::exit(1);
    }
    let listener_result = {
        let _trace = ui_performance::trace::scope("Startup.instance_listener");
        let _open_stage = crate::open_log::Stage::new("Startup.instance_listener", "");
        primary_instance.start_listener()
    };
    let forwarded_requests = match listener_result {
        Ok(receiver) => receiver,
        Err(error) => {
            log::error!("VCLogg2 单实例监听失败：{error:#}");
            return;
        }
    };
    // Secondary launches have already forwarded and returned. Keep directory
    // scans and crash-report cleanup off the external-open delivery path.
    {
        let _trace = ui_performance::trace::scope("Startup.crash_reporting");
        let _open_stage = crate::open_log::Stage::new("Startup.crash_reporting", "");
        crash_report::install_panic_hook();
    }
    let initial_documents = initial_paths
        .into_iter()
        .map(InitialDocument::from_path)
        .collect::<Vec<_>>();
    #[cfg(target_os = "macos")]
    i18n::initialize_native_language();
    let platform = {
        let _scope = ui_performance::trace::scope("Startup.platform");
        let _open_stage = crate::open_log::Stage::new("Startup.platform", "");
        gpui_kit::platform::current_platform(false)
    };
    let system_fonts = {
        let _scope = ui_performance::trace::scope("Startup.fonts");
        let _open_stage = crate::open_log::Stage::new("Startup.fonts", "");
        system_fonts::SystemFonts::new(platform.text_system())
    };
    let app = {
        let _scope = ui_performance::trace::scope("Startup.application");
        let _open_stage = crate::open_log::Stage::new("Startup.application", "");
        Application::with_platform(platform).with_assets(app_assets::Assets)
    };
    let (platform_open_sender, platform_open_receiver) = async_channel::unbounded();
    app.on_open_urls(move |urls| {
        let paths = urls
            .into_iter()
            .filter_map(|value| {
                url::Url::parse(&value)
                    .ok()
                    .filter(|url| url.scheme() == "file")
                    .and_then(|url| url.to_file_path().ok())
                    .or_else(|| {
                        let path = PathBuf::from(value);
                        path.is_absolute().then_some(path)
                    })
            })
            .collect::<Vec<_>>();
        if !paths.is_empty() {
            _ = platform_open_sender.send_blocking(single_instance::OpenRequest::new(paths));
        }
    });
    app.on_reopen(|cx| {
        handle_external_open_request(single_instance::OpenRequest::new(Vec::new()), cx);
    });

    let trace_attachment = cold_trace
        .as_ref()
        .map(ui_performance::trace::startup::StartupTrace::attachment);
    drop(startup_scope);
    app.run(move |cx| {
        if let Some(attachment) = trace_attachment {
            _ = attachment.send(cx.foreground_journal().collector());
        }
        let services_scope = ui_performance::trace::scope("Startup.initialize_services");
        {
            let _trace = ui_performance::trace::scope("Startup.gpui_kit");
            let _open_stage = crate::open_log::Stage::new("Startup.gpui_kit", "");
            gpui_kit::init(cx);
        }
        notifications::init(cx);
        cx.set_global(system_fonts);
        cx.set_cursor_hide_mode(CursorHideMode::Never);
        {
            let _trace = ui_performance::trace::scope("Startup.theme");
            let _open_stage = crate::open_log::Stage::new("Startup.theme", "");
            ui_theme::apply_product_theme(ThemeMode::Light, cx);
        }
        actions::init(cx);
        Workspace::init_window_registry(cx);
        {
            let _trace = ui_performance::trace::scope("Startup.app_icon");
            let _open_stage = crate::open_log::Stage::new("Startup.app_icon", "");
            app_icon::init(cx);
        }
        let mut request_tasks = vec![external_request_listener(platform_open_receiver, cx)];
        if let Some(receiver) = forwarded_requests {
            request_tasks.push(external_request_listener(receiver, cx));
        }
        cx.set_global(SingleInstanceRuntime {
            _request_tasks: request_tasks,
        });
        std::mem::forget(cx.on_app_quit(Workspace::flush_all_on_quit));
        cx.set_quit_mode(QuitMode::LastWindowClosed);
        // Window creation has its own scopes and is not service initialization.
        drop(services_scope);

        // Platform window geometry is a physical boundary; descendant UI uses
        // GPUI rem helpers and theme-relative sizing instead of fixed pixels.
        open_workspace_window(cx, true, initial_documents).expect("failed to open VCLogg2 window");
        cx.on_window_closed(|cx, window_id| {
            Workspace::unregister_window(window_id, cx);
        })
        .detach();
    });
}
