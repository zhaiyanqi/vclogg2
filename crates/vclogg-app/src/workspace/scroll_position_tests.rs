use super::*;
use crate::virtual_log_list::VirtualLogListPosition;

fn test_viewport(word_wrap: bool, count: usize) -> LogViewportState<usize> {
    let viewport = VirtualLogViewport::new();
    viewport.set_item_count(count, px(20.));
    LogViewportState::new(word_wrap, viewport, Rc::default())
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
        38..52
    );
    assert_eq!(
        scrollbar_preload_range(point(px(0.), px(-20_000.)), 100, px(200.), px(20.)),
        88..100
    );
}

#[test]
fn candidate_measurement_range_is_bounded_around_the_anchor() {
    assert_eq!(
        wrapped_viewport_measurement_range(40, px(200.), px(20.), 100),
        38..52
    );
    assert_eq!(
        wrapped_viewport_measurement_range(8_000_000, px(200.), px(20.), 10_000_000),
        7_999_998..8_000_012
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
