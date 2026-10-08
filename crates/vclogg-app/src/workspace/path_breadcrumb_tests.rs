use super::super::{DocumentLoadState, PreparedDocument, Workspace};
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, InputEvent as _, MouseButton, MouseDownEvent, Task, TestAppContext, px, size,
};
use std::{collections::BTreeMap, sync::Arc};
use vclogg_core::{LogDocument, SearchRange, SearchResult};

#[gpui_kit::test]
fn compact_breadcrumb_context_menu_focuses_before_paint_and_dismisses(cx: &mut TestAppContext) {
    check_breadcrumb_context_menu(cx, false);
}

#[gpui_kit::test]
fn full_breadcrumb_context_menu_focuses_before_paint_and_dismisses(cx: &mut TestAppContext) {
    check_breadcrumb_context_menu(cx, true);
}

fn check_breadcrumb_context_menu(cx: &mut TestAppContext, show_full_path: bool) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::actions::init(cx);
        Workspace::init_window_registry(cx);
        crate::notifications::init(cx);
        crate::app_icon::init(cx);
    });
    let directory = tempfile::tempdir().unwrap();
    let paths = [directory.path().join("a.log")];
    let prepared = paths
        .iter()
        .map(|path| {
            std::fs::write(path, "error\ninfo\n").unwrap();
            (
                path.clone(),
                Ok(PreparedDocument {
                    document: Arc::new(LogDocument::open(path).unwrap()),
                    cached_complete_document: None,
                    session: None,
                    color_labels_snapshot: None,
                    resolved_color_rules: Arc::default(),
                    search_result: SearchResult::default(),
                    search_range: SearchRange::default(),
                    search_matcher: None,
                    search_case_sensitive: false,
                    search_regex: false,
                    warning: None,
                    load_state: DocumentLoadState::Ready,
                    pending_index_cache: None,
                    upgrade_frame: None,
                }),
            )
        })
        .collect();
    let mut workspace = None;
    let handle = cx.open_window(size(px(1400.), px(900.)), |window, cx| {
        let view = cx.new(|cx| Workspace::new(false, Vec::new(), window, cx));
        view.update(cx, |view, cx| {
            view.persistence._bootstrap_task = Task::ready(());
            view.persistence.state_tasks.clear();
            view._cloud_client_bootstrap_task = Task::ready(());
            view.install_documents(
                prepared,
                Some(&paths[0]),
                &BTreeMap::new(),
                None,
                true,
                window,
                cx,
            );
        });
        workspace = Some(view.clone());
        Root::new(view, window, cx)
    });
    let workspace = workspace.unwrap();
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        workspace.update(cx, |view, cx| {
            view.app_settings.show_full_path = show_full_path;
            cx.notify();
        });
        window.render_frame(cx);
        let breadcrumb = format!("toolbar-breadcrumb-{:?}", paths[0]);
        // Focus the actual breadcrumb through keyboard navigation, without
        // invoking its left-click action (which opens the system file manager).
        for _ in 0..100 {
            if window.find(breadcrumb.clone()).focused() == Some(true) {
                break;
            }
            window.press("tab", cx);
        }
        assert_eq!(window.find(breadcrumb.clone()).focused(), Some(true));
        let previous_focus = window.focused(cx);
        let position = window.find(breadcrumb).bounds().center();
        window.dispatch_event(
            MouseDownEvent {
                button: MouseButton::Right,
                position,
                modifiers: Default::default(),
                click_count: 1,
                first_mouse: false,
            }
            .to_platform_input(),
            cx,
        );
        // Runs after ContextMenu's deferred builder but before the next draw.
        window.defer(cx, move |window, cx| {
            assert_ne!(
                window.focused(cx),
                previous_focus,
                "the menu must take focus before prepaint starts"
            );
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        let focus_before_frame = window.focused(cx);
        window.render_frame(cx);
        assert_eq!(
            window.focused(cx),
            focus_before_frame,
            "opening the menu must transfer focus before prepaint starts"
        );
        assert!(window.find("popup-menu").visible());
        window.press("escape", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("popup-menu").is_none());
        assert!(window.focused(cx).is_some());
    })
    .unwrap();
}
