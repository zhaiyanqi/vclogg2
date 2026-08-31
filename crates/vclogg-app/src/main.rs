#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

mod actions;
mod app_log;
mod app_paths;
mod cloud_filters;
mod color_labels;
mod color_labels_dialog;
mod crash_report;
mod directory_search_dialog;
mod global_search_files_dialog;
mod global_search_table;
mod history_dialog;
mod i18n;
mod log_table;
mod open_directory;
mod predefined_filters;
mod predefined_filters_dialog;
mod rename_tab_dialog;
mod result_export;
mod search_autocomplete;
mod search_context;
mod selectable_log_text;
mod settings_dialog;
mod single_instance;
mod state_store;
mod tab_resume;
mod trash;
mod ui_performance;
mod ui_theme;
mod updater;
mod workspace;

use gpui::*;
use gpui_component::{Root, TitleBar, theme::ThemeMode};

use crate::workspace::{InitialDocument, Workspace};

struct SingleInstanceRuntime {
    _request_task: Task<()>,
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
}

pub(crate) fn open_workspace_window_at(
    cx: &mut App,
    primary: bool,
    initial_documents: Vec<InitialDocument>,
    bounds: Bounds<Pixels>,
    display_id: Option<DisplayId>,
) -> anyhow::Result<()> {
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
) -> anyhow::Result<()> {
    let window_options = WindowOptions {
        window_bounds: Some(window_bounds),
        display_id,
        ..TitleBar::window_options()
    };
    let handle = cx.open_window(window_options, |window, cx| {
        window.set_window_title("VCLogg2");
        let workspace = cx.new(|cx| Workspace::new(primary, initial_documents, window, cx));
        Workspace::register_window(&workspace, window, cx);
        cx.new(|cx| Root::new(workspace, window, cx))
    })?;
    cx.activate(true);
    _ = handle.update(cx, |_, window, _| window.activate_window());
    Ok(())
}

fn handle_external_open_request(request: single_instance::OpenRequest, cx: &mut App) {
    if request.paths.is_empty() {
        if let Err(error) = open_workspace_window(cx, false, Vec::new()) {
            log::error!("外部启动未能创建 VCLogg2 窗口：{error:#}");
        }
        return;
    }

    if Workspace::open_external_paths_in_last_active_window(&request.paths, cx) {
        return;
    }

    let initial_documents = request
        .paths
        .into_iter()
        .map(InitialDocument::from_path)
        .collect();
    if let Err(error) = open_workspace_window(cx, false, initial_documents) {
        log::error!("外部文件未能在 VCLogg2 中打开：{error:#}");
    }
}

fn install_single_instance_listener(
    receiver: async_channel::Receiver<single_instance::OpenRequest>,
    cx: &mut App,
) {
    let request_task = cx.spawn(async move |cx| {
        while let Ok(request) = receiver.recv().await {
            cx.update(|cx| handle_external_open_request(request, cx));
        }
    });
    cx.set_global(SingleInstanceRuntime {
        _request_task: request_task,
    });
}

fn main() {
    app_log::init();
    app_paths::log_development_override();
    crash_report::install_panic_hook();
    log::info!("VCLogg2 {} starting", env!("CARGO_PKG_VERSION"));

    let initial_paths = single_instance::command_line_paths();
    let primary_instance = match single_instance::acquire_or_forward(&initial_paths) {
        Ok(single_instance::Startup::Primary(instance)) => instance,
        Ok(single_instance::Startup::Forwarded) => return,
        Err(error) => {
            log::error!("VCLogg2 单实例初始化失败：{error:#}");
            return;
        }
    };
    let forwarded_requests = match primary_instance.start_listener() {
        Ok(receiver) => receiver,
        Err(error) => {
            log::error!("VCLogg2 单实例监听失败：{error:#}");
            return;
        }
    };
    let initial_documents = initial_paths
        .into_iter()
        .map(InitialDocument::from_path)
        .collect::<Vec<_>>();
    let app = gpui_platform::application().with_assets(gpui_component_assets::Assets);

    app.run(move |cx| {
        ui_performance::init_ui_thread();
        ui_performance::start_framework_monitor(cx);
        gpui_component::init(cx);
        ui_theme::apply_product_theme(ThemeMode::Light, cx);
        actions::init(cx);
        Workspace::init_window_registry(cx);
        if let Some(receiver) = forwarded_requests {
            install_single_instance_listener(receiver, cx);
        }
        std::mem::forget(cx.on_app_quit(Workspace::flush_all_on_quit));
        cx.set_quit_mode(QuitMode::LastWindowClosed);

        // Platform window geometry is a physical boundary; descendant UI uses
        // GPUI rem helpers and theme-relative sizing instead of fixed pixels.
        open_workspace_window(cx, true, initial_documents).expect("failed to open VCLogg2 window");
        cx.on_window_closed(|cx, window_id| {
            Workspace::unregister_window(window_id, cx);
        })
        .detach();
    });
}
