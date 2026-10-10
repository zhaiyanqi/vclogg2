use super::*;
use crate::virtual_log_list::VirtualLogListPosition;

fn test_viewport(word_wrap: bool, count: usize) -> LogViewportState<usize> {
    let viewport = VirtualLogViewport::new();
    viewport.set_item_count(count, px(20.));
    LogViewportState::new(word_wrap, viewport, Rc::default())
}

#[test]
fn new_search_preserves_row_bookmarks_instead_of_short_result_bottom() {
    use super::search_tabs::SearchTabState;
    use crate::search_context::{PersistedSearchRowKey, PersistedSearchTab};

    let mut saved = PersistedSearchTab {
        created_at: Some("2026-10-09 14:30:05".into()),
        ..Default::default()
    };
    saved.local.selected_source_row = Some(7_000);
    saved.local.viewport = Some(ViewportBookmark::new(7_000, 40., 12., true));
    saved.context.viewport = Some(PersistedSearchViewport::new(
        PersistedSearchRowKey {
            path: "test.log".into(),
            source_row: Some(7_000),
        },
        40.,
        12.,
        true,
        2,
    ));
    let mut state = SearchTabState::restored(saved.clone());

    state.prepare_result_viewport(true);
    assert_eq!(state.saved, saved, "session restoration retains its bottom");

    state.prepare_result_viewport(false);
    saved.local.viewport.as_mut().unwrap().at_end = false;
    saved.context.viewport.as_mut().unwrap().at_end = false;
    assert_eq!(
        state.saved, saved,
        "new search retains row, Y and selection"
    );

    for word_wrap in [false, true] {
        let viewport = test_viewport(word_wrap, 3);
        viewport.scroll_to_end();
        let bookmark = state.saved.local.viewport.unwrap();
        // The selected source row was result #3; the broadened query now includes
        // every source row, moving it to result #7001 without changing its identity.
        viewport.restore_viewport(
            bookmark.anchor_source_row,
            px(bookmark.anchor_viewport_y()),
            bookmark.at_end,
            px(20.),
        );
        let range = viewport.requested_row_range(10_000, px(200.), px(20.));
        assert!(range.contains(&7_000));
        assert!(
            !range.contains(&9_999),
            "new results must not jump to the bottom"
        );
    }
}

#[test]
fn visible_selected_anchor_does_not_inherit_bottom_following() {
    assert!(!Workspace::viewport_anchor_retains_end(true, Some(2), 2));
    assert!(Workspace::viewport_anchor_retains_end(true, Some(50), 2));
    assert!(Workspace::viewport_anchor_retains_end(true, None, 0));
}

fn wrapped_layout_key_for_test() -> WrappedLayoutKey {
    WrappedLayoutKey {
        content_revision: 1,
        width: px(640.),
        rem_size: px(16.),
        font_family: "Consolas".into(),
        font_size: 13,
        base_height: px(19.),
        horizontal_padding: px(8.),
    }
}

#[test]
fn mode_switch_keeps_the_same_row_anchor_and_scroll_owner() {
    let viewport = test_viewport(false, 10_000_000);
    viewport.viewport().set_position(VirtualLogListPosition {
        row_ix: 8_000_000,
        offset_in_row: px(7.),
    });
    let owner = viewport.viewport().clone();

    viewport.set_word_wrap(true);

    assert_eq!(owner.position().row_ix, 8_000_000);
    assert_eq!(viewport.viewport().position().row_ix, 8_000_000);
    assert_eq!(viewport.viewport().position().offset_in_row, px(7.));
    assert!(viewport.is_wrapped());
}

#[test]
fn pixel_scroll_browses_inside_a_long_visible_row() {
    let viewport = test_viewport(true, 3);
    viewport.viewport().record_indexed_height(0, px(120.));

    let target = viewport.wheel_scroll_target(
        Point::default(),
        LogWheelScrollRequest {
            delta_y: px(-70.),
            row_count: 3,
            row_height: px(20.),
            line_count: 3,
            line_scroll: false,
            scale: 1.,
        },
    );

    let target = target.expect("wheel input should produce a future viewport target");
    assert_eq!(viewport.viewport().position().offset_in_row, px(0.));
    viewport.commit_scroll_frame_target(LogScrollFrameTarget::Viewport(target), 3, px(20.));
    assert_eq!(viewport.viewport().position().row_ix, 0);
    assert_eq!(viewport.viewport().position().offset_in_row, px(70.));
}

#[test]
fn upward_scroll_uses_minimum_height_for_an_unknown_previous_row() {
    let viewport = test_viewport(true, 3);
    viewport.viewport().set_position(VirtualLogListPosition {
        row_ix: 2,
        offset_in_row: px(0.),
    });

    viewport.viewport().scroll_by_pixels(px(-25.));

    assert_eq!(viewport.viewport().position().row_ix, 0);
    assert_eq!(viewport.viewport().position().offset_in_row, px(15.));
}

#[test]
fn logical_scrollbar_is_buffered_until_visible_data_is_prepared() {
    let viewport = test_viewport(false, 100);
    let requested = point(px(0.), px(-420.));

    viewport
        .wrapped_logical_scroll_handle(100, px(20.))
        .set_offset(requested);

    assert_eq!(viewport.viewport().position().row_ix, 0);
    assert_eq!(viewport.take_pending_scrollbar_offset(), Some(requested));
}

#[test]
fn mode_switch_consumes_the_latest_scrollbar_target() {
    let viewport = test_viewport(false, 100);
    let key = (7, WrappedRegion::Log);
    let mut pending = PendingLogScrollFrames::default();
    pending.request(
        key,
        LogScrollFrameTarget::Viewport(point(px(0.), px(-300.))),
    );
    let requested = point(px(0.), px(-1_200.));
    viewport
        .wrapped_logical_scroll_handle(100, px(20.))
        .set_offset(requested);

    assert_eq!(
        take_pending_log_scroll_target(&mut pending, key, &viewport),
        Some(LogScrollFrameTarget::Scrollbar(requested))
    );
    assert_eq!(pending.latest(key), None);
}

#[test]
fn layout_change_invalidates_only_the_bounded_measurement_cache() {
    let viewport = test_viewport(true, 100);
    viewport.viewport().record_measured_height(
        40,
        LogRowKey::Row {
            document_id: 1,
            source_row: 40,
        },
        px(60.),
    );
    assert!(viewport.has_known_wrapped_row_height(40));

    assert!(viewport.invalidate_wrapped_layout_preserving_position(
        wrapped_layout_key_for_test(),
        Some(40),
    ));

    assert!(!viewport.has_known_wrapped_row_height(40));
    assert_eq!(viewport.viewport().position().row_ix, 0);
}

#[test]
fn subpixel_width_noise_keeps_measurements() {
    let viewport = test_viewport(true, 100);
    let current = wrapped_layout_key_for_test();
    assert!(viewport.invalidate_wrapped_layout_preserving_position(current.clone(), None));
    viewport.viewport().record_indexed_height(4, px(60.));

    let mut next = current;
    next.width += px(0.25);

    assert!(!viewport.invalidate_wrapped_layout_preserving_position(next, None));
    assert!(viewport.has_known_wrapped_row_height(4));
}

#[test]
fn measured_heights_follow_stable_keys_after_projection_changes() {
    let mut viewport = test_viewport(true, 3);
    let key = LogRowKey::Row {
        document_id: 7,
        source_row: 20,
    };
    viewport.viewport().record_measured_height(1, key, px(60.));

    viewport.reset_wrapped_with_remapped_heights(
        3,
        px(20.),
        viewport.viewport().measured_heights(),
        |candidate| (*candidate == key).then_some(0),
    );

    assert_eq!(viewport.effective_row_height(0, px(20.)), px(60.));
    assert_eq!(viewport.effective_row_height(1, px(20.)), px(20.));
}

#[test]
fn hit_testing_uses_the_single_visible_geometry_map() {
    let bounds = Rc::new(RefCell::new(BTreeMap::from([
        (
            2,
            Bounds::new(point(px(0.), px(0.)), size(px(100.), px(20.))),
        ),
        (
            3,
            Bounds::new(point(px(0.), px(20.)), size(px(100.), px(20.))),
        ),
    ])));
    let viewport = LogViewportState::<usize>::new(false, VirtualLogViewport::new(), bounds);

    assert_eq!(viewport.row_at_position(point(px(4.), px(4.))), Some(2));
    assert_eq!(viewport.visible_row_edge(true), Some(3));

    viewport.set_word_wrap(true);
    assert_eq!(viewport.row_at_position(point(px(4.), px(4.))), None);
}

#[test]
fn scrollbar_preload_is_derived_directly_from_the_logical_row_slot() {
    assert_eq!(
        scrollbar_preload_range(point(px(0.), px(-800.)), 100, px(200.), px(20.)),
        40..51
    );
    assert_eq!(
        scrollbar_preload_range(point(px(0.), px(-20_000.)), 100, px(200.), px(20.)),
        89..100
    );
}

#[test]
fn candidate_measurement_range_is_bounded_around_the_anchor() {
    assert_eq!(
        wrapped_viewport_measurement_range(40, px(200.), px(20.), 100),
        40..51
    );
    assert_eq!(
        wrapped_viewport_measurement_range(8_000_000, px(200.), px(20.), 10_000_000),
        8_000_000..8_000_011
    );
}

#[test]
fn pending_scroll_requests_coalesce_to_the_latest_target() {
    let mut pending = PendingLogScrollFrames::default();
    let key = (7, WrappedRegion::Log);
    pending.request(
        key,
        LogScrollFrameTarget::Scrollbar(point(px(0.), px(-100.))),
    );
    pending.request(
        key,
        LogScrollFrameTarget::Scrollbar(point(px(0.), px(-900.))),
    );

    assert_eq!(
        pending.take(key),
        Some(LogScrollFrameTarget::Scrollbar(point(px(0.), px(-900.))))
    );
}

#[test]
fn log_jump_preload_range_covers_target_and_edges() {
    assert_eq!(centered_log_jump_preload_range(50, 100, 10), 35..65);
    assert_eq!(centered_log_jump_preload_range(2, 100, 10), 0..30);
    assert_eq!(centered_log_jump_preload_range(98, 100, 10), 70..100);
    assert_eq!(centered_log_jump_preload_range(0, 0, 10), 0..0);
}

#[test]
fn only_cross_row_line_drag_changes_selection_after_initial_select() {
    let drag = |target_row, mode| RowDragSelection {
        document_id: 1,
        region: WrappedRegion::GlobalResults,
        pointer: Point::default(),
        start_row: 3,
        target_row,
        mode,
    };

    assert!(!drag(3, RowDragMode::Lines).changed_row_selection());
    assert!(!drag(4, RowDragMode::Text).changed_row_selection());
    assert!(drag(4, RowDragMode::Lines).changed_row_selection());
}

#[test]
fn row_drag_only_owns_wheel_events_from_its_log_region() {
    let local_drag = RowDragSelection {
        document_id: 7,
        region: WrappedRegion::Log,
        pointer: Point::default(),
        start_row: 3,
        target_row: 3,
        mode: RowDragMode::Text,
    };
    assert!(local_drag.owns_region(7, WrappedRegion::Log));
    assert!(!local_drag.owns_region(7, WrappedRegion::Results));
    assert!(!local_drag.owns_region(8, WrappedRegion::Log));

    let global_drag = RowDragSelection {
        document_id: 0,
        region: WrappedRegion::GlobalResults,
        pointer: Point::default(),
        start_row: 3,
        target_row: 4,
        mode: RowDragMode::Lines,
    };
    assert!(global_drag.owns_region(99, WrappedRegion::GlobalResults));
    assert!(!global_drag.owns_region(99, WrappedRegion::Results));
}

#[gpui_kit::test]
fn wheel_scroll_extends_held_log_row_selection(cx: &mut gpui_kit::TestAppContext) {
    check_wheel_during_log_selection(cx, false);
}

#[gpui_kit::test]
fn wheel_scroll_extends_held_wrapped_log_selection(cx: &mut gpui_kit::TestAppContext) {
    check_wheel_during_log_selection(cx, true);
}

fn check_wheel_during_log_selection(cx: &mut gpui_kit::TestAppContext, word_wrap: bool) {
    use gpui_kit::InputEvent as _;
    use gpui_kit::test::TestWindowExt as _;
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::actions::init(cx);
        Workspace::init_window_registry(cx);
        crate::notifications::init(cx);
        crate::app_icon::init(cx);
    });
    let directory = tempfile::tempdir().unwrap();
    let paths = [directory.path().join("wheel-drag.log")];
    let prepared = paths
        .iter()
        .map(|path| {
            std::fs::write(
                path,
                (0..500)
                    .map(|i| format!("line {i} selectable text\n"))
                    .collect::<String>(),
            )
            .unwrap();
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
        gpui_kit::component::Root::new(view, window, cx)
    });
    let workspace = workspace.unwrap();
    cx.run_until_parked();
    cx.update_window(handle.into(), |_, window, cx| {
        workspace.update(cx, |view, cx| {
            if view.active_document().unwrap().log_viewport.is_wrapped() != word_wrap {
                view.toggle_word_wrap(&ToggleWordWrap, window, cx);
            }
        });
        window.render_frame(cx);
        let document_id = workspace.read(cx).active_document().unwrap().id;
        let bounds = workspace.read(cx).row_drag_bounds[&(document_id, WrappedRegion::Log)];
        let start = point(bounds.left() + px(180.), bounds.center().y);
        window.dispatch_event(
            MouseDownEvent {
                position: start,
                button: MouseButton::Left,
                click_count: 1,
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
        let pointer = start + point(px(0.), px(60.));
        window.dispatch_event(
            MouseMoveEvent {
                position: pointer,
                pressed_button: Some(MouseButton::Left),
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
        for _ in 0..3 {
            window.simulate_next_frame(cx);
            window.render_frame(cx);
        }
        let drag = workspace
            .read(cx)
            .row_drag_selection
            .expect("row drag active");
        assert_eq!(drag.mode, RowDragMode::Lines);
        let before = workspace
            .read(cx)
            .active_document()
            .unwrap()
            .log_viewport
            .first_visible(500, workspace.read(cx).log_row_height());
        for delta in [-120., -120.] {
            window.dispatch_event(
                ScrollWheelEvent {
                    position: pointer,
                    delta: gpui_kit::ScrollDelta::Pixels(point(px(0.), px(delta))),
                    ..Default::default()
                }
                .to_platform_input(),
                cx,
            );
            for _ in 0..3 {
                window.simulate_next_frame(cx);
                window.render_frame(cx);
            }
        }
        let view = workspace.read(cx);
        let after = view
            .active_document()
            .unwrap()
            .log_viewport
            .first_visible(500, view.log_row_height());
        let extended = view.row_drag_selection.unwrap();
        assert!(after > before, "wheel must scroll while left mouse is held");
        assert_eq!(extended.start_row, drag.start_row);
        assert!(
            extended.target_row > drag.target_row,
            "stationary pointer extends selection after scrolling"
        );
        window.dispatch_event(
            ScrollWheelEvent {
                position: pointer,
                delta: gpui_kit::ScrollDelta::Pixels(point(px(0.), px(120.))),
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
        for _ in 0..3 {
            window.simulate_next_frame(cx);
            window.render_frame(cx);
        }
        assert!(
            workspace.read(cx).row_drag_selection.unwrap().target_row < extended.target_row,
            "reversing the wheel contracts selection without moving the pointer"
        );
        window.dispatch_event(
            MouseUpEvent {
                position: pointer,
                button: MouseButton::Left,
                click_count: 1,
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
        assert!(workspace.read(cx).row_drag_selection.is_none());
    })
    .unwrap();
}
