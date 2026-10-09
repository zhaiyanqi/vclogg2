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
fn file_tabs_fade_in_and_close_without_delaying_removal(cx: &mut TestAppContext) {
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
        assert_eq!(view.workspace_tab_opacity(view.tabs[2]), 0.);
    })
    .unwrap();
    cx.run_until_parked();
    for frame in 1..=10 {
        cx.background_executor
            .advance_clock(Duration::from_millis(16));
        cx.run_until_parked();
        cx.update(|cx| {
            let view = workspace.read(cx);
            let opacity = view.workspace_tab_opacity(view.tabs[2]);
            if frame < 10 {
                assert!(opacity > 0. && opacity < 1.);
            } else {
                assert_eq!(opacity, 1.);
            }
        });
    }
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
        assert_eq!(view.workspace_tab_opacity(view.tabs[0]), 1.);
    })
    .unwrap();
}
