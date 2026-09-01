use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet},
    ops::Range,
    rc::Rc,
};

use gpui::{
    App, BorderStyle, Bounds, Corners, Edges, Element, ElementId, GlobalElementId, Hitbox,
    InspectorElementId, IntoElement, LayoutId, MouseButton, MouseDownEvent, PaintQuad, Pixels,
    Point, SharedString, StyledText, Subscription, Window, transparent_black,
};
use gpui_base::{
    GlobalState, TextSelection, TextSelectionEvent, TextSelectionHandle, TextSelectionRegistration,
    TextSelectionRun,
};

const CACHE_LIMIT: usize = 2048;
const TAB_STOP_COLUMNS: usize = 8;
type MeasureCallback = dyn Fn(Pixels, &mut Window, &mut App);

#[derive(Clone)]
struct TabExpansion {
    source_offset: usize,
    display: Range<usize>,
}

/// A log line keeps the decoded source text authoritative while providing a
/// layout-safe representation for GPUI. GPUI's plain text element does not
/// implement tab stops, so tabs are expanded only for layout and painting.
#[derive(Clone, Default)]
pub(crate) struct LogText {
    source: SharedString,
    display: SharedString,
    tab_expansions: Rc<[TabExpansion]>,
}

impl LogText {
    pub(crate) fn new(source: SharedString) -> Self {
        if !source.contains('\t') {
            return Self {
                display: source.clone(),
                source,
                tab_expansions: Rc::default(),
            };
        }

        let mut display = String::with_capacity(source.len());
        let mut tab_expansions = Vec::new();
        let mut column = 0usize;
        for (source_start, character) in source.char_indices() {
            if character == '\t' {
                let display_start = display.len();
                let spaces = TAB_STOP_COLUMNS - column % TAB_STOP_COLUMNS;
                display.extend(std::iter::repeat_n(' ', spaces));
                column = column.saturating_add(spaces);
                tab_expansions.push(TabExpansion {
                    source_offset: source_start,
                    display: display_start..display.len(),
                });
            } else {
                display.push(character);
                column = column.saturating_add(1);
            }
        }

        Self {
            source,
            display: display.into(),
            tab_expansions: tab_expansions.into(),
        }
    }

    pub(crate) fn preview(mut source: String, truncated: bool) -> Self {
        if truncated {
            source.push('…');
        }
        Self::new(source.into())
    }

    pub(crate) fn source(&self) -> &SharedString {
        &self.source
    }

    pub(crate) fn display(&self) -> &SharedString {
        &self.display
    }

    pub(crate) fn retained_bytes(&self) -> usize {
        let text_bytes = if self.tab_expansions.is_empty() && self.source == self.display {
            self.source.len()
        } else {
            self.source.len().saturating_add(self.display.len())
        };
        text_bytes.saturating_add(
            self.tab_expansions
                .len()
                .saturating_mul(std::mem::size_of::<TabExpansion>()),
        )
    }

    pub(crate) fn display_range(&self, source_range: Range<usize>) -> Option<Range<usize>> {
        if self.tab_expansions.is_empty() {
            return (source_range.end <= self.source.len()).then_some(source_range);
        }
        Some(self.display_offset(source_range.start)?..self.display_offset(source_range.end)?)
    }

    fn display_offset(&self, source_offset: usize) -> Option<usize> {
        if source_offset > self.source.len() || !self.source.is_char_boundary(source_offset) {
            return None;
        }
        let completed = self
            .tab_expansions
            .partition_point(|expansion| expansion.source_offset < source_offset);
        let added = completed
            .checked_sub(1)
            .map(|index| {
                let expansion = &self.tab_expansions[index];
                expansion
                    .display
                    .end
                    .saturating_sub(expansion.source_offset.saturating_add(1))
            })
            .unwrap_or_default();
        source_offset.checked_add(added)
    }

    fn source_range(&self, display_range: Range<usize>) -> Option<Range<usize>> {
        if self.tab_expansions.is_empty() {
            return (display_range.end <= self.source.len()).then_some(display_range);
        }
        if display_range.start > display_range.end || display_range.end > self.display.len() {
            return None;
        }
        Some(
            self.source_offset_at_display_start(display_range.start)?
                ..self.source_offset_at_display_end(display_range.end)?,
        )
    }

    fn source_offset_at_display_start(&self, display_offset: usize) -> Option<usize> {
        let completed = self
            .tab_expansions
            .partition_point(|expansion| expansion.display.end <= display_offset);
        if let Some(expansion) = self.tab_expansions.get(completed)
            && expansion.display.start <= display_offset
        {
            return Some(expansion.source_offset);
        }
        let source_offset =
            display_offset.checked_sub(self.added_display_bytes_after(completed.checked_sub(1)))?;
        self.source_char_boundary_before(source_offset)
    }

    fn source_offset_at_display_end(&self, display_offset: usize) -> Option<usize> {
        let completed = self
            .tab_expansions
            .partition_point(|expansion| expansion.display.end < display_offset);
        if let Some(expansion) = self.tab_expansions.get(completed)
            && expansion.display.start < display_offset
        {
            return expansion.source_offset.checked_add(1);
        }
        let source_offset =
            display_offset.checked_sub(self.added_display_bytes_after(completed.checked_sub(1)))?;
        self.source_char_boundary_after(source_offset)
    }

    fn added_display_bytes_after(&self, expansion_ix: Option<usize>) -> usize {
        expansion_ix
            .and_then(|index| self.tab_expansions.get(index))
            .map(|expansion| {
                expansion
                    .display
                    .end
                    .saturating_sub(expansion.source_offset.saturating_add(1))
            })
            .unwrap_or_default()
    }

    fn source_char_boundary_before(&self, mut offset: usize) -> Option<usize> {
        if offset > self.source.len() {
            return None;
        }
        while !self.source.is_char_boundary(offset) {
            offset = offset.checked_sub(1)?;
        }
        Some(offset)
    }

    fn source_char_boundary_after(&self, mut offset: usize) -> Option<usize> {
        while offset < self.source.len() && !self.source.is_char_boundary(offset) {
            offset = offset.checked_add(1)?;
        }
        (offset <= self.source.len()).then_some(offset)
    }

    fn source_text(&self, display_range: Range<usize>) -> Option<String> {
        self.source_range(display_range)
            .and_then(|range| self.source.get(range))
            .map(str::to_string)
    }
}

struct CachedSelection {
    selection: LogTextSelection,
    last_used: u64,
    _refresh: Subscription,
    _activity_subscription: Subscription,
}

impl Drop for CachedSelection {
    fn drop(&mut self) {
        self.selection.set_active(false);
    }
}

#[derive(Default)]
struct TextSelectionActivity {
    active_count: Cell<usize>,
}

impl TextSelectionActivity {
    fn is_active(&self) -> bool {
        self.active_count.get() > 0
    }
}

#[derive(Default)]
struct LogTextSelectionState {
    text: LogText,
    projected_range: Option<Range<usize>>,
    custom_range: Option<Range<usize>>,
}

impl LogTextSelectionState {
    fn copy_text(&self) -> String {
        let range = self.custom_range.clone().or(self.projected_range.clone());
        range
            .and_then(|range| self.text.source_text(range))
            .unwrap_or_default()
    }
}

#[derive(Clone)]
pub struct LogTextSelection {
    handle: TextSelectionHandle,
    state: Rc<RefCell<LogTextSelectionState>>,
    activity: Rc<TextSelectionActivity>,
    active: Rc<Cell<bool>>,
}

impl LogTextSelection {
    fn set_active(&self, active: bool) {
        let previous = self.active.replace(active);
        if previous == active {
            return;
        }
        let count = self.activity.active_count.get();
        self.activity.active_count.set(if active {
            count.saturating_add(1)
        } else {
            count.saturating_sub(1)
        });
    }
}

/// Keeps stable selection participants for virtual rows.
///
/// Key order is unrelated to viewport recency: global results group keys by document and source
/// row, so evicting the smallest key can discard a row created earlier in the same render pass.
/// The separate recency index prevents that churn while keeping the cache bounded.
pub struct TextSelectionCache<K> {
    entries: BTreeMap<K, CachedSelection>,
    recency: BTreeSet<(u64, K)>,
    use_clock: u64,
    activity: Rc<TextSelectionActivity>,
}

impl<K> Default for TextSelectionCache<K> {
    fn default() -> Self {
        Self {
            entries: BTreeMap::new(),
            recency: BTreeSet::new(),
            use_clock: 0,
            activity: Rc::default(),
        }
    }
}

impl<K: Clone + Ord> TextSelectionCache<K> {
    fn next_use_order(&mut self) -> u64 {
        if self.use_clock == u64::MAX {
            let keys = self
                .recency
                .iter()
                .map(|(_, key)| key.clone())
                .collect::<Vec<_>>();
            self.recency.clear();
            self.use_clock = 0;
            for key in keys {
                self.use_clock += 1;
                if let Some(selection) = self.entries.get_mut(&key) {
                    selection.last_used = self.use_clock;
                    self.recency.insert((self.use_clock, key));
                }
            }
        }
        self.use_clock += 1;
        self.use_clock
    }

    fn touch(&mut self, key: &K, use_order: u64) {
        let Some(selection) = self.entries.get_mut(key) else {
            return;
        };
        self.recency.remove(&(selection.last_used, key.clone()));
        selection.last_used = use_order;
        self.recency.insert((use_order, key.clone()));
    }

    fn evict_oldest_inactive(&mut self, cx: &App) -> bool {
        let evict = self.recency.iter().find_map(|(_, key)| {
            let selection = &self.entries[key].selection;
            (selection.handle.snapshot(cx).is_none() && !selection.handle.has_local_selection(cx))
                .then(|| key.clone())
        });
        let Some(evict) = evict else {
            return false;
        };
        if let Some(selection) = self.entries.remove(&evict) {
            self.recency.remove(&(selection.last_used, evict));
            true
        } else {
            false
        }
    }

    pub fn handle(
        &mut self,
        key: K,
        text: &LogText,
        window: &Window,
        cx: &mut App,
    ) -> LogTextSelection {
        let use_order = self.next_use_order();
        if !self.entries.contains_key(&key) {
            while self.entries.len() >= CACHE_LIMIT {
                if !self.evict_oldest_inactive(cx) {
                    break;
                }
            }
            let fallback_text = text.source().to_string();
            let handle = TextSelectionHandle::new(fallback_text, cx);
            let state = Rc::new(RefCell::new(LogTextSelectionState {
                text: text.clone(),
                ..Default::default()
            }));
            let clear_state = state.clone();
            handle.clear_with(
                move |_| {
                    let mut state = clear_state.borrow_mut();
                    state.custom_range = None;
                    state.projected_range = None;
                },
                cx,
            );
            let copy_state = state.clone();
            handle.copy_with(move |_| copy_state.borrow().copy_text(), cx);
            let refresh = handle.refresh_window_on_change(window, cx);
            let active = Rc::new(Cell::new(false));
            let activity = self.activity.clone();
            let active_for_subscription = active.clone();
            let activity_subscription = handle.subscribe(
                move |event, _| match event {
                    TextSelectionEvent::SelectionChanged(snapshot) => {
                        let next_active = snapshot.is_some();
                        let previous = active_for_subscription.replace(next_active);
                        if previous != next_active {
                            let count = activity.active_count.get();
                            activity.active_count.set(if next_active {
                                count.saturating_add(1)
                            } else {
                                count.saturating_sub(1)
                            });
                        }
                    }
                    TextSelectionEvent::Cleared => {
                        if active_for_subscription.replace(false) {
                            activity
                                .active_count
                                .set(activity.active_count.get().saturating_sub(1));
                        }
                    }
                    TextSelectionEvent::AutoScroll(_) => {}
                },
                cx,
            );
            self.entries.insert(
                key.clone(),
                CachedSelection {
                    selection: LogTextSelection {
                        handle,
                        state,
                        activity: self.activity.clone(),
                        active,
                    },
                    last_used: use_order,
                    _refresh: refresh,
                    _activity_subscription: activity_subscription,
                },
            );
            self.recency.insert((use_order, key.clone()));
        } else {
            self.touch(&key, use_order);
        }
        let selection = self.entries[&key].selection.clone();
        if selection.state.borrow().text.source().as_ref() != text.source().as_ref() {
            selection
                .handle
                .set_fallback_copy_text(text.source().to_string(), cx);
            selection.state.borrow_mut().text = text.clone();
        }
        selection
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.recency.clear();
        self.use_clock = 0;
    }
}

pub struct SelectableLogText {
    selection: LogTextSelection,
    text: LogText,
    styled_text: StyledText,
    document_order: u64,
    selection_color: gpui::Hsla,
    suppress_selection: bool,
    word_boundary_characters: SharedString,
    on_measure: Option<Box<MeasureCallback>>,
}

impl SelectableLogText {
    pub fn new(
        selection: LogTextSelection,
        document_order: u64,
        text: LogText,
        styled_text: StyledText,
        selection_color: gpui::Hsla,
    ) -> Self {
        Self {
            selection,
            text,
            styled_text,
            document_order,
            selection_color,
            suppress_selection: false,
            word_boundary_characters: SharedString::default(),
            on_measure: None,
        }
    }

    pub fn suppress_selection(mut self, suppress: bool) -> Self {
        self.suppress_selection = suppress;
        self
    }

    pub fn word_boundary_characters(mut self, characters: impl Into<SharedString>) -> Self {
        self.word_boundary_characters = characters.into();
        self
    }

    pub fn on_measure(
        mut self,
        callback: impl Fn(Pixels, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_measure = Some(Box::new(callback));
        self
    }

    fn selection_bounds(
        start: Point<Pixels>,
        end: Point<Pixels>,
        bounds: Bounds<Pixels>,
        line_height: Pixels,
    ) -> Vec<Bounds<Pixels>> {
        if start.y == end.y {
            return vec![Bounds::from_corners(
                start,
                Point::new(end.x, end.y + line_height),
            )];
        }
        let mut quads = vec![Bounds::from_corners(
            start,
            Point::new(bounds.right(), start.y + line_height),
        )];
        if end.y > start.y + line_height {
            quads.push(Bounds::from_corners(
                Point::new(bounds.left(), start.y + line_height),
                Point::new(bounds.right(), end.y),
            ));
        }
        quads.push(Bounds::from_corners(
            Point::new(bounds.left(), end.y),
            Point::new(end.x, end.y + line_height),
        ));
        quads
    }

    fn paint_selection(&self, layout: &gpui::TextLayout, range: Range<usize>, window: &mut Window) {
        let (Some(start), Some(end)) = (
            layout.position_for_index(range.start),
            layout.position_for_index(range.end),
        ) else {
            return;
        };
        for bounds in Self::selection_bounds(start, end, layout.bounds(), layout.line_height()) {
            window.paint_quad(PaintQuad {
                bounds,
                background: self.selection_color.into(),
                corner_radii: Corners::default(),
                border_widths: Edges::default(),
                border_color: transparent_black(),
                border_style: BorderStyle::default(),
            });
        }
    }
}

fn word_ranges_near_offset(
    text: &str,
    offset: usize,
    boundary_characters: &str,
) -> Vec<Range<usize>> {
    let previous = text
        .char_indices()
        .take_while(|(start, _)| *start < offset)
        .last()
        .map(|(start, _)| start);
    [Some(offset.min(text.len())), previous]
        .into_iter()
        .flatten()
        .filter_map(|candidate| word_range_at_offset(text, candidate, boundary_characters))
        .fold(Vec::new(), |mut ranges, range| {
            if !ranges.contains(&range) {
                ranges.push(range);
            }
            ranges
        })
}

fn word_range_at_offset(
    text: &str,
    offset: usize,
    boundary_characters: &str,
) -> Option<Range<usize>> {
    let characters = text
        .char_indices()
        .map(|(start, character)| (start, start + character.len_utf8(), character))
        .collect::<Vec<_>>();
    let selected_ix = characters
        .iter()
        .position(|(start, end, _)| *start <= offset && offset < *end)?;
    let is_boundary = |character: char| {
        character.is_whitespace() || boundary_characters.chars().any(|item| item == character)
    };
    if is_boundary(characters[selected_ix].2) {
        return None;
    }
    let mut start_ix = selected_ix;
    while start_ix > 0 && !is_boundary(characters[start_ix - 1].2) {
        start_ix -= 1;
    }
    let mut end_ix = selected_ix;
    while end_ix + 1 < characters.len() && !is_boundary(characters[end_ix + 1].2) {
        end_ix += 1;
    }
    Some(characters[start_ix].0..characters[end_ix].1)
}

impl IntoElement for SelectableLogText {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for SelectableLogText {
    type RequestLayoutState = ();
    type PrepaintState = Hitbox;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let _performance_scope = crate::ui_performance::scope("SelectableLogText::request_layout");
        self.styled_text
            .request_layout(id, inspector_id, window, cx)
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let _performance_scope = crate::ui_performance::scope("SelectableLogText::prepaint");
        self.styled_text
            .prepaint(id, inspector_id, bounds, &mut (), window, cx);
        let text_bounds = self.styled_text.layout().bounds();
        if let Some(on_measure) = &self.on_measure {
            on_measure(text_bounds.size.height, window, cx);
        }
        let hitbox = window.insert_hitbox(bounds, gpui::HitboxBehavior::Normal);
        // gpui-component 的 register 会向当前全部文本选择参与者发布快照。空闲帧若让
        // 每个可见日志行都注册，会形成 O(可见行数²) 的重复遍历；仅保留鼠标所在行
        // 作为拖选起点，跨行选择激活后再恢复全部可见行。双击选词属于参与者本地选择，
        // 它没有几何快照，必须单独保持当前行注册，否则鼠标离开后帧清理会清掉词选区。
        if self.selection.activity.is_active()
            || self.selection.handle.has_local_selection(cx)
            || bounds.contains(&window.mouse_position())
        {
            let _performance_scope =
                crate::ui_performance::scope("SelectableLogText::register_selection");
            self.selection.handle.register(
                TextSelectionRegistration::new(hitbox.clone(), bounds)
                    .with_document_order(self.document_order)
                    .with_text_bounds(vec![text_bounds]),
                window,
                cx,
            );
        }
        hitbox
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        hitbox: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let _performance_scope = crate::ui_performance::scope("SelectableLogText::paint");
        let layout = self.styled_text.layout().clone();
        let selection = self.selection.clone();
        let selection_state = selection.state.clone();
        let event_layout = layout.clone();
        let event_hitbox = hitbox.clone();
        let boundary_characters = self.word_boundary_characters.clone();
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
            if !phase.bubble()
                || event.button != MouseButton::Left
                || event.click_count != 2
                || event.modifiers.control
                || event.modifiers.shift
                || !event_hitbox.is_hovered(window)
            {
                return;
            }
            let Ok(offset) = event_layout.index_for_position(event.position) else {
                return;
            };
            let range = word_ranges_near_offset(
                selection_state.borrow().text.display(),
                offset,
                &boundary_characters,
            )
            .into_iter()
            .find(|range| {
                let (Some(start), Some(end)) = (
                    event_layout.position_for_index(range.start),
                    event_layout.position_for_index(range.end),
                ) else {
                    return false;
                };
                SelectableLogText::selection_bounds(
                    start,
                    end,
                    event_layout.bounds(),
                    event_layout.line_height(),
                )
                .into_iter()
                .any(|bounds| bounds.contains(&event.position))
            });
            let Some(range) = range else {
                return;
            };
            GlobalState::suppress_text_selection(cx);
            TextSelection::clear(window, cx);
            selection.state.borrow_mut().custom_range = Some(range);
            selection.handle.set_local_selection(true, cx);
            window.refresh();
        });
        // update_runs 会更新当前行的投影；选择快照变化已由 register 和订阅触发刷新。
        // 不在每一行 paint 前后调用 selected_text，避免再次遍历所有参与者形成 O(N²)。
        let projection = self.selection.handle.update_runs(
            &[TextSelectionRun::new(
                self.text.display().clone(),
                layout.clone(),
                bounds,
            )],
            cx,
        );
        {
            let mut state = self.selection.state.borrow_mut();
            if state.text.source().as_ref() != self.text.source().as_ref() {
                state.text = self.text.clone();
            }
            state.projected_range = projection.ranges().first().and_then(Clone::clone);
        }
        self.styled_text
            .paint(id, inspector_id, bounds, &mut (), &mut (), window, cx);
        let painted_range = self
            .selection
            .state
            .borrow()
            .custom_range
            .clone()
            .or_else(|| projection.ranges().first().and_then(Clone::clone));
        if !self.suppress_selection
            && let Some(range) = painted_range
        {
            self.paint_selection(&layout, range, window);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{
        Context, Hsla, Modifiers, MouseMoveEvent, MouseUpEvent, ParentElement as _, Render,
        Styled as _, TestAppContext, div, hsla, point, px,
    };
    use gpui_base::TextSelectionLayer;

    const TEST_SELECTION_COLOR: Hsla = hsla(0.37, 0.91, 0.43, 1.);

    struct SelectableLogTextTestView {
        text: LogText,
        selections: TextSelectionCache<usize>,
    }

    struct CachedRowsTestView {
        text: LogText,
        selections: TextSelectionCache<usize>,
        visible_keys: Vec<usize>,
    }

    #[test]
    fn tab_coordinates_round_trip_with_sparse_expansions() {
        let text = LogText::new("ab\t中x".into());

        assert_eq!(text.display().as_ref(), "ab      中x");
        assert_eq!(text.tab_expansions.len(), 1);
        assert_eq!(text.display_range(0..2), Some(0..2));
        assert_eq!(text.display_range(2..3), Some(2..8));
        assert_eq!(text.display_range(3..6), Some(8..11));
        assert_eq!(text.display_range(6..7), Some(11..12));
        assert_eq!(text.source_range(4..5), Some(2..3));
        assert_eq!(text.source_range(8..11), Some(3..6));
        assert_eq!(text.source_range(9..10), Some(3..6));
        assert_eq!(text.source_range(11..12), Some(6..7));
        assert_eq!(text.display_range(4..5), None);
    }

    #[test]
    fn tab_mapping_storage_scales_with_tabs_instead_of_characters() {
        let mut source = "a".repeat(64 * 1024);
        source.push('\t');
        let text = LogText::new(source.clone().into());

        assert_eq!(text.tab_expansions.len(), 1);
        assert!(
            text.retained_bytes()
                <= source
                    .len()
                    .saturating_add(text.display().len())
                    .saturating_add(std::mem::size_of::<TabExpansion>())
        );
    }

    impl Render for SelectableLogTextTestView {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let selection = self.selections.handle(0, &self.text, window, cx);
            let styled_text = StyledText::new(self.text.display().clone());
            div().size_full().child(TextSelectionLayer).child(
                div()
                    .absolute()
                    .left(px(10.))
                    .top(px(10.))
                    .text_size(px(14.))
                    .child(SelectableLogText::new(
                        selection,
                        0,
                        self.text.clone(),
                        styled_text,
                        TEST_SELECTION_COLOR,
                    )),
            )
        }
    }

    impl Render for CachedRowsTestView {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let rows = self
                .visible_keys
                .clone()
                .into_iter()
                .enumerate()
                .map(|(ix, key)| {
                    let selection = self.selections.handle(key, &self.text, window, cx);
                    div()
                        .absolute()
                        .left(px(10.))
                        .top(px(10. + ix as f32 * 24.))
                        .h(px(24.))
                        .text_size(px(14.))
                        .child(SelectableLogText::new(
                            selection,
                            key as u64,
                            self.text.clone(),
                            StyledText::new(self.text.display().clone()),
                            TEST_SELECTION_COLOR,
                        ))
                });
            div().size_full().child(TextSelectionLayer).children(rows)
        }
    }

    #[gpui::test]
    fn double_clicked_word_survives_dragging_outside_the_line(cx: &mut TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, _| SelectableLogTextTestView {
            text: LogText::new("alpha beta".into()),
            selections: TextSelectionCache::default(),
        });
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });

        let word_position = point(px(20.), px(18.));
        cx.simulate_event(MouseDownEvent {
            position: word_position,
            modifiers: Modifiers::default(),
            button: MouseButton::Left,
            click_count: 2,
            first_mouse: false,
        });
        cx.update(|window, cx| {
            let _ = window.draw(cx);
            let view = view.read(cx);
            assert!(!view.selections.activity.is_active());
            assert!(
                view.selections.entries[&0]
                    .selection
                    .handle
                    .has_local_selection(cx)
            );
            assert_eq!(TextSelection::selected_text(window, cx), "alpha");
            assert!(
                window
                    .painted_quads()
                    .iter()
                    .any(|quad| quad.background == TEST_SELECTION_COLOR.into())
            );
        });

        let outside_position = point(px(200.), px(80.));
        cx.simulate_event(MouseMoveEvent {
            position: outside_position,
            pressed_button: Some(MouseButton::Left),
            modifiers: Modifiers::default(),
        });
        cx.simulate_event(MouseUpEvent {
            position: outside_position,
            modifiers: Modifiers::default(),
            button: MouseButton::Left,
            click_count: 2,
        });
        cx.update(|window, cx| {
            let _ = window.draw(cx);
            window.simulate_next_frame(cx);
            let _ = window.draw(cx);
            assert_eq!(TextSelection::selected_text(window, cx), "alpha");
            assert!(
                window
                    .painted_quads()
                    .iter()
                    .any(|quad| quad.background == TEST_SELECTION_COLOR.into())
            );
        });
    }

    #[gpui::test]
    fn recently_rendered_middle_rows_stay_cached_at_capacity(cx: &mut TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, _| SelectableLogTextTestView {
            text: LogText::new("alpha beta".into()),
            selections: TextSelectionCache::default(),
        });

        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                for key in 10_000..10_000 + CACHE_LIMIT {
                    view.selections.handle(key, &view.text, window, cx);
                }

                let first = view.selections.handle(1, &view.text, window, cx);
                view.selections.handle(2, &view.text, window, cx);

                assert_eq!(
                    view.selections.entries[&1].selection.handle.entity_id(),
                    first.handle.entity_id(),
                    "rendering the next middle row must not evict the row rendered just before it"
                );
            });
        });
    }

    #[gpui::test]
    fn middle_row_selection_survives_the_next_render_at_capacity(cx: &mut TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, _| CachedRowsTestView {
            text: LogText::new("alpha beta".into()),
            selections: TextSelectionCache::default(),
            visible_keys: Vec::new(),
        });
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                for key in 10_000..10_000 + CACHE_LIMIT {
                    view.selections.handle(key, &view.text, window, cx);
                }
                view.visible_keys = vec![1, 2];
                cx.notify();
            });
            let _ = window.draw(cx);
        });

        cx.simulate_event(MouseDownEvent {
            position: point(px(20.), px(18.)),
            modifiers: Modifiers::default(),
            button: MouseButton::Left,
            click_count: 2,
            first_mouse: false,
        });
        cx.update(|window, cx| {
            let _ = window.draw(cx);
            window.simulate_next_frame(cx);
            let _ = window.draw(cx);
            assert_eq!(TextSelection::selected_text(window, cx), "alpha");
            assert!(
                window
                    .painted_quads()
                    .iter()
                    .any(|quad| quad.background == TEST_SELECTION_COLOR.into())
            );
            TextSelection::clear(window, cx);
        });

        let drag_start = point(px(20.), px(18.));
        let drag_end = point(px(50.), px(18.));
        cx.simulate_event(MouseMoveEvent {
            position: drag_start,
            pressed_button: None,
            modifiers: Modifiers::default(),
        });
        cx.update(|window, cx| {
            let _ = window.draw(cx);
        });
        cx.simulate_event(MouseDownEvent {
            position: drag_start,
            modifiers: Modifiers::default(),
            button: MouseButton::Left,
            click_count: 1,
            first_mouse: false,
        });
        cx.simulate_event(MouseMoveEvent {
            position: drag_end,
            pressed_button: Some(MouseButton::Left),
            modifiers: Modifiers::default(),
        });
        cx.update(|window, cx| {
            let _ = window.draw(cx);
            window.simulate_next_frame(cx);
            let _ = window.draw(cx);
            assert!(!TextSelection::selected_text(window, cx).is_empty());
        });
        cx.simulate_event(MouseUpEvent {
            position: drag_end,
            modifiers: Modifiers::default(),
            button: MouseButton::Left,
            click_count: 1,
        });
    }

    #[gpui::test]
    fn local_selection_is_not_evicted_by_later_rows(cx: &mut TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, _| SelectableLogTextTestView {
            text: LogText::new("alpha beta".into()),
            selections: TextSelectionCache::default(),
        });

        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                for key in 10_000..10_000 + CACHE_LIMIT {
                    view.selections.handle(key, &view.text, window, cx);
                }
                let selected = view.selections.handle(1, &view.text, window, cx);
                selected.handle.set_local_selection(true, cx);

                for key in 2..CACHE_LIMIT + 2 {
                    view.selections.handle(key, &view.text, window, cx);
                }

                assert_eq!(
                    view.selections.entries[&1].selection.handle.entity_id(),
                    selected.handle.entity_id()
                );
            });
        });
    }
}
