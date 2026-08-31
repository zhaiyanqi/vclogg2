use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
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
struct DisplaySpan {
    source: Range<usize>,
    display: Range<usize>,
}

/// A log line keeps the decoded source text authoritative while providing a
/// layout-safe representation for GPUI. GPUI's plain text element does not
/// implement tab stops, so tabs are expanded only for layout and painting.
#[derive(Clone, Default)]
pub(crate) struct LogText {
    source: SharedString,
    display: SharedString,
    display_spans: Rc<[DisplaySpan]>,
}

impl LogText {
    pub(crate) fn new(source: SharedString) -> Self {
        if !source.contains('\t') {
            return Self {
                display: source.clone(),
                source,
                display_spans: Rc::default(),
            };
        }

        let mut display = String::with_capacity(source.len());
        let mut display_spans = Vec::new();
        let mut column = 0usize;
        for (source_start, character) in source.char_indices() {
            let source_end = source_start + character.len_utf8();
            let display_start = display.len();
            if character == '\t' {
                let spaces = TAB_STOP_COLUMNS - column % TAB_STOP_COLUMNS;
                display.extend(std::iter::repeat_n(' ', spaces));
                column = column.saturating_add(spaces);
            } else {
                display.push(character);
                column = column.saturating_add(1);
            }
            display_spans.push(DisplaySpan {
                source: source_start..source_end,
                display: display_start..display.len(),
            });
        }

        Self {
            source,
            display: display.into(),
            display_spans: display_spans.into(),
        }
    }

    pub(crate) fn source(&self) -> &SharedString {
        &self.source
    }

    pub(crate) fn display(&self) -> &SharedString {
        &self.display
    }

    pub(crate) fn display_range(&self, source_range: Range<usize>) -> Option<Range<usize>> {
        if self.display_spans.is_empty() {
            return (source_range.end <= self.source.len()).then_some(source_range);
        }
        Some(self.display_offset(source_range.start)?..self.display_offset(source_range.end)?)
    }

    fn display_offset(&self, source_offset: usize) -> Option<usize> {
        if source_offset == self.source.len() {
            return Some(self.display.len());
        }
        self.display_spans
            .iter()
            .find(|span| span.source.start == source_offset)
            .map(|span| span.display.start)
    }

    fn source_range(&self, display_range: Range<usize>) -> Option<Range<usize>> {
        if self.display_spans.is_empty() {
            return (display_range.end <= self.source.len()).then_some(display_range);
        }
        let first = self
            .display_spans
            .iter()
            .find(|span| span.display.end > display_range.start)?;
        let last = self
            .display_spans
            .iter()
            .rev()
            .find(|span| span.display.start < display_range.end)?;
        Some(first.source.start..last.source.end)
    }

    fn source_text(&self, display_range: Range<usize>) -> Option<String> {
        self.source_range(display_range)
            .and_then(|range| self.source.get(range))
            .map(str::to_string)
    }
}

struct CachedSelection {
    selection: LogTextSelection,
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

pub struct TextSelectionCache<K> {
    entries: BTreeMap<K, CachedSelection>,
    activity: Rc<TextSelectionActivity>,
}

impl<K> Default for TextSelectionCache<K> {
    fn default() -> Self {
        Self {
            entries: BTreeMap::new(),
            activity: Rc::default(),
        }
    }
}

impl<K: Clone + Ord> TextSelectionCache<K> {
    pub fn handle(
        &mut self,
        key: K,
        text: &LogText,
        window: &Window,
        cx: &mut App,
    ) -> LogTextSelection {
        if !self.entries.contains_key(&key) {
            if self.entries.len() >= CACHE_LIMIT
                && let Some(evict) = self.entries.iter().find_map(|(key, selection)| {
                    selection
                        .selection
                        .handle
                        .snapshot(cx)
                        .is_none()
                        .then(|| key.clone())
                })
            {
                self.entries.remove(&evict);
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
                    _refresh: refresh,
                    _activity_subscription: activity_subscription,
                },
            );
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
        // 作为拖选起点，选择激活后再恢复全部可见行，保证跨行拖选仍可连续扩展。
        if self.selection.activity.is_active() || bounds.contains(&window.mouse_position()) {
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
            selection.set_active(true);
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
