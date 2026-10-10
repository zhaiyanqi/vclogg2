use super::*;
use gpui_kit::{TestAppContext, component::Root, test::TestWindowExt};

fn init(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::actions::init(cx);
        Workspace::init_window_registry(cx);
        crate::notifications::init(cx);
        crate::app_icon::init(cx);
        cx.set_global(crate::system_fonts::SystemFonts::new(Arc::new(
            gpui_kit::NoopTextSystem,
        )));
    });
}

fn workspace(
    cx: &mut TestAppContext,
    store: Arc<StateStore>,
) -> (AnyWindowHandle, Entity<Workspace>) {
    let mut owner = None;
    let window = cx.add_window(|window, cx| {
        let view = cx.new(|cx| Workspace::new(false, Vec::new(), window, cx));
        view.update(cx, |view, _| {
            view.persistence._bootstrap_task = Task::ready(());
            view.persistence.store = Some(store);
        });
        Workspace::register_window(&view, window, cx);
        owner = Some(view.clone());
        Root::new(view, window, cx)
    });
    (window.into(), owner.unwrap())
}

fn open(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    owner: &Entity<Workspace>,
) -> (AnyWindowHandle, Entity<SettingsWindow>) {
    cx.update_window(handle, |_, window, cx| {
        owner.update(cx, |owner, cx| owner.open_settings_dialog(None, window, cx))
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let (handle, view) = cx
            .global::<WorkspaceWindowRegistry>()
            .settings_window
            .clone()
            .unwrap();
        (handle, view.upgrade().unwrap())
    })
}

fn settle(cx: &mut TestAppContext) {
    cx.run_until_parked();
    cx.background_executor
        .advance_clock(Duration::from_millis(350));
    cx.run_until_parked();
}

#[gpui_kit::test]
fn changes_apply_across_windows_and_save_without_closing(cx: &mut TestAppContext) {
    init(cx);
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(StateStore::open(directory.path().join("state.db")).unwrap());
    let (main, owner) = workspace(cx, store.clone());
    let (other, other_owner) = workspace(cx, store.clone());
    let (settings, _) = open(cx, main, &owner);
    assert_eq!(open(cx, other, &other_owner).0, settings);
    cx.update_window(settings, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("settings-window-save").is_none());
        assert!(window.try_find("settings-window-cancel").is_none());
        window.click("settings-show-full-path", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(!owner.read(cx).app_settings.show_full_path);
        assert!(!other_owner.read(cx).app_settings.show_full_path);
    });
    assert!(
        store.load_app_settings().unwrap().show_full_path,
        "continuous edits are debounced before writing"
    );
    settle(cx);
    assert!(!store.load_app_settings().unwrap().show_full_path);
    cx.update(|cx| {
        assert!(
            cx.global::<WorkspaceWindowRegistry>()
                .settings_window
                .is_some()
        )
    });
    cx.update_window(main, |_, window, cx| {
        owner.update(cx, |owner, cx| {
            owner.update_app_setting(|settings| settings.log_font_size = 20, window, cx)
        });
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(settings, |_, window, cx| window.press("escape", cx))
        .unwrap();
    cx.run_until_parked();
    let persisted = store.load_app_settings().unwrap();
    assert!(!persisted.show_full_path);
    assert_eq!(persisted.log_font_size, 20);
}

#[gpui_kit::test]
fn closing_before_debounce_flushes_the_change(cx: &mut TestAppContext) {
    init(cx);
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(StateStore::open(directory.path().join("state.db")).unwrap());
    let (main, owner) = workspace(cx, store.clone());
    let (settings, _) = open(cx, main, &owner);
    cx.update_window(settings, |_, window, cx| {
        window.render_frame(cx);
        window.click("settings-show-full-path", cx);
        window.press("secondary-w", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(!store.load_app_settings().unwrap().show_full_path);
    cx.update(|cx| {
        assert!(
            cx.global::<WorkspaceWindowRegistry>()
                .settings_window
                .is_none()
        )
    });
}

#[gpui_kit::test]
fn edits_during_an_active_save_are_not_lost(cx: &mut TestAppContext) {
    init(cx);
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(StateStore::open(directory.path().join("state.db")).unwrap());
    let (main, owner) = workspace(cx, store.clone());
    let (settings, view) = open(cx, main, &owner);
    cx.update_window(settings, |_, window, cx| {
        window.render_frame(cx);
        window.click("settings-show-full-path", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(settings, |_, window, cx| {
        view.update(cx, |view, cx| view.save(window, cx));
        assert!(view.read(cx).saving);
        window.render_frame(cx);
        window.click("settings-show-full-path", cx);
    })
    .unwrap();
    settle(cx);
    assert!(store.load_app_settings().unwrap().show_full_path);
    cx.update(|cx| assert!(owner.read(cx).app_settings.show_full_path));
}

#[gpui_kit::test]
fn storage_failure_keeps_changes_and_allows_retry(cx: &mut TestAppContext) {
    init(cx);
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(StateStore::open(directory.path().join("state.db")).unwrap());
    let (main, owner) = workspace(cx, store.clone());
    let (settings, view) = open(cx, main, &owner);
    cx.update(|cx| owner.update(cx, |owner, _| owner.persistence.store = None));
    cx.update_window(settings, |_, window, cx| {
        window.render_frame(cx);
        window.click("settings-show-full-path", cx);
    })
    .unwrap();
    settle(cx);
    cx.update(|cx| assert!(view.read(cx).save_error.is_some()));
    cx.update_window(settings, |_, window, cx| window.press("escape", cx))
        .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(
            cx.global::<WorkspaceWindowRegistry>()
                .settings_window
                .is_some()
        );
        owner.update(cx, |owner, _| owner.persistence.store = Some(store.clone()));
    });
    cx.update_window(settings, |_, window, cx| {
        window.render_frame(cx);
        window.click("settings-retry-save", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(!store.load_app_settings().unwrap().show_full_path);
    cx.update(|cx| assert!(view.read(cx).save_error.is_none()));
}

#[gpui_kit::test]
async fn quitting_flushes_debounced_settings(cx: &mut TestAppContext) {
    init(cx);
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(StateStore::open(directory.path().join("state.db")).unwrap());
    let (main, owner) = workspace(cx, store.clone());
    let (settings, _) = open(cx, main, &owner);
    cx.update_window(settings, |_, window, cx| {
        window.render_frame(cx);
        window.click("settings-show-full-path", cx);
    })
    .unwrap();
    cx.run_until_parked();
    let flush = cx.update(Workspace::flush_all_on_quit);
    flush.await;
    assert!(!store.load_app_settings().unwrap().show_full_path);
}

#[gpui_kit::test]
fn settings_keep_saving_after_the_source_window_closes(cx: &mut TestAppContext) {
    init(cx);
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(StateStore::open(directory.path().join("state.db")).unwrap());
    let (main, owner) = workspace(cx, store.clone());
    let (settings, _) = open(cx, main, &owner);
    cx.update_window(main, |_, window, cx| {
        Workspace::unregister_window(window.window_handle().window_id(), cx);
        window.remove_window();
    })
    .unwrap();
    cx.update_window(settings, |_, window, cx| {
        window.render_frame(cx);
        window.click("settings-show-full-path", cx);
    })
    .unwrap();
    settle(cx);
    assert!(!store.load_app_settings().unwrap().show_full_path);
}

#[gpui_kit::test]
fn highlight_changes_are_saved_without_locking_the_editor(cx: &mut TestAppContext) {
    init(cx);
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(StateStore::open(directory.path().join("state.db")).unwrap());
    let (main, owner) = workspace(cx, store.clone());
    let (settings, view) = open(cx, main, &owner);
    let original = cx.update(|cx| owner.read(cx).app_settings.highlight_log_levels);
    cx.update_window(settings, |_, window, cx| {
        view.read(cx).settings.clone().update(cx, |editor, cx| {
            editor.select_category(SettingsCategory::Highlight, window, cx)
        });
        window.render_frame(cx);
        window.click("log-coloring-enabled", cx);
    })
    .unwrap();
    settle(cx);
    assert_eq!(
        store.load_app_settings().unwrap().highlight_log_levels,
        !original
    );
    cx.update(|cx| assert!(view.read(cx).save_error.is_none()));
    cx.update_window(settings, |_, window, cx| {
        window.render_frame(cx);
        window.click("log-coloring-enabled", cx);
    })
    .unwrap();
    settle(cx);
    assert_eq!(
        store.load_app_settings().unwrap().highlight_log_levels,
        original
    );
}

#[gpui_kit::test]
fn network_text_is_saved_without_connecting(cx: &mut TestAppContext) {
    init(cx);
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(StateStore::open(directory.path().join("state.db")).unwrap());
    let (main, owner) = workspace(cx, store.clone());
    let (settings, view) = open(cx, main, &owner);
    cx.update_window(settings, |_, window, cx| {
        view.read(cx).settings.clone().update(cx, |editor, cx| {
            editor.select_category(SettingsCategory::Network, window, cx)
        });
        window.render_frame(cx);
        window.click("settings-network-server", cx);
        window.press("secondary-a", cx);
        window.input("https://settings.example.com", cx);
    })
    .unwrap();
    settle(cx);
    assert_eq!(
        store.load_cloud_settings().unwrap().server_url,
        "https://settings.example.com"
    );
    cx.update(|cx| assert!(owner.read(cx).cloud.connection.is_none()));
}

#[gpui_kit::test]
fn selection_style_changes_reach_autosave(cx: &mut TestAppContext) {
    init(cx);
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(StateStore::open(directory.path().join("state.db")).unwrap());
    let (main, owner) = workspace(cx, store.clone());
    let (settings, view) = open(cx, main, &owner);
    let original = store.load_app_settings().unwrap().selection_styles;
    cx.update_window(settings, |_, window, cx| {
        view.read(cx).settings.clone().update(cx, |editor, cx| {
            editor.select_category(SettingsCategory::Highlight, window, cx)
        });
        window.render_frame(cx);
        window.within("log-coloring-tabs").click(1usize, cx);
        window.click("selection-underline", cx);
    })
    .unwrap();
    settle(cx);
    assert_ne!(
        store.load_app_settings().unwrap().selection_styles,
        original
    );
}

#[gpui_kit::test]
fn restoring_defaults_keeps_settings_open_and_autosave_usable(cx: &mut TestAppContext) {
    init(cx);
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(StateStore::open(directory.path().join("state.db")).unwrap());
    let (main, owner) = workspace(cx, store.clone());
    let (settings, _) = open(cx, main, &owner);
    cx.update_window(settings, |_, window, cx| {
        window.render_frame(cx);
        window.click("settings-show-full-path", cx);
    })
    .unwrap();
    settle(cx);
    cx.update_window(settings, |_, window, cx| {
        window.render_frame(cx);
        window.click("settings-window-reset", cx);
        window.render_frame(cx);
        window.click("settings-reset-confirm", cx);
    })
    .unwrap();
    cx.run_until_parked();
    assert!(store.load_app_settings().unwrap().show_full_path);
    cx.update_window(settings, |_, window, cx| {
        window.render_frame(cx);
        window.click("settings-show-full-path", cx);
    })
    .unwrap();
    settle(cx);
    assert!(!store.load_app_settings().unwrap().show_full_path);
}

#[gpui_kit::test]
fn clearing_search_history_requires_confirmation_then_autosaves(cx: &mut TestAppContext) {
    init(cx);
    let directory = tempfile::tempdir().unwrap();
    let store = Arc::new(StateStore::open(directory.path().join("state.db")).unwrap());
    store.save_search_history(&["example".to_owned()]).unwrap();
    let (main, owner) = workspace(cx, store.clone());
    cx.update(|cx| {
        owner.update(cx, |owner, _| {
            owner.search_history = vec!["example".to_owned()]
        })
    });
    let (settings, view) = open(cx, main, &owner);
    cx.update_window(settings, |_, window, cx| {
        view.read(cx).settings.clone().update(cx, |editor, cx| {
            editor.select_category(SettingsCategory::History, window, cx)
        });
        window.render_frame(cx);
        window.click("settings-search-history-tab", cx);
        window.click("settings-search-history-clear", cx);
        window.render_frame(cx);
        window.click("settings-history-cancel", cx);
    })
    .unwrap();
    settle(cx);
    assert_eq!(store.load_search_history().unwrap(), ["example"]);
    cx.update_window(settings, |_, window, cx| {
        window.render_frame(cx);
        window.click("settings-search-history-clear", cx);
        window.render_frame(cx);
        window.click("settings-history-clear-confirm", cx);
    })
    .unwrap();
    settle(cx);
    assert!(store.load_search_history().unwrap().is_empty());
}
