use super::*;
use gpui::{AvailableSpace, TestAppContext};

struct WrappedHeightTestView;

impl Render for WrappedHeightTestView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
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

fn assert_wrapped_layout_change_invalidates(update: impl FnOnce(&mut WrappedLayoutKey)) {
    let state = WrappedListState::<usize>::default();
    let current = wrapped_layout_key_for_test();
    assert!(state.invalidate_for_layout(current.clone()));
    state.prime_measured_heights(2, current.base_height, [(0, px(57.))]);
    assert_eq!(state.measured_heights.borrow().get(&0), Some(&px(57.)));

    let mut next = current;
    update(&mut next);
    assert!(state.invalidate_for_layout(next));
    assert!(state.measured_heights.borrow().is_empty());
}

#[gpui::test]
fn primed_wrapped_height_matches_the_row_text_layout(cx: &mut TestAppContext) {
    let (_, cx) = cx.add_window_view(|_, _| WrappedHeightTestView);
    let line: SharedString = "INFO 混合文字 alpha beta gamma delta epsilon zeta eta theta"
        .repeat(8)
        .into();
    let wrap_width = px(240.);
    let font_size = 13;
    let base_height = px(19.);
    let font_family: SharedString = "monospace".into();

    let predicted = cx.update(|window, _| {
        Workspace::measure_wrapped_line_height(
            line.clone(),
            wrap_width,
            font_size,
            &font_family,
            base_height,
            window,
        )
    });
    let actual_height = Rc::new(Cell::new(px(0.)));
    let measured_height = actual_height.clone();
    cx.draw(
        Point::default(),
        size(
            AvailableSpace::Definite(wrap_width),
            AvailableSpace::MinContent,
        ),
        move |_, _| {
            div()
                .w(wrap_width)
                .whitespace_normal()
                .text_size(px(font_size as f32))
                .line_height(base_height)
                .font_family(font_family)
                .on_prepaint(move |bounds, _, _| {
                    measured_height.set(bounds.size.height);
                })
                .child(StyledText::new(line))
        },
    );

    assert_eq!(predicted, actual_height.get());
}

#[test]
fn only_cross_row_line_drag_changes_selection_after_the_initial_table_event() {
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
fn centers_a_row_when_the_viewport_has_room_on_both_sides() {
    assert_eq!(
        centered_scroll_top(px(400.), px(20.), px(200.), px(800.)),
        px(310.)
    );
}

#[test]
fn keeps_centering_within_the_scrollable_edges() {
    assert_eq!(
        centered_scroll_top(px(20.), px(20.), px(200.), px(800.)),
        px(0.)
    );
    assert_eq!(
        centered_scroll_top(px(920.), px(20.), px(200.), px(820.)),
        px(820.)
    );
}

#[test]
fn log_jump_preload_range_covers_the_target_and_viewport_edges() {
    assert_eq!(centered_log_jump_preload_range(50, 100, 10), 35..65);
    assert_eq!(centered_log_jump_preload_range(2, 100, 10), 0..30);
    assert_eq!(centered_log_jump_preload_range(98, 100, 10), 70..100);
    assert_eq!(centered_log_jump_preload_range(0, 0, 10), 0..0);
}

#[test]
fn result_jump_tab_switch_preloads_the_target_before_activation() {
    assert_eq!(tab_switch_log_jump_preload_range(50, 100, 0, 0, 10), 35..65);
    assert_eq!(
        tab_switch_log_jump_preload_range(98, 100, 4, 8, 10),
        70..100
    );
}

#[test]
fn search_scope_switch_preloads_the_restored_viewport_before_commit() {
    assert_eq!(
        search_scope_switch_preload_range(50, false, 100, 10),
        35..65
    );
    assert_eq!(search_scope_switch_preload_range(2, false, 100, 10), 0..30);
    assert_eq!(search_scope_switch_preload_range(0, true, 100, 10), 70..100);
    assert_eq!(search_scope_switch_preload_range(0, false, 0, 10), 0..0);
}

#[test]
fn scrollbar_preload_range_clamps_to_the_target_viewport() {
    assert_eq!(
        scrollbar_preload_range(point(px(0.), px(-400.)), 100, px(200.), px(20.)),
        18..32
    );
    assert_eq!(
        scrollbar_preload_range(point(px(0.), px(-10_000.)), 100, px(200.), px(20.)),
        88..100
    );
    assert_eq!(
        scrollbar_preload_range(point(px(0.), px(-400.)), 0, px(200.), px(20.)),
        0..0
    );
}

#[test]
fn atomic_scrollbar_handle_keeps_the_committed_list_position_until_install() {
    let handle = UniformListScrollHandle::new();
    let pending_offset = Rc::new(Cell::new(None));
    let atomic = AtomicUniformScrollHandle {
        handle: handle.clone(),
        pending_offset: pending_offset.clone(),
    };
    let requested = point(px(0.), px(-420.));

    atomic.set_offset(requested);

    assert_eq!(handle.0.borrow().base_handle.offset(), Point::default());
    assert_eq!(pending_offset.get(), Some(requested));
}

#[test]
fn wrapped_scrollbar_keeps_the_committed_list_position_until_install() {
    let state = WrappedListState::<usize>::default();
    let logical = state.logical_scroll_handle(100, px(20.));
    let requested = point(px(0.), px(-420.));

    logical.set_offset(requested);

    assert_eq!(state.scroll_handle.offset(), Point::default());
    assert_eq!(state.pending_scrollbar_offset.get(), Some(requested));
}

#[test]
fn mode_switch_consumes_the_latest_scrollbar_position_before_changing_owner() {
    let viewport =
        LogViewportState::<usize>::new(false, UniformListScrollHandle::new(), Rc::default());
    let key = (7, WrappedRegion::Log);
    let stale_wheel = LogScrollFrameTarget::Viewport(point(px(0.), px(-300.)));
    let scrollbar = point(px(0.), px(-10_000.));
    let mut pending = PendingLogScrollFrames::default();
    pending.request(key, stale_wheel);
    viewport.atomic_fixed_scroll_handle().set_offset(scrollbar);

    assert_eq!(
        take_pending_log_scroll_target(&mut pending, key, &viewport),
        Some(LogScrollFrameTarget::Scrollbar(scrollbar))
    );
    assert_eq!(pending.latest(key), None);
    assert_eq!(viewport.take_pending_scrollbar_offset(), None);
}

#[test]
fn first_wrapped_frame_uses_the_committed_middle_viewport() {
    let viewport =
        LogViewportState::<usize>::new(false, UniformListScrollHandle::new(), Rc::default());
    viewport
        .fixed
        .scroll_handle
        .0
        .borrow()
        .base_handle
        .set_offset(point(px(0.), px(-400.)));

    assert_eq!(
        viewport.prospective_wrapped_measurement_range(100, px(200.), px(20.)),
        18..32
    );
}

#[test]
fn scroll_requests_coalesce_to_the_latest_target_before_the_next_frame() {
    let mut pending = PendingLogScrollFrames::default();
    let key = (7, WrappedRegion::Log);
    let first = LogScrollFrameTarget::Scrollbar(point(px(0.), px(-100.)));
    let middle = LogScrollFrameTarget::Scrollbar(point(px(0.), px(-500.)));
    let latest = LogScrollFrameTarget::Scrollbar(point(px(0.), px(-900.)));

    pending.request(key, first);
    pending.request(key, middle);
    pending.request(key, latest);

    assert_eq!(pending.latest(key), Some(latest));
    assert_eq!(pending.take(key), Some(latest));
    assert_eq!(pending.latest(key), None);
}

#[test]
fn clearing_pending_scroll_discards_the_unpainted_target() {
    let mut pending = PendingLogScrollFrames::default();
    let key = (7, WrappedRegion::Results);
    let target = LogScrollFrameTarget::Viewport(point(px(0.), px(-900.)));

    pending.request(key, target);
    pending.clear(key);

    assert_eq!(pending.latest(key), None);
    assert_eq!(pending.take(key), None);
}

#[test]
fn management_dialog_is_centered_in_the_viewport() {
    assert_eq!(centered_dialog_margin_top(px(900.), px(640.)), px(130.));
}

#[test]
fn management_dialog_centering_clamps_small_viewports() {
    assert_eq!(centered_dialog_margin_top(px(480.), px(640.)), px(0.));
}

#[test]
fn viewport_anchor_prefers_a_visible_selected_row() {
    assert_eq!(
        viewport_anchor_row(100, 20, Some(24), |row_ix| (20..30).contains(&row_ix)),
        24
    );
}

#[test]
fn viewport_anchor_falls_back_to_the_first_visible_row() {
    assert_eq!(
        viewport_anchor_row(100, 20, Some(60), |row_ix| (20..30).contains(&row_ix)),
        20
    );
}

#[test]
fn font_layout_changes_include_every_vertical_text_metric() {
    let current = AppSettings::default();

    let mut font_size = current.clone();
    font_size.log_font_size += 1;
    assert!(log_font_layout_changed(&current, &font_size));

    let mut line_spacing = current.clone();
    line_spacing.log_line_spacing += 1;
    assert!(log_font_layout_changed(&current, &line_spacing));

    let mut font_family = current.clone();
    font_family.log_font_family = crate::state_store::LogFontFamily::SystemMonospace;
    assert!(log_font_layout_changed(&current, &font_family));

    let mut unrelated = current.clone();
    unrelated.mouse_wheel_scroll_percent += 1;
    assert!(!log_font_layout_changed(&current, &unrelated));
}

#[test]
fn row_visibility_uses_the_row_and_viewport_edges() {
    assert!(row_intersects_viewport(px(-5.), px(20.), px(200.)));
    assert!(row_intersects_viewport(px(199.), px(20.), px(200.)));
    assert!(!row_intersects_viewport(px(-20.), px(20.), px(200.)));
    assert!(!row_intersects_viewport(px(200.), px(20.), px(200.)));
}

#[test]
fn text_selection_origin_must_be_inside_a_log_region() {
    let log = Bounds::new(point(px(10.), px(20.)), size(px(300.), px(200.)));
    let results = Bounds::new(point(px(10.), px(240.)), size(px(300.), px(120.)));

    assert!(point_in_text_selection_regions(
        point(px(50.), px(80.)),
        [log, results]
    ));
    assert!(point_in_text_selection_regions(
        point(px(50.), px(300.)),
        [log, results]
    ));
    assert!(!point_in_text_selection_regions(
        point(px(5.), px(80.)),
        [log, results]
    ));
    assert!(!point_in_text_selection_regions(
        point(px(50.), px(380.)),
        [log, results]
    ));
}

#[test]
fn row_height_lands_on_whole_device_pixels() {
    for scale_factor in [1., 1.25, 1.5, 1.75, 2.] {
        let snapped = snap_to_device_pixels(px(27.), scale_factor);
        let device_pixels = snapped.as_f32() * scale_factor;
        assert!(
            (device_pixels - device_pixels.round()).abs() < 1e-3,
            "{scale_factor} scale left {device_pixels} device pixels"
        );
        assert!((snapped.as_f32() - 27.).abs() <= 0.5 / scale_factor + 1e-3);
    }
}

#[test]
fn snapped_row_height_keeps_every_row_pitch_equal() {
    let snapped = snap_to_device_pixels(px(27.), 1.25);
    for row_ix in 0..64 {
        let top = (snapped * row_ix as f32).as_f32() * 1.25;
        assert!(
            (top - top.round()).abs() < 1e-3,
            "row {row_ix} landed at {top}"
        );
    }
}

#[test]
fn snapping_keeps_the_value_when_the_scale_factor_is_unusable() {
    assert_eq!(snap_to_device_pixels(px(27.), 0.), px(27.));
    assert_eq!(snap_to_device_pixels(px(27.), f32::NAN), px(27.));
}

#[test]
fn wrapped_measurement_range_covers_the_first_expanded_frame() {
    assert_eq!(
        wrapped_viewport_measurement_range(20, px(100.), px(20.), 100),
        18..27
    );
    assert_eq!(
        wrapped_viewport_measurement_range(0, px(0.), px(20.), 2),
        0..2
    );
}

#[test]
fn wrapped_measurements_are_invalidated_by_every_layout_dependency() {
    assert_wrapped_layout_change_invalidates(|key| key.content_revision += 1);
    assert_wrapped_layout_change_invalidates(|key| key.width += px(1.));
    assert_wrapped_layout_change_invalidates(|key| key.rem_size += px(1.));
    assert_wrapped_layout_change_invalidates(|key| key.font_family = "JetBrains Mono".into());
    assert_wrapped_layout_change_invalidates(|key| key.font_size += 1);
    assert_wrapped_layout_change_invalidates(|key| key.base_height += px(1.));
    assert_wrapped_layout_change_invalidates(|key| key.horizontal_padding += px(1.));
}

#[test]
fn subpixel_width_noise_keeps_wrapped_measurements() {
    let state = WrappedListState::<usize>::default();
    let current = wrapped_layout_key_for_test();
    assert!(state.invalidate_for_layout(current.clone()));
    state.prime_measured_heights(1, current.base_height, [(0, px(57.))]);

    let mut next = current;
    next.width += px(0.25);
    assert!(!state.invalidate_for_layout(next));
    assert_eq!(state.measured_heights.borrow().get(&0), Some(&px(57.)));
}

#[test]
fn positions_a_wrapped_row_top_at_the_requested_viewport_y() {
    let state = WrappedListState::<usize>::default();
    state.sizes(100, px(20.));

    state.scroll_row_to_viewport_y(40, px(7.));

    assert_eq!(
        state.prefix_height(40) + state.scroll_handle.offset().y,
        px(7.)
    );
    assert_eq!(
        state.measurement_anchor.get(),
        Some(RowViewportPosition {
            row_ix: 40,
            viewport_y: px(7.),
        })
    );
}

#[test]
fn measured_heights_reapply_the_exact_row_top_position() {
    let state = WrappedListState::<usize>::default();
    state.sizes(100, px(20.));
    state.scroll_row_to_viewport_y(40, px(-5.));

    assert!(state.queue_measured_height(10, px(60.), px(20.)));
    state.sizes(100, px(20.));

    assert_eq!(
        state.prefix_height(40) + state.scroll_handle.offset().y,
        px(-5.)
    );
}

#[test]
fn mode_switch_uses_a_fresh_wrapped_scroll_owner() {
    let mut state = WrappedListState::<usize>::default();
    let previous = state.scroll_handle.base_handle().clone();
    previous.set_offset(point(px(0.), px(-300.)));
    state
        .scroll_handle
        .scroll_to_item(80, ScrollStrategy::Center);

    state.reset_scroll_for_mode_switch();
    state.sizes(100, px(20.));
    state.scroll_row_to_viewport_y(40, px(7.));

    assert_eq!(previous.offset().y, px(-300.));
    assert_eq!(
        state.prefix_height(40) + state.scroll_handle.offset().y,
        px(7.)
    );
}

#[test]
fn viewport_routes_hit_testing_to_the_active_geometry_backend() {
    let fixed_bounds = Rc::new(RefCell::new(BTreeMap::from([
        (
            2,
            Bounds::new(point(px(0.), px(0.)), size(px(100.), px(20.))),
        ),
        (
            3,
            Bounds::new(point(px(0.), px(20.)), size(px(100.), px(20.))),
        ),
    ])));
    let viewport =
        LogViewportState::<usize>::new(false, UniformListScrollHandle::new(), fixed_bounds);
    viewport.wrapped_row_bounds().borrow_mut().insert(
        7,
        Bounds::new(point(px(0.), px(0.)), size(px(100.), px(20.))),
    );

    assert_eq!(viewport.row_at_position(point(px(4.), px(4.))), Some(2));
    assert_eq!(viewport.visible_row_edge(true), Some(3));

    viewport.set_word_wrap(true);

    assert_eq!(viewport.row_at_position(point(px(4.), px(4.))), Some(7));
    assert_eq!(viewport.visible_row_edge(false), Some(7));
}

#[test]
fn viewport_layout_invalidation_preserves_the_visible_row_position() {
    let viewport =
        LogViewportState::<usize>::new(true, UniformListScrollHandle::new(), Rc::default());
    viewport.wrapped_sizes(100, px(20.));
    viewport
        .wrapped_scroll_handle()
        .set_offset(point(px(0.), px(-793.)));

    assert!(viewport.invalidate_wrapped_layout_preserving_position(
        wrapped_layout_key_for_test(),
        Some(40),
    ));
    viewport.wrapped_sizes(100, px(20.));

    assert_eq!(
        px(20.) * 40. + viewport.wrapped_scroll_handle().offset().y,
        px(7.)
    );
}

#[test]
fn primed_heights_are_available_before_the_first_wrapped_frame() {
    let state = WrappedListState::<usize>::default();

    state.prime_measured_heights(100, px(20.), [(39, px(40.)), (40, px(60.)), (41, px(40.))]);
    state.scroll_row_to_viewport_y(40, px(7.));

    assert!(state.pending_heights.borrow().is_empty());
    assert_eq!(state.measured_heights.borrow().get(&40), Some(&px(60.)));
    assert_eq!(
        state.prefix_height(40) + state.scroll_handle.offset().y,
        px(7.)
    );
}

#[test]
fn wrapped_state_keeps_only_sparse_measurements_for_large_lists() {
    let state = WrappedListState::<usize>::default();

    let measurements = state.sizes(10_000_000, px(20.));

    assert_eq!(measurements.item_count, 10_000_000);
    assert!(measurements.measured_heights.borrow().is_empty());
    assert!(measurements.cumulative_corrections.borrow().is_empty());
}

#[test]
fn measured_heights_follow_stable_rows_when_search_results_change() {
    let mut state = WrappedListState::<usize>::default();
    state.prime_measured_heights(3, px(20.), [(0, px(40.))]);
    assert!(state.queue_measured_height(1, px(60.), px(20.)));
    let previous_rows = [10usize, 20, 30];
    let next_rows = [20usize, 10, 40];
    let measured_heights =
        state.measured_heights_by_key(|row_ix| previous_rows.get(row_ix).copied());

    state.reset_with_remapped_heights(next_rows.len(), px(20.), measured_heights, |source_row| {
        next_rows.iter().position(|row| row == source_row)
    });

    assert!(state.pending_heights.borrow().is_empty());
    assert_eq!(state.measured_heights.borrow().get(&0), Some(&px(60.)));
    assert_eq!(state.measured_heights.borrow().get(&1), Some(&px(40.)));
    assert_eq!(state.measured_heights.borrow().get(&2), None);
}

#[test]
fn global_measured_heights_follow_rows_after_a_group_collapses() {
    let mut state = WrappedListState::<LogRowKey>::default();
    state.prime_measured_heights(5, px(20.), [(1, px(40.)), (4, px(60.))]);
    let previous_rows = [
        LogRowKey::FileGroup { document_id: 1 },
        LogRowKey::Row {
            document_id: 1,
            source_row: 10,
        },
        LogRowKey::Row {
            document_id: 1,
            source_row: 11,
        },
        LogRowKey::FileGroup { document_id: 2 },
        LogRowKey::Row {
            document_id: 2,
            source_row: 20,
        },
    ];
    let next_rows = [
        LogRowKey::FileGroup { document_id: 1 },
        LogRowKey::FileGroup { document_id: 2 },
        LogRowKey::Row {
            document_id: 2,
            source_row: 20,
        },
    ];
    let measured_heights =
        state.measured_heights_by_key(|row_ix| previous_rows.get(row_ix).copied());

    state.reset_with_remapped_heights(next_rows.len(), px(20.), measured_heights, |key| {
        next_rows.iter().position(|row| row == key)
    });

    assert_eq!(state.measured_heights.borrow().get(&0), None);
    assert_eq!(state.measured_heights.borrow().get(&1), None);
    assert_eq!(state.measured_heights.borrow().get(&2), Some(&px(60.)));
}

#[test]
fn matches_only_keeps_an_empty_match_set_empty() {
    let marks = [2, 7].into_iter().collect();
    let rows = compute_result_rows(ResultMode::MatchesOnly, None, &marks);

    assert!(rows.is_empty());
}

#[test]
fn matches_and_marks_shows_marks_when_the_match_set_is_empty() {
    let marks = [2, 7].into_iter().collect();
    let rows = compute_result_rows(ResultMode::MatchesAndMarks, None, &marks);

    assert_eq!(rows.iter().collect::<Vec<_>>(), vec![2, 7]);
}

#[test]
fn result_modes_report_whether_they_include_marks() {
    assert!(ResultMode::MatchesAndMarks.includes_marks());
    assert!(ResultMode::MarksOnly.includes_marks());
    assert!(!ResultMode::MatchesOnly.includes_marks());
}

#[test]
fn restored_marks_can_make_the_result_projection_visible() {
    assert!(restored_results_visible(false, ResultMode::MarksOnly, true));
    assert!(!restored_results_visible(
        false,
        ResultMode::MatchesOnly,
        true
    ));
}

#[test]
fn hidden_results_cannot_own_restored_selection() {
    assert_eq!(
        restored_selection_table(PersistedLogRegion::CurrentResults, false),
        SelectionTable::Log
    );
    assert_eq!(
        restored_log_region(PersistedLogRegion::CurrentResults, false),
        LogRegion::Body
    );
    assert_eq!(
        restored_selection_table(PersistedLogRegion::CurrentResults, true),
        SelectionTable::Results
    );
    assert_eq!(
        restored_log_region(PersistedLogRegion::CurrentResults, true),
        LogRegion::CurrentResults
    );
}

#[test]
fn global_search_scopes_own_their_word_wrap_state() {
    assert!(!SearchScope::CurrentFile.owns_global_word_wrap());
    assert!(SearchScope::AllOpenFiles.owns_global_word_wrap());
    assert!(SearchScope::Directory.owns_global_word_wrap());
}

#[test]
fn restored_current_result_reapplies_the_domain_selection() {
    let rows = [4, 10, 42].into_iter().collect();
    let document = Arc::new(LogDocument::placeholder("restore-selection.log"));
    let delegate = LogTableDelegate::projected(1, document, rows);

    assert_eq!(delegate.settle_table_selection(1), Some(10));
    assert_eq!(delegate.selected_source_rows(), vec![10]);
}

#[test]
fn restored_documents_wait_for_ready_before_replacing_the_loading_surface() {
    assert!(!should_upgrade_loading_document(
        DocumentLoadState::Opening,
        DocumentLoadState::Preview,
        true,
    ));
    assert!(should_upgrade_loading_document(
        DocumentLoadState::Opening,
        DocumentLoadState::Ready,
        true,
    ));
    assert!(should_upgrade_loading_document(
        DocumentLoadState::Opening,
        DocumentLoadState::Preview,
        false,
    ));
    assert!(should_upgrade_loading_document(
        DocumentLoadState::Preview,
        DocumentLoadState::Ready,
        false,
    ));
    assert!(should_upgrade_loading_document(
        DocumentLoadState::IndexFailed,
        DocumentLoadState::Preview,
        false,
    ));
    assert!(should_upgrade_loading_document(
        DocumentLoadState::IndexFailed,
        DocumentLoadState::Ready,
        false,
    ));
}
