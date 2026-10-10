use super::*;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{InputEvent as _, TestAppContext};

#[gpui_kit::test]
fn file_tabs_reorder_outside_strip_and_fly_back_before_restoring(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::actions::init(cx);
        Workspace::init_window_registry(cx);
        crate::notifications::init(cx);
        crate::app_icon::init(cx);
    });
    let mut workspace = None;
    let handle = cx.open_window(size(px(1400.), px(900.)), |window, cx| {
        let view = cx.new(|cx| Workspace::new(false, Vec::new(), window, cx));
        view.update(cx, |view, cx| {
            view.persistence._bootstrap_task = Task::ready(());
            view.persistence.state_tasks.clear();
            view._cloud_client_bootstrap_task = Task::ready(());
            view.create_new_tab(window, cx);
            view.create_new_tab(window, cx);
        });
        Workspace::register_window(&view, window, cx);
        workspace = Some(view.clone());
        gpui_kit::component::Root::new(view, window, cx)
    });
    let workspace = workspace.unwrap();
    let mut dragged_id = None;
    let mut release_y = px(0.);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let view = workspace.read(cx);
        let id = *view.tabs.last().unwrap();
        dragged_id = Some(id);
        let source = view.tab_drag.file_bounds.borrow()[&id];
        let to = view.tab_drop_layout.borrow().tabs[0].center();
        let from = source.center();
        window.dispatch_event(
            MouseDownEvent {
                position: from,
                button: MouseButton::Left,
                modifiers: Default::default(),
                click_count: 1,
                first_mouse: false,
            }
            .to_platform_input(),
            cx,
        );
        for fraction in [0.15, 0.5, 1.] {
            window.dispatch_event(
                MouseMoveEvent {
                    position: point(
                        from.x + (to.x - from.x) * fraction,
                        if fraction == 0.15 {
                            from.y
                        } else {
                            from.y + px(140.)
                        },
                    ),
                    pressed_button: Some(MouseButton::Left),
                    modifiers: Default::default(),
                }
                .to_platform_input(),
                cx,
            );
            window.render_frame(cx);
        }
        assert_eq!(
            workspace.read(cx).tabs[0],
            id,
            "sort before mouse release, outside the strip"
        );
        assert_eq!(workspace.read(cx).tab_drag.file_hidden, Some(id));
        assert!(
            workspace
                .read(cx)
                .tab_drag
                .file_offsets
                .values()
                .any(|offset| *offset != px(0.))
        );
        let WorkspaceTabId::New(new_id) = id else {
            unreachable!()
        };
        let preview_id = ElementId::from(("new-tab-drag-preview", new_id));
        let preview = window.find(preview_id.clone()).bounds();
        assert_eq!(preview.size, source.size);
        window.dispatch_event(
            MouseUpEvent {
                position: point(to.x, from.y + px(140.)),
                button: MouseButton::Left,
                modifiers: Default::default(),
                click_count: 1,
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
        assert!(workspace.read(cx).tab_drag.flight.is_some());
        assert_eq!(workspace.read(cx).tab_drag.file_hidden, Some(id));
        let flight = window.find(preview_id).bounds();
        assert_eq!(
            flight.origin, preview.origin,
            "the return flight must start without jumping"
        );
        release_y = flight.top();
    })
    .unwrap();
    cx.run_until_parked();
    for _ in 0..5 {
        cx.background_executor
            .advance_clock(Duration::from_millis(16));
        cx.run_until_parked();
    }
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let WorkspaceTabId::New(id) = dragged_id.unwrap() else {
            unreachable!()
        };
        let flight = window
            .find(ElementId::from(("new-tab-drag-preview", id)))
            .bounds();
        let target = workspace.read(cx).tab_drop_layout.borrow().tabs[0];
        assert!(
            flight.top() < release_y && flight.top() > target.top(),
            "preview should be partway home"
        );
        assert_eq!(workspace.read(cx).tab_drag.file_hidden, dragged_id);
    })
    .unwrap();
    for _ in 0..5 {
        cx.background_executor
            .advance_clock(Duration::from_millis(16));
        cx.run_until_parked();
    }
    cx.update(|cx| {
        assert!(workspace.read(cx).tab_drag.flight.is_none());
        assert!(workspace.read(cx).tab_drag.file_hidden.is_none());
    });
    // Cancellation and reduced motion must never leave a hidden source tab behind.
    for cancel in [true, false] {
        cx.update_window(handle.into(), |_, window, cx| {
            cx.set_reduce_motion(!cancel);
            window.render_frame(cx);
            let id = dragged_id.unwrap();
            let from = workspace.read(cx).tab_drag.file_bounds.borrow()[&id].center();
            window.dispatch_event(
                MouseDownEvent {
                    position: from,
                    button: MouseButton::Left,
                    modifiers: Default::default(),
                    click_count: 1,
                    first_mouse: false,
                }
                .to_platform_input(),
                cx,
            );
            let to = point(from.x + px(10.), from.y);
            window.dispatch_event(
                MouseMoveEvent {
                    position: to,
                    pressed_button: Some(MouseButton::Left),
                    modifiers: Default::default(),
                }
                .to_platform_input(),
                cx,
            );
            window.render_frame(cx);
            assert_eq!(workspace.read(cx).tab_drag.file_hidden, Some(id));
            if cancel {
                window.press("escape", cx);
            } else {
                window.dispatch_event(
                    MouseUpEvent {
                        position: to,
                        button: MouseButton::Left,
                        modifiers: Default::default(),
                        click_count: 1,
                    }
                    .to_platform_input(),
                    cx,
                );
            }
            assert!(workspace.read(cx).tab_drag.file_hidden.is_none());
            assert!(workspace.read(cx).tab_drag.flight.is_none());
            assert!(!cx.has_active_drag());
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn file_tabs_close_without_delaying_removal(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::actions::init(cx);
        Workspace::init_window_registry(cx);
        crate::notifications::init(cx);
        crate::app_icon::init(cx);
    });
    let mut workspace = None;
    let handle = cx.open_window(size(px(1400.), px(900.)), |window, cx| {
        let view = cx.new(|cx| Workspace::new(false, Vec::new(), window, cx));
        view.update(cx, |view, _| {
            view.persistence._bootstrap_task = Task::ready(());
            view.persistence.state_tasks.clear();
            view._cloud_client_bootstrap_task = Task::ready(());
        });
        Workspace::register_window(&view, window, cx);
        workspace = Some(view.clone());
        gpui_kit::component::Root::new(view, window, cx)
    });
    let workspace = workspace.unwrap();
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("new-workspace-tab", cx);
        window.click("new-workspace-tab", cx);
        let view = workspace.read(cx);
        assert_eq!(view.tabs.len(), 3);
    })
    .unwrap();
    cx.run_until_parked();
    let mut initial_x = px(0.);
    let mut middle_x = px(0.);
    let mut last_id = WorkspaceTabId::New(0);
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let view = workspace.read(cx);
        let WorkspaceTabId::New(id) = view.tabs[1] else {
            unreachable!()
        };
        last_id = view.tabs[2];
        initial_x = view.tab_drag.file_bounds.borrow()[&last_id].left();
        window.click(ElementId::from(("close-new-tab", id)), cx);
        let view = workspace.read(cx);
        assert_eq!(view.tabs.len(), 2);
        assert!(!view.tabs.contains(&WorkspaceTabId::New(id)));
        assert_eq!(view.tab_motion.closing.len(), 1);
        assert_eq!(
            view.tab_drag.file_bounds.borrow()[&last_id].left(),
            initial_x
        );
    })
    .unwrap();
    cx.run_until_parked();
    for _ in 0..5 {
        cx.background_executor
            .advance_clock(Duration::from_millis(16));
        cx.run_until_parked();
    }
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        middle_x = workspace.read(cx).tab_drag.file_bounds.borrow()[&last_id].left();
        assert!(middle_x < initial_x);
        // Closing another tab mid-animation must preserve both visual slots.
        let WorkspaceTabId::New(id) = workspace.read(cx).tabs[0] else {
            unreachable!()
        };
        window.click(ElementId::from(("close-new-tab", id)), cx);
        assert_eq!(workspace.read(cx).tab_motion.closing.len(), 2);
        assert_eq!(workspace.read(cx).workspace_tab_visual_index(0), 2);
    })
    .unwrap();
    for _ in 0..10 {
        cx.background_executor
            .advance_clock(Duration::from_millis(16));
        cx.run_until_parked();
    }
    cx.update_window(handle.into(), |_, window, cx| {
        window.render_frame(cx);
        let view = workspace.read(cx);
        assert!(view.tab_motion.closing.is_empty());
        assert_eq!(view.tabs, vec![last_id]);
        assert!(view.tab_drag.file_bounds.borrow()[&last_id].left() < middle_x);
        cx.set_reduce_motion(true);
        let WorkspaceTabId::New(id) = last_id else {
            unreachable!()
        };
        window.click(ElementId::from(("close-new-tab", id)), cx);
        let view = workspace.read(cx);
        assert!(view.tab_motion.closing.is_empty());
        assert_eq!(
            view.tabs.len(),
            1,
            "closing the last tab creates a blank tab"
        );
        assert_ne!(view.tabs[0], last_id);
    })
    .unwrap();
}

fn init_drag_test(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::actions::init(cx);
        Workspace::init_window_registry(cx);
        crate::notifications::init(cx);
        crate::app_icon::init(cx);
        cx.on_window_closed(|cx, id| Workspace::unregister_window(id, cx))
            .detach();
    });
}

fn drag_workspace(
    cx: &mut TestAppContext,
    paths: &[PathBuf],
    origin: Point<Pixels>,
) -> (AnyWindowHandle, Entity<Workspace>) {
    cx.update(|cx| {
        let handle = crate::open_workspace_window_at(
            cx,
            false,
            Vec::new(),
            Bounds::new(origin, size(px(1200.), px(800.))),
            None,
        )
        .unwrap();
        let workspace = cx
            .global::<WorkspaceWindowRegistry>()
            .windows
            .last()
            .unwrap()
            .workspace
            .clone();
        handle
            .update(cx, |_, window, cx| {
                workspace.update(cx, |this, cx| {
                    this.persistence._bootstrap_task = Task::ready(());
                    this.persistence.state_tasks.clear();
                    this._cloud_client_bootstrap_task = Task::ready(());
                    this.install_documents(
                        paths
                            .iter()
                            .map(|path| {
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
                            .collect(),
                        paths.first().map(PathBuf::as_path),
                        &BTreeMap::new(),
                        None,
                        true,
                        window,
                        cx,
                    );
                });
                window.render_frame(cx);
            })
            .unwrap();
        (handle, workspace)
    })
}

fn drag_move(window: &mut Window, position: Point<Pixels>, copy: bool, cx: &mut App) {
    window.dispatch_event(
        MouseMoveEvent {
            position,
            pressed_button: Some(MouseButton::Left),
            modifiers: gpui_kit::Modifiers {
                control: copy,
                ..Default::default()
            },
        }
        .to_platform_input(),
        cx,
    );
    window.render_frame(cx);
}

fn start_document_drag(
    workspace: &Entity<Workspace>,
    id: u64,
    window: &mut Window,
    cx: &mut App,
) -> Point<Pixels> {
    let from =
        workspace.read(cx).tab_drag.file_bounds.borrow()[&WorkspaceTabId::Document(id)].center();
    window.dispatch_event(
        MouseDownEvent {
            position: from,
            button: MouseButton::Left,
            modifiers: Default::default(),
            click_count: 1,
            first_mouse: false,
        }
        .to_platform_input(),
        cx,
    );
    let position = from + point(px(12.), px(0.));
    drag_move(window, position, false, cx);
    let preview = window
        .find(ElementId::from(("document-tab-drag-preview", id)))
        .bounds();
    position - preview.origin
}

#[gpui_kit::test]
fn document_moves_between_windows_and_detaches_in_one_gesture(cx: &mut TestAppContext) {
    move_document_between_windows(cx, false);
}

#[gpui_kit::test]
fn entering_window_body_transfers_immediately_without_creating_a_window(cx: &mut TestAppContext) {
    move_document_between_windows(cx, true);
}

fn move_document_between_windows(cx: &mut TestAppContext, enter_body: bool) {
    init_drag_test(cx);
    let directory = tempfile::tempdir().unwrap();
    let a = directory.path().join("a.log");
    let b = directory.path().join("b.log");
    for path in [&a, &b] {
        std::fs::write(path, "first\nsecond\nthird\n").unwrap();
    }
    let (source_window, source) =
        drag_workspace(cx, std::slice::from_ref(&a), point(px(0.), px(0.)));
    let (target_window, target) = drag_workspace(cx, &[b], point(px(1300.), px(0.)));
    let mut floating_window = None;
    cx.update_window(source_window, |_, window, cx| {
        let source_id = source.read(cx).documents[0].id;
        let original_document = source.read(cx).documents[0].document.clone();
        source.update(cx, |this, _| {
            let tab = &mut this.documents[0];
            tab.file.custom_title = Some("my log".into());
            tab.view.word_wrap = true;
            tab.file.marked_rows.insert(1);
        });
        let offset = start_document_drag(&source, source_id, window, cx);
        let target_point = target_window
            .update(cx, |_, window, cx| {
                window.render_frame(cx);
                let layout = target.read(cx).tab_drop_layout.borrow();
                let local = if enter_body {
                    let position = point(px(450.), px(400.));
                    assert!(layout.drop_index(position).is_none());
                    position
                } else {
                    layout.tabs[0].center()
                };
                window.bounds().origin + local
            })
            .unwrap();
        drag_move(window, target_point, false, cx);
        assert!(
            source.read(cx).documents.is_empty(),
            "move immediately, before mouse up"
        );
        assert_eq!(target.read(cx).documents.len(), 2);
        assert_eq!(
            cx.global::<WorkspaceWindowRegistry>().windows.len(),
            2,
            "entering any part of the target must not create a floating shell"
        );
        let moved = target.read(cx).active_document().unwrap();
        assert!(
            Arc::ptr_eq(&original_document, &moved.document),
            "reuse the loaded document"
        );
        assert_eq!(moved.file.custom_title.as_deref(), Some("my log"));
        assert!(moved.view.word_wrap);
        assert!(moved.file.marked_rows.contains(1));
        let target_id = moved.id;
        assert_ne!(
            target_id, source_id,
            "document ids are local to each workspace"
        );
        assert!(cx.has_active_drag());
        target_window
            .update(cx, |_, window, cx| {
                window.render_frame(cx);
                let preview = window
                    .find(ElementId::from(("document-tab-drag-preview", target_id)))
                    .bounds();
                assert_eq!(
                    target_point - window.bounds().origin - preview.origin,
                    offset
                );
            })
            .unwrap();
        let moved_point = target_point + point(px(45.), px(0.));
        drag_move(window, moved_point, false, cx);
        target_window
            .update(cx, |_, window, cx| {
                window.render_frame(cx);
                let preview = window
                    .find(ElementId::from(("document-tab-drag-preview", target_id)))
                    .bounds();
                assert_eq!(
                    moved_point - window.bounds().origin - preview.origin,
                    offset,
                    "same grab point after transfer"
                );
            })
            .unwrap();
        window.render_frame(cx);
        let back = if enter_body {
            point(px(450.), px(400.))
        } else {
            source.read(cx).tab_drop_layout.borrow().tabs[0].center()
        };
        drag_move(window, back, false, cx);
        assert_eq!(
            source.read(cx).documents.len(),
            1,
            "move back without releasing"
        );
        assert_eq!(target.read(cx).documents.len(), 1);
        assert!(cx.has_active_drag());
        // A gap between windows creates a floating workspace and keeps the gesture alive.
        let outside = point(px(500.), px(950.));
        drag_move(window, outside, false, cx);
        let registry = cx.global::<WorkspaceWindowRegistry>();
        assert_eq!(registry.windows.len(), 3);
        let drag = registry.cross_window_tab_drag.as_ref().unwrap();
        let floating = drag.target.as_ref().unwrap().window;
        floating_window = Some(floating);
        let first_origin = drag.floating.unwrap().origin;
        assert!(cx.has_active_drag());
        drag_move(window, outside + point(px(35.), px(25.)), false, cx);
        let drag = cx
            .global::<WorkspaceWindowRegistry>()
            .cross_window_tab_drag
            .as_ref()
            .unwrap();
        assert_eq!(
            drag.floating.unwrap().origin,
            first_origin + point(px(35.), px(25.))
        );
        assert_eq!(
            cx.global::<WorkspaceWindowRegistry>().windows.len(),
            3,
            "do not create another shell"
        );
        drag_move(window, target_point, false, cx);
        assert_eq!(
            target.read(cx).documents.len(),
            2,
            "dock the floating document before release"
        );
        assert!(
            cx.global::<WorkspaceWindowRegistry>()
                .cross_window_tab_drag
                .as_ref()
                .unwrap()
                .floating
                .is_none()
        );
        let current_id = target.read(cx).active_document().unwrap().id;
        window.dispatch_event(
            MouseUpEvent {
                position: target_point,
                button: MouseButton::Left,
                modifiers: Default::default(),
                click_count: 1,
            }
            .to_platform_input(),
            cx,
        );
        assert!(!cx.has_active_drag());
        assert!(
            cx.global::<WorkspaceWindowRegistry>()
                .cross_window_tab_drag
                .is_none()
        );
        assert_eq!(
            target.read(cx).active_tab_id,
            WorkspaceTabId::Document(current_id)
        );
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(
            cx.global::<WorkspaceWindowRegistry>()
                .windows
                .iter()
                .all(|entry| Some(entry.window) != floating_window),
            "remove the empty floating shell after docking"
        );
    });
}

#[gpui_kit::test]
fn control_drag_copies_once_and_escape_settles_the_current_owner(cx: &mut TestAppContext) {
    init_drag_test(cx);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("copy.log");
    std::fs::write(&path, "first\nsecond\n").unwrap();
    let (handle, source) = drag_workspace(cx, &[path], point(px(0.), px(0.)));
    let (target_handle, target) = drag_workspace(cx, &[], point(px(1300.), px(0.)));
    cx.update_window(handle, |_, window, cx| {
        let id = source.read(cx).documents[0].id;
        start_document_drag(&source, id, window, cx);
        let position = target_handle
            .update(cx, |_, window, cx| {
                window.bounds().origin + target.read(cx).tab_drop_layout.borrow().tabs[0].center()
            })
            .unwrap();
        drag_move(window, position, true, cx);
        assert_eq!(source.read(cx).documents.len(), 1);
        assert_eq!(target.read(cx).documents.len(), 1);
        drag_move(window, point(px(600.), px(950.)), true, cx);
        assert!(
            target.read(cx).documents.is_empty(),
            "after copying once, move the dragged copy"
        );
        assert_eq!(source.read(cx).documents.len(), 1);
        window.press("escape", cx);
        assert!(!cx.has_active_drag());
        assert!(
            cx.global::<WorkspaceWindowRegistry>()
                .cross_window_tab_drag
                .is_none()
        );
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            cx.global::<WorkspaceWindowRegistry>().windows.len(),
            3,
            "Escape keeps the last valid window"
        );
        assert!(
            cx.global::<WorkspaceWindowRegistry>()
                .windows
                .iter()
                .all(|entry| entry.workspace.read(cx).tab_drag.file_hidden.is_none())
        );
    });
}

#[gpui_kit::test]
fn duplicate_target_keeps_both_documents_and_drag_can_be_cancelled(cx: &mut TestAppContext) {
    init_drag_test(cx);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("same.log");
    std::fs::write(&path, "first\nsecond\n").unwrap();
    let (handle, source) = drag_workspace(cx, std::slice::from_ref(&path), point(px(0.), px(0.)));
    let (target_handle, target) = drag_workspace(cx, &[path], point(px(1300.), px(0.)));
    cx.update_window(handle, |_, window, cx| {
        let id = source.read(cx).documents[0].id;
        let target_document = target.read(cx).documents[0].document.clone();
        start_document_drag(&source, id, window, cx);
        let position = target_handle
            .update(cx, |_, window, cx| {
                window.bounds().origin + target.read(cx).tab_drop_layout.borrow().tabs[0].center()
            })
            .unwrap();
        drag_move(window, position, false, cx);
        assert_eq!(source.read(cx).documents.len(), 1);
        assert_eq!(target.read(cx).documents.len(), 1);
        assert!(Arc::ptr_eq(
            &target_document,
            &target.read(cx).documents[0].document
        ));
        assert!(cx.has_active_drag());
        window.press("escape", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(!cx.has_active_drag());
        assert!(source.read(cx).tab_drag.file_hidden.is_none());
        assert!(target.read(cx).tab_drag.file_hidden.is_none());
        assert_eq!(cx.global::<WorkspaceWindowRegistry>().windows.len(), 2);
    });
}

#[gpui_kit::test]
fn closing_pointer_capture_window_settles_transferred_tab(cx: &mut TestAppContext) {
    init_drag_test(cx);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("close.log");
    std::fs::write(&path, "first\nsecond\n").unwrap();
    let (handle, source) = drag_workspace(cx, &[path], point(px(0.), px(0.)));
    let (target_handle, target) = drag_workspace(cx, &[], point(px(1300.), px(0.)));
    cx.update_window(handle, |_, window, cx| {
        let id = source.read(cx).documents[0].id;
        start_document_drag(&source, id, window, cx);
        let position = target_handle
            .update(cx, |_, window, cx| {
                window.bounds().origin + target.read(cx).tab_drop_layout.borrow().tabs[0].center()
            })
            .unwrap();
        drag_move(window, position, false, cx);
        assert_eq!(target.read(cx).documents.len(), 1);
        window.remove_window();
    })
    .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(!cx.has_active_drag());
        assert!(
            cx.global::<WorkspaceWindowRegistry>()
                .cross_window_tab_drag
                .is_none()
        );
        assert!(target.read(cx).tab_drag.file_hidden.is_none());
        assert_eq!(target.read(cx).documents.len(), 1);
    });
}
