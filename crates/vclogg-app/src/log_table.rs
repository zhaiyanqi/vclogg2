use std::{
    cell::{Cell, RefCell},
    cmp::Reverse,
    collections::{BTreeMap, BinaryHeap},
    ops::Range,
    rc::Rc,
    sync::Arc,
};

use gpui::{
    AnyElement, App, AppContext as _, Bounds, Context, Div, HighlightStyle, Hsla,
    InteractiveElement as _, IntoElement, MouseButton, MouseDownEvent, ParentElement as _, Pixels,
    SharedString, Stateful, Styled as _, StyledText, Task, Window, div, linear_color_stop,
    linear_gradient, point, prelude::FluentBuilder as _, px,
};
use gpui_base::{GlobalState, TextSelection};
use gpui_component::{
    ActiveTheme as _, ElementExt as _, h_flex,
    table::{Column, TableDelegate, TableState},
    theme::try_parse_color,
    v_flex,
};
use vclogg_core::{CompressedRows, LinePreviewReader, LogDocument, SearchMatcher};

#[cfg(test)]
use vclogg_core::LinePreview;

use crate::color_labels::ResolvedColorRules;
use crate::selectable_log_text::{LogSeverity, LogText, SelectableLogText, TextSelectionCache};
use crate::state_store::{AppSettings, DEFAULT_WORD_BOUNDARY_CHARACTERS, LogFontFamily};
use crate::ui_theme;
use crate::virtual_log_lines::{
    LogRowKey, LogRowProjection, MAX_VISIBLE_LINE_COLUMNS, StagedVisibleLineLoadRequest,
    StagedVisibleLineLoadResult, VisibleLineLoadRequest, VisibleLineLoadResult, VisibleLineStore,
};

pub(crate) fn log_cell_horizontal_padding(cx: &App) -> Pixels {
    cx.theme().spacing_tokens().sm
}

pub(crate) fn log_line_height(font_size: u16, line_spacing: u16) -> Pixels {
    px(font_size.saturating_add(line_spacing) as f32)
}

/// 行号字号按 `clamp(8px, log-font-size - 2px, 18px)` 计算，避免小字号下看不清，
/// 也避免大字号下喧宾夺主。
pub(crate) fn line_number_font_size(log_font_size: u16) -> Pixels {
    px(log_font_size.saturating_sub(2).clamp(8, 18) as f32)
}

pub(crate) fn log_row_selection_color(cx: &App) -> Hsla {
    cx.theme().table_active
}

/// 分隔线使用行内绝对定位的 1px 覆盖层，不参与布局。自动换行时不会把内容撑高，
/// 固定行高和自动换行也可以复用完全相同的单元格绘制规则。
pub(crate) fn log_row_separator_overlay(at_top: bool, cx: &App) -> Div {
    div()
        .absolute()
        .left_0()
        .right_0()
        .h(px(1.))
        .bg(cx.theme().border)
        .map(|line| {
            if at_top {
                line.top_0()
            } else {
                line.bottom_0()
            }
        })
}

pub(crate) fn log_line_number_cell(
    source_row: usize,
    log_font_size: u16,
    line_height: Pixels,
    text_color: Hsla,
    background_color: Hsla,
    show_separator: bool,
    cx: &App,
) -> Div {
    h_flex()
        .relative()
        .justify_end()
        .px(log_cell_horizontal_padding(cx))
        .bg(background_color)
        .text_right()
        .text_size(line_number_font_size(log_font_size))
        .line_height(line_height)
        .text_color(text_color)
        .when(show_separator, |cell| {
            cell.child(log_row_separator_overlay(false, cx))
        })
        .child((source_row + 1).to_string())
}

/// 固定行高模式下 `DataTable` 会在固定列组右边缘画一条 1px 竖线，且画在固定列组内部的最后
/// 1px 上。自动换行模式自己拼行，需要在同一位置补一条一样的线，两种模式才看起来一致。
pub(crate) fn log_fixed_column_divider_overlay(fixed_columns_width: Pixels, cx: &App) -> Div {
    div()
        .absolute()
        .top_0()
        .bottom_0()
        .left(fixed_columns_width - px(1.))
        .w(px(1.))
        .bg(cx.theme().border)
}

pub(crate) fn log_row_selection_overlay(
    show_top_border: bool,
    show_bottom_border: bool,
    cx: &App,
) -> Div {
    div()
        .absolute()
        .inset_0()
        .when(show_top_border, |overlay| overlay.border_t_1())
        .when(show_bottom_border, |overlay| overlay.border_b_1())
        .border_color(cx.theme().table_active_border)
}

/// 级别着色使用「整行实色底 + 最左侧 3px 色条」。
#[derive(Clone, Copy)]
pub(crate) struct SeverityStyle {
    pub(crate) background: Hsla,
    pub(crate) accent: Hsla,
}

pub(crate) fn severity_style(severity: LogSeverity, cx: &App) -> SeverityStyle {
    let colors = ui_theme::palette(cx);
    match severity {
        LogSeverity::Error => SeverityStyle {
            background: colors.severity_error_background,
            accent: colors.severity_error_accent,
        },
        LogSeverity::Warning => SeverityStyle {
            background: colors.severity_warning_background,
            accent: colors.severity_warning_accent,
        },
        LogSeverity::Info => SeverityStyle {
            background: colors.severity_info_background,
            accent: colors.severity_info_accent,
        },
        LogSeverity::Debug => SeverityStyle {
            background: colors.severity_debug_background,
            accent: colors.severity_debug_accent,
        },
    }
}

/// 级别色条使用不占布局的绝对定位覆盖层，挂在标记列上即可贴住行的左缘。
pub(crate) fn severity_accent_overlay(accent: Hsla) -> Div {
    div()
        .absolute()
        .left_0()
        .top_0()
        .bottom_0()
        .w(px(3.))
        .bg(accent)
}

pub(crate) fn message_column_width(
    max_columns: usize,
    font_family: SharedString,
    font_size: u16,
    cx: &App,
) -> gpui::Pixels {
    let max_columns = bounded_message_columns(max_columns);
    let em = px(font_size as f32);
    let font_id = cx.text_system().resolve_font(&gpui::font(font_family));
    let column_advance = cx
        .text_system()
        .ch_advance(font_id, em)
        .unwrap_or(em * 0.62);
    (column_advance * max_columns as f32 + em * 4.).max(em * 24.)
}

fn bounded_message_columns(max_columns: usize) -> usize {
    max_columns.min(MAX_VISIBLE_LINE_COLUMNS)
}

pub(crate) fn line_marker_column_width() -> Pixels {
    // Keep the marker gutter fixed instead of scaling it with the configurable log font.
    px(22.)
}

pub(crate) fn line_marker(marked: bool, matched: bool, cx: &App) -> AnyElement {
    let colors = ui_theme::palette(cx);
    let dot_border = if marked {
        colors.marker_marked_border
    } else if matched {
        colors.marker_matched_border
    } else {
        colors.marker_border
    };
    let dot = h_flex()
        .size(px(8.))
        .overflow_hidden()
        .rounded_full()
        .border_1()
        .border_color(dot_border)
        .when(!marked && !matched, |dot| dot.bg(colors.surface))
        .when(marked && !matched, |dot| dot.bg(colors.marker_marked))
        .when(matched && !marked, |dot| dot.bg(colors.marker_matched))
        .when(marked && matched, |dot| {
            dot.bg(linear_gradient(
                135.,
                linear_color_stop(colors.marker_matched, 0.5),
                linear_color_stop(colors.marker_marked, 0.5),
            ))
        });
    h_flex()
        .size(px(12.))
        .justify_center()
        .rounded_full()
        .when(matched && !marked, |halo| {
            halo.bg(colors.marker_matched.opacity(0.10))
        })
        .when(marked, |halo| halo.bg(colors.marker_marked.opacity(0.18)))
        .child(dot)
        .into_any_element()
}

/// 命中与标记的正文高亮使用实色底配深色文字，背景与前景必须成对给出，
/// 避免深色主题下出现浅底浅字。
pub(crate) fn text_highlight_style(highlight: TextHighlight, cx: &App) -> HighlightStyle {
    let colors = ui_theme::palette(cx);
    let (background, foreground) = match highlight {
        // 颜色标签由用户挑选，统一使用近黑文字保证可读。
        TextHighlight::Color(color) => (color, gpui::rgb(0x141414).into()),
        TextHighlight::Search => (colors.search_match, colors.search_match_foreground),
        TextHighlight::QuickFind => (colors.quick_find, colors.quick_find_foreground),
    };
    HighlightStyle {
        background_color: Some(background),
        color: Some(foreground),
        ..Default::default()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum TextHighlight {
    Color(gpui::Hsla),
    Search,
    QuickFind,
}

#[derive(Default)]
pub(crate) struct RowSelection {
    ranges: Vec<(usize, usize)>,
    selected_count: usize,
    revision: u64,
    anchor: Option<usize>,
    pending_pointer_row: Option<usize>,
    pointer_drag_anchor: Option<usize>,
    pointer_drag_additive: bool,
    pointer_drag_base_ranges: Vec<(usize, usize)>,
    pointer_drag_base_count: usize,
    pointer_restore_ranges: Vec<(usize, usize)>,
    pointer_restore_count: usize,
    pointer_restore_anchor: Option<usize>,
    pointer_text_selection_allowed: bool,
}

pub(crate) trait LogTableCursor {
    fn active_log_row(&self) -> Option<usize>;
    fn set_active_log_row(&self, row_ix: Option<usize>);
    fn suppress_next_table_clear(&self);
    fn take_suppressed_table_clear(&self) -> bool;
}

pub(crate) trait LogTableRows {
    fn reset_visible_log_row_owner(&mut self);

    fn schedule_visible_log_rows(
        &mut self,
        visible_range: Range<usize>,
        cx: &mut Context<TableState<Self>>,
    ) where
        Self: Sized + TableDelegate;
}

pub(crate) trait LogTableStateExt {
    fn active_log_row(&self) -> Option<usize>;
    fn set_active_log_row(&mut self, row_ix: usize, cx: &mut Context<Self>)
    where
        Self: Sized;
    fn sync_active_log_row(&mut self, cx: &mut Context<Self>) -> bool
    where
        Self: Sized;
    fn refresh_log_rows(&mut self, cx: &mut Context<Self>)
    where
        Self: Sized;
    /// Hands the unchanged visible window to the fixed table owner and invalidates its view in
    /// the same entity update.
    fn reacquire_visible_log_rows(&mut self, cx: &mut Context<Self>)
    where
        Self: Sized;
}

pub(crate) fn scroll_uniform_log_row_to_viewport_y(
    handle: &gpui::UniformListScrollHandle,
    row_ix: usize,
    viewport_y: Pixels,
    row_height: Pixels,
) {
    let base_handle = {
        let mut state = handle.0.borrow_mut();
        state.deferred_scroll_to_item = None;
        state.base_handle.clone()
    };
    let top = (row_height * row_ix as f32 - viewport_y).max(px(0.));
    base_handle.set_offset(point(base_handle.offset().x, -top));
}

impl<D> LogTableStateExt for TableState<D>
where
    D: TableDelegate + LogTableCursor + LogTableRows,
{
    fn active_log_row(&self) -> Option<usize> {
        self.delegate().active_log_row()
    }

    fn set_active_log_row(&mut self, row_ix: usize, cx: &mut Context<Self>) {
        self.delegate().set_active_log_row(Some(row_ix));
        self.delegate().suppress_next_table_clear();
        self.set_selected_row(row_ix, cx);
        self.clear_selection(cx);
    }

    fn sync_active_log_row(&mut self, cx: &mut Context<Self>) -> bool {
        if let Some(row_ix) = self.delegate().active_log_row() {
            self.set_active_log_row(row_ix, cx);
            true
        } else {
            self.delegate().suppress_next_table_clear();
            self.clear_selection(cx);
            false
        }
    }

    fn refresh_log_rows(&mut self, cx: &mut Context<Self>) {
        let visible_range = self.visible_range().rows().clone();
        self.delegate_mut()
            .schedule_visible_log_rows(visible_range, cx);
        self.refresh(cx);
    }

    fn reacquire_visible_log_rows(&mut self, cx: &mut Context<Self>) {
        let visible_range = self.visible_range().rows().clone();
        let delegate = self.delegate_mut();
        delegate.reset_visible_log_row_owner();
        delegate.schedule_visible_log_rows(visible_range, cx);
        self.refresh(cx);
        cx.notify();
    }
}

impl RowSelection {
    pub(crate) fn contains(&self, row_ix: usize) -> bool {
        self.ranges
            .partition_point(|(start, _)| *start <= row_ix)
            .checked_sub(1)
            .is_some_and(|range_ix| row_ix <= self.ranges[range_ix].1)
    }

    pub(crate) fn count(&self) -> usize {
        self.selected_count
    }

    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    fn advance_revision(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    fn replace_with(&mut self, start: usize, end: usize) {
        let range = ordered_range(start, end);
        self.ranges.clear();
        self.ranges.push(range);
        self.selected_count = inclusive_range_len(range);
        self.advance_revision();
    }

    fn add_range(&mut self, start: usize, end: usize) {
        let (mut start, mut end) = ordered_range(start, end);
        let first = self
            .ranges
            .partition_point(|(_, range_end)| range_end.saturating_add(1) < start);
        let mut after = first;
        while let Some((range_start, range_end)) = self.ranges.get(after).copied() {
            if end.saturating_add(1) < range_start {
                break;
            }
            start = start.min(range_start);
            end = end.max(range_end);
            after += 1;
        }
        let removed_count = self.ranges[first..after]
            .iter()
            .copied()
            .fold(0_usize, |count, range| {
                count.saturating_add(inclusive_range_len(range))
            });
        self.ranges.splice(first..after, [(start, end)]);
        self.selected_count = self
            .selected_count
            .saturating_sub(removed_count)
            .saturating_add(inclusive_range_len((start, end)));
        self.advance_revision();
    }

    fn toggle(&mut self, row_ix: usize) {
        let Some(range_ix) = self
            .ranges
            .partition_point(|(start, _)| *start <= row_ix)
            .checked_sub(1)
            .filter(|range_ix| row_ix <= self.ranges[*range_ix].1)
        else {
            self.add_range(row_ix, row_ix);
            return;
        };
        let (start, end) = self.ranges.remove(range_ix);
        self.selected_count = self.selected_count.saturating_sub(1);
        self.advance_revision();
        if start < row_ix {
            self.ranges.insert(range_ix, (start, row_ix - 1));
        }
        if row_ix < end {
            let insert_ix = range_ix + usize::from(start < row_ix);
            self.ranges.insert(insert_ix, (row_ix + 1, end));
        }
    }

    pub(crate) fn begin_pointer_selection(
        &mut self,
        row_ix: usize,
        control: bool,
        shift: bool,
        click_count: usize,
    ) {
        let allow_drag = click_count == 1;
        self.pending_pointer_row = Some(row_ix);
        let drag_anchor = if shift {
            self.anchor.unwrap_or(row_ix)
        } else {
            row_ix
        };
        if click_count >= 3 && !control && !shift {
            self.replace_with(row_ix, row_ix);
            self.anchor = Some(row_ix);
        } else if shift {
            let anchor = self.anchor.unwrap_or(row_ix);
            if control {
                self.add_range(anchor, row_ix);
            } else {
                self.replace_with(anchor, row_ix);
            }
        } else if control {
            self.toggle(row_ix);
            self.anchor = Some(row_ix);
        } else {
            self.replace_with(row_ix, row_ix);
            self.anchor = Some(row_ix);
        }
        self.pointer_drag_anchor = allow_drag.then_some(drag_anchor);
        self.pointer_drag_additive = allow_drag && control;
        self.pointer_drag_base_ranges = self.ranges.clone();
        self.pointer_drag_base_count = self.selected_count;
        self.pointer_restore_ranges = self.ranges.clone();
        self.pointer_restore_count = self.selected_count;
        self.pointer_restore_anchor = self.anchor;
        self.pointer_text_selection_allowed = allow_drag && !control && !shift;
    }

    pub(crate) fn extend_pointer_selection(&mut self, row_ix: usize) {
        let Some(anchor) = self.pointer_drag_anchor else {
            return;
        };
        self.pending_pointer_row = Some(row_ix);
        if self.pointer_drag_additive {
            self.ranges.clone_from(&self.pointer_drag_base_ranges);
            self.selected_count = self.pointer_drag_base_count;
            self.add_range(anchor, row_ix);
        } else {
            self.replace_with(anchor, row_ix);
        }
    }

    pub(crate) fn restore_pointer_selection(&mut self) {
        if self.pointer_drag_anchor.is_none() {
            return;
        }
        self.ranges.clone_from(&self.pointer_restore_ranges);
        self.selected_count = self.pointer_restore_count;
        self.advance_revision();
        self.anchor = self.pointer_restore_anchor;
        self.pending_pointer_row = self.pointer_drag_anchor;
    }

    pub(crate) fn end_pointer_selection(&mut self) {
        self.pending_pointer_row = None;
        self.pointer_drag_anchor = None;
        self.pointer_drag_additive = false;
        self.pointer_drag_base_ranges.clear();
        self.pointer_drag_base_count = 0;
        self.pointer_restore_ranges.clear();
        self.pointer_restore_count = 0;
        self.pointer_restore_anchor = None;
        self.pointer_text_selection_allowed = false;
    }

    pub(crate) fn settle_table_selection(&mut self, row_ix: usize) {
        if self.pending_pointer_row.take() != Some(row_ix) {
            self.replace_with(row_ix, row_ix);
            self.anchor = Some(row_ix);
        }
    }

    pub(crate) fn prepare_context_selection(&mut self, row_ix: usize) {
        if !self.contains(row_ix) {
            self.replace_with(row_ix, row_ix);
            self.anchor = Some(row_ix);
        }
        self.pending_pointer_row = Some(row_ix);
    }

    pub(crate) fn extend_keyboard_selection(&mut self, row_ix: usize) {
        let anchor = self.anchor.unwrap_or(row_ix);
        self.replace_with(anchor, row_ix);
        self.pending_pointer_row = Some(row_ix);
    }

    pub(crate) fn is_pointer_selecting(&self) -> bool {
        self.pointer_drag_anchor.is_some()
    }

    pub(crate) fn pointer_drag_anchor(&self) -> Option<usize> {
        self.pointer_drag_anchor
    }

    pub(crate) fn is_text_selection_allowed(&self) -> bool {
        self.pointer_text_selection_allowed
    }

    pub(crate) fn clear(&mut self) {
        self.ranges.clear();
        self.selected_count = 0;
        self.advance_revision();
        self.anchor = None;
        self.pending_pointer_row = None;
        self.pointer_drag_anchor = None;
        self.pointer_drag_additive = false;
        self.pointer_drag_base_ranges.clear();
        self.pointer_drag_base_count = 0;
        self.pointer_restore_ranges.clear();
        self.pointer_restore_count = 0;
        self.pointer_restore_anchor = None;
        self.pointer_text_selection_allowed = false;
    }

    pub(crate) fn selected_ranges(&self) -> impl Iterator<Item = (usize, usize)> + '_ {
        self.ranges.iter().copied()
    }

    pub(crate) fn anchor(&self) -> Option<usize> {
        self.anchor
    }

    pub(crate) fn replace_ranges_with_anchor(
        &mut self,
        ranges: impl IntoIterator<Item = (usize, usize)>,
        anchor: Option<usize>,
    ) {
        self.clear();
        for (start, end) in ranges {
            self.add_range(start, end);
        }
        self.anchor = anchor.or_else(|| self.ranges.first().map(|(start, _)| *start));
    }
}

fn ordered_range(first: usize, second: usize) -> (usize, usize) {
    (first.min(second), first.max(second))
}

fn inclusive_range_len((start, end): (usize, usize)) -> usize {
    end.saturating_sub(start).saturating_add(1)
}

pub(crate) fn combined_match_ranges(
    text: &str,
    color_rules: &ResolvedColorRules,
    search_matcher: Option<&SearchMatcher>,
    quick_find_matcher: Option<&SearchMatcher>,
) -> Vec<(Range<usize>, TextHighlight)> {
    #[derive(Clone, Copy)]
    struct Candidate {
        start: usize,
        end: usize,
        priority: usize,
        highlight: TextHighlight,
    }

    let mut color_matches = color_rules.matching_ranges(text);
    color_matches.sort_by(|(left, _, left_order), (right, _, right_order)| {
        left.start
            .cmp(&right.start)
            .then_with(|| right.len().cmp(&left.len()))
            .then_with(|| right_order.cmp(left_order))
    });

    let quick_ranges = quick_find_matcher
        .map(|matcher| matcher.matching_ranges(text))
        .unwrap_or_default();
    let mut candidates = quick_ranges
        .into_iter()
        .map(|range| Candidate {
            start: range.start,
            end: range.end,
            priority: 0,
            highlight: TextHighlight::QuickFind,
        })
        .collect::<Vec<_>>();
    candidates.extend(color_matches.into_iter().enumerate().map(
        |(priority, (range, color, _))| Candidate {
            start: range.start,
            end: range.end,
            priority: priority + 1,
            highlight: TextHighlight::Color(color),
        },
    ));
    let search_priority = candidates.len().saturating_add(1);
    let search_ranges = search_matcher
        .map(|matcher| matcher.matching_ranges(text))
        .unwrap_or_default();
    candidates.extend(search_ranges.into_iter().map(|range| Candidate {
        start: range.start,
        end: range.end,
        priority: search_priority,
        highlight: TextHighlight::Search,
    }));
    if candidates.is_empty() {
        return Vec::new();
    }

    let mut boundaries = candidates
        .iter()
        .flat_map(|candidate| [candidate.start, candidate.end])
        .collect::<Vec<_>>();
    boundaries.sort_unstable();
    boundaries.dedup();
    let mut starts = (0..candidates.len()).collect::<Vec<_>>();
    starts.sort_unstable_by_key(|candidate_ix| (candidates[*candidate_ix].start, *candidate_ix));
    let mut ends = (0..candidates.len()).collect::<Vec<_>>();
    ends.sort_unstable_by_key(|candidate_ix| (candidates[*candidate_ix].end, *candidate_ix));
    let mut start_cursor = 0;
    let mut end_cursor = 0;
    let mut active = vec![false; candidates.len()];
    let mut owners = BinaryHeap::<Reverse<(usize, usize)>>::new();
    let mut ranges: Vec<(Range<usize>, TextHighlight)> = Vec::new();
    for boundary in boundaries.windows(2) {
        let start = boundary[0];
        let end = boundary[1];
        while end_cursor < ends.len() && candidates[ends[end_cursor]].end <= start {
            active[ends[end_cursor]] = false;
            end_cursor += 1;
        }
        while start_cursor < starts.len() && candidates[starts[start_cursor]].start <= start {
            let candidate_ix = starts[start_cursor];
            active[candidate_ix] = true;
            owners.push(Reverse((candidates[candidate_ix].priority, candidate_ix)));
            start_cursor += 1;
        }
        while owners
            .peek()
            .is_some_and(|Reverse((_, candidate_ix))| !active[*candidate_ix])
        {
            owners.pop();
        }
        let Some(Reverse((_, owner_ix))) = owners.peek().copied() else {
            continue;
        };
        let owner = candidates[owner_ix];
        if let Some((previous, highlight)) = ranges.last_mut()
            && previous.end == start
            && *highlight == owner.highlight
        {
            previous.end = end;
        } else {
            ranges.push((start..end, owner.highlight));
        }
    }
    ranges
}

pub struct LogTableDelegate {
    source: LogRowSource,
    presenter: LogRowPresenter,
    interaction: LogInteractionState,
    visible_line_task: Option<Task<()>>,
}

struct LogRowSource {
    document_id: u64,
    document: Arc<LogDocument>,
    content_revision: u64,
    row_projection: LogRowProjection,
    visible_lines: VisibleLineStore<usize>,
}

struct LogRowPresenter {
    marked_rows: CompressedRows,
    empty_message: SharedString,
    show_line_numbers: bool,
    show_row_separators: bool,
    highlight_log_levels: bool,
    log_font_family: LogFontFamily,
    log_font_size: u16,
    log_line_spacing: u16,
    line_number_width: u16,
    line_number_text_color: Option<Hsla>,
    line_number_background_color: Option<Hsla>,
    show_line_number_row_separators: bool,
    search_matcher: Option<SearchMatcher>,
    matched_rows: CompressedRows,
    quick_find_matcher: Option<SearchMatcher>,
    color_rules: Arc<ResolvedColorRules>,
}

struct LogInteractionState {
    text_selections: TextSelectionCache<usize>,
    suppress_text_selection: Cell<bool>,
    word_boundary_characters: SharedString,
    row_selection: Rc<RefCell<RowSelection>>,
    active_row: Cell<Option<usize>>,
    suppress_table_clear: Cell<bool>,
    row_bounds: Rc<RefCell<BTreeMap<usize, Bounds<Pixels>>>>,
}

impl LogRowSource {
    fn new(document_id: u64, document: Arc<LogDocument>, row_projection: LogRowProjection) -> Self {
        Self {
            document_id,
            document,
            content_revision: 1,
            row_projection,
            visible_lines: VisibleLineStore::default(),
        }
    }

    fn replace(&mut self, document: Arc<LogDocument>, row_projection: LogRowProjection) {
        if !Arc::ptr_eq(&self.document, &document) {
            self.visible_lines.clear();
            self.content_revision = self.content_revision.saturating_add(1);
        } else {
            match &row_projection {
                LogRowProjection::All => self.visible_lines.invalidate_window(),
                LogRowProjection::SourceRows(source_rows) => self
                    .visible_lines
                    .retain(|source_row| source_rows.contains(*source_row)),
            }
        }
        self.document = document;
        self.row_projection = row_projection;
    }

    fn source_row(&self, row_ix: usize) -> Option<usize> {
        match &self.row_projection {
            LogRowProjection::All => self.document.source_row(row_ix),
            LogRowProjection::SourceRows(source_rows) => source_rows.get(row_ix),
        }
    }

    fn row_key(&self, row_ix: usize) -> Option<LogRowKey> {
        Some(LogRowKey::Row {
            document_id: self.document_id,
            source_row: self.source_row(row_ix)?,
        })
    }

    fn row_ix(&self, key: LogRowKey) -> Option<usize> {
        let LogRowKey::Row {
            document_id,
            source_row,
        } = key
        else {
            return None;
        };
        if document_id != self.document_id {
            return None;
        }
        match &self.row_projection {
            LogRowProjection::All => self.document.local_row(source_row),
            LogRowProjection::SourceRows(source_rows) => source_rows.position(source_row),
        }
    }

    fn row_count(&self) -> usize {
        match &self.row_projection {
            LogRowProjection::All => self.document.line_count(),
            LogRowProjection::SourceRows(source_rows) => source_rows.len(),
        }
    }

    fn selected_source_rows(
        &self,
        ranges: impl IntoIterator<Item = (usize, usize)>,
    ) -> CompressedRows {
        match &self.row_projection {
            LogRowProjection::All => {
                let row_count = self.row_count();
                CompressedRows::from_inclusive_ranges(ranges.into_iter().filter_map(
                    |(first, last)| {
                        let start = first.min(last);
                        if start >= row_count {
                            return None;
                        }
                        let end = first.max(last).min(row_count - 1);
                        Some((self.source_row(start)?, self.source_row(end)?))
                    },
                ))
            }
            LogRowProjection::SourceRows(source_rows) => {
                source_rows.rows_at_position_ranges(ranges)
            }
        }
    }

    fn position_ranges_for_selected_source_rows(
        &self,
        selected_rows: &CompressedRows,
    ) -> Vec<(usize, usize)> {
        let projection = match &self.row_projection {
            LogRowProjection::All => {
                let row_count = self.document.line_count();
                if row_count == 0 {
                    return Vec::new();
                }
                let first = self.document.segment_start_row();
                CompressedRows::from_inclusive_ranges([(
                    first,
                    first.saturating_add(row_count - 1),
                )])
            }
            LogRowProjection::SourceRows(source_rows) => source_rows.clone(),
        };
        projection.position_ranges_for_subset(selected_rows)
    }

    fn request_visible_rows(
        &self,
        visible_range: Range<usize>,
    ) -> Option<VisibleLineLoadRequest<usize>> {
        self.visible_lines
            .request_visible_rows(visible_range, self.row_count(), |row_ix| {
                self.source_row(row_ix)
            })
    }

    fn stage_visible_rows(
        &self,
        visible_range: Range<usize>,
    ) -> Option<StagedVisibleLineLoadRequest<usize>> {
        self.visible_lines
            .stage_visible_rows(visible_range, self.row_count(), |row_ix| {
                self.source_row(row_ix)
            })
    }

    fn line_text(&self, source_row: usize) -> Option<LogText> {
        (!self.visible_lines.source_unavailable(source_row))
            .then(|| self.visible_lines.line(source_row))
            .flatten()
    }
}

impl LogRowPresenter {
    fn new(empty_message: SharedString) -> Self {
        Self {
            marked_rows: CompressedRows::default(),
            empty_message,
            show_line_numbers: true,
            show_row_separators: false,
            highlight_log_levels: false,
            log_font_family: LogFontFamily::default(),
            log_font_size: 13,
            log_line_spacing: 6,
            line_number_width: 60,
            line_number_text_color: None,
            line_number_background_color: None,
            show_line_number_row_separators: false,
            search_matcher: None,
            matched_rows: CompressedRows::default(),
            quick_find_matcher: None,
            color_rules: Arc::default(),
        }
    }

    fn present(&self, text: LogText) -> LogRowPresentation {
        let source_highlights = combined_match_ranges(
            text.source(),
            &self.color_rules,
            self.search_matcher.as_ref(),
            self.quick_find_matcher.as_ref(),
        );
        LogRowPresentation {
            highlights: source_highlights
                .into_iter()
                .filter_map(|(range, highlight)| {
                    text.display_range(range).map(|range| (range, highlight))
                })
                .collect(),
            text,
            source_unavailable: false,
        }
    }
}

impl Default for LogInteractionState {
    fn default() -> Self {
        Self {
            text_selections: TextSelectionCache::default(),
            suppress_text_selection: Cell::default(),
            word_boundary_characters: DEFAULT_WORD_BOUNDARY_CHARACTERS.into(),
            row_selection: Rc::default(),
            active_row: Cell::default(),
            suppress_table_clear: Cell::default(),
            row_bounds: Rc::default(),
        }
    }
}

#[derive(Clone)]
struct LogRowPresentation {
    text: LogText,
    highlights: Arc<[(Range<usize>, TextHighlight)]>,
    source_unavailable: bool,
}

pub(crate) struct WrappedLogRow {
    pub source_row: usize,
    pub text: LogText,
    pub selected: bool,
    pub marked: bool,
    pub matched: bool,
    pub highlight_severity: bool,
    pub source_unavailable: bool,
    pub highlights: Arc<[(Range<usize>, TextHighlight)]>,
}

impl LogTableDelegate {
    pub fn all(document_id: u64, document: Arc<LogDocument>) -> Self {
        Self {
            source: LogRowSource::new(document_id, document, LogRowProjection::All),
            presenter: LogRowPresenter::new(
                crate::tr!("文件中没有日志行", "The file has no log lines").into(),
            ),
            interaction: LogInteractionState::default(),
            visible_line_task: None,
        }
    }

    pub fn projected(
        document_id: u64,
        document: Arc<LogDocument>,
        source_rows: CompressedRows,
    ) -> Self {
        Self {
            source: LogRowSource::new(
                document_id,
                document,
                LogRowProjection::SourceRows(source_rows),
            ),
            presenter: LogRowPresenter::new(
                crate::tr!("没有匹配的日志行", "No log lines match").into(),
            ),
            interaction: LogInteractionState::default(),
            visible_line_task: None,
        }
    }

    pub fn set_row_projection(&mut self, source_rows: CompressedRows) {
        self.interaction
            .row_selection
            .borrow_mut()
            .end_pointer_selection();
        if matches!(
            &self.source.row_projection,
            LogRowProjection::SourceRows(current) if current == &source_rows
        ) {
            return;
        }
        let (selected_rows, active_row, selection_anchor) = self.stable_interaction_rows();
        self.source
            .visible_lines
            .retain(|source_row| source_rows.contains(*source_row));
        self.source.row_projection = LogRowProjection::SourceRows(source_rows);
        self.interaction.row_bounds.borrow_mut().clear();
        self.restore_stable_interaction_rows(selected_rows, active_row, selection_anchor);
    }

    pub fn set_marked_rows(&mut self, marked_rows: CompressedRows) {
        self.presenter.marked_rows = marked_rows;
    }

    pub fn set_view_options(&mut self, show_line_numbers: bool, show_row_separators: bool) {
        self.presenter.show_line_numbers = show_line_numbers;
        self.presenter.show_row_separators = show_row_separators;
    }

    pub fn set_highlight_log_levels(&mut self, enabled: bool) {
        self.presenter.highlight_log_levels = enabled;
    }

    pub fn set_appearance(&mut self, settings: &AppSettings) {
        self.presenter.log_font_family = settings.log_font_family;
        self.presenter.log_font_size = settings.log_font_size.clamp(8, 32);
        self.presenter.log_line_spacing = settings.log_line_spacing.clamp(1, 40);
        self.presenter.line_number_width = settings.line_number_width.clamp(40, 160);
        self.presenter.line_number_text_color = settings
            .line_number_text_color
            .as_deref()
            .and_then(|value| try_parse_color(value).ok());
        self.presenter.line_number_background_color = settings
            .line_number_background_color
            .as_deref()
            .and_then(|value| try_parse_color(value).ok());
        self.presenter.show_line_number_row_separators = settings.show_line_number_row_separators;
        self.source
            .visible_lines
            .set_overscan(usize::from(settings.viewer_overscan.clamp(4, 40)));
    }

    pub fn set_word_boundary_characters(&mut self, characters: impl Into<SharedString>) {
        self.interaction.word_boundary_characters = characters.into();
    }

    pub(crate) fn resolved_font_family(&self, cx: &App) -> SharedString {
        match self.presenter.log_font_family {
            LogFontFamily::CascadiaMono => "Cascadia Mono".into(),
            LogFontFamily::JetBrainsMono => "JetBrains Mono".into(),
            LogFontFamily::Consolas => "Consolas".into(),
            LogFontFamily::SystemMonospace => cx.theme().mono_font_family.clone(),
        }
    }

    pub(crate) fn content_revision(&self) -> u64 {
        self.source.content_revision
    }

    pub fn set_search_matcher(&mut self, search_matcher: Option<SearchMatcher>) {
        self.presenter.search_matcher = search_matcher;
    }

    pub fn set_matched_rows(&mut self, matched_rows: CompressedRows) {
        self.presenter.matched_rows = matched_rows;
    }

    pub fn set_quick_find_matcher(&mut self, quick_find_matcher: Option<SearchMatcher>) {
        self.presenter.quick_find_matcher = quick_find_matcher;
    }

    pub fn set_color_rules(&mut self, color_rules: Arc<ResolvedColorRules>) {
        self.presenter.color_rules = color_rules;
    }

    pub fn replace_with_all(&mut self, document: Arc<LogDocument>) {
        self.replace_source(document, LogRowProjection::All);
    }

    pub fn replace_with_rows(&mut self, document: Arc<LogDocument>, source_rows: CompressedRows) {
        self.replace_source(document, LogRowProjection::SourceRows(source_rows));
    }

    fn replace_source(&mut self, document: Arc<LogDocument>, row_projection: LogRowProjection) {
        self.interaction
            .row_selection
            .borrow_mut()
            .end_pointer_selection();
        let (selected_rows, active_row, selection_anchor) = self.stable_interaction_rows();
        self.source.replace(document, row_projection);
        self.interaction.text_selections.clear();
        self.interaction.row_bounds.borrow_mut().clear();
        self.restore_stable_interaction_rows(selected_rows, active_row, selection_anchor);
    }

    fn stable_interaction_rows(&self) -> (CompressedRows, Option<LogRowKey>, Option<LogRowKey>) {
        let selection = self.interaction.row_selection.borrow();
        let selected_rows = self
            .source
            .selected_source_rows(selection.selected_ranges());
        let selection_anchor = selection
            .anchor()
            .and_then(|row_ix| self.source.row_key(row_ix));
        let active_row = self
            .interaction
            .active_row
            .get()
            .and_then(|row_ix| self.source.row_key(row_ix));
        (selected_rows, active_row, selection_anchor)
    }

    fn restore_stable_interaction_rows(
        &self,
        selected_rows: CompressedRows,
        active_row: Option<LogRowKey>,
        selection_anchor: Option<LogRowKey>,
    ) {
        let selected_ranges = self
            .source
            .position_ranges_for_selected_source_rows(&selected_rows);
        let anchor = selection_anchor.and_then(|key| self.source.row_ix(key));
        self.interaction
            .row_selection
            .borrow_mut()
            .replace_ranges_with_anchor(selected_ranges, anchor);
        self.interaction
            .active_row
            .set(active_row.and_then(|key| self.source.row_ix(key)));
    }

    pub fn source_row(&self, row_ix: usize) -> Option<usize> {
        self.source.source_row(row_ix)
    }

    pub(crate) fn row_key(&self, row_ix: usize) -> Option<LogRowKey> {
        self.source.row_key(row_ix)
    }

    pub(crate) fn row_ix_for_key(&self, key: LogRowKey) -> Option<usize> {
        self.source.row_ix(key)
    }

    pub(crate) fn projected_rows(&self) -> Option<&CompressedRows> {
        match &self.source.row_projection {
            LogRowProjection::All => None,
            LogRowProjection::SourceRows(rows) => Some(rows),
        }
    }

    pub(crate) fn row_bounds_handle(&self) -> Rc<RefCell<BTreeMap<usize, Bounds<Pixels>>>> {
        self.interaction.row_bounds.clone()
    }

    pub(crate) fn wrapped_row(&self, row_ix: usize) -> Option<WrappedLogRow> {
        let source_row = self.source_row(row_ix)?;
        let presentation = self.row_presentation(source_row)?;
        Some(WrappedLogRow {
            source_row,
            selected: self.interaction.row_selection.borrow().contains(row_ix),
            marked: self.presenter.marked_rows.contains(source_row),
            matched: self.presenter.matched_rows.contains(source_row),
            highlight_severity: self.presenter.highlight_log_levels,
            source_unavailable: presentation.source_unavailable,
            highlights: presentation.highlights,
            text: presentation.text,
        })
    }

    pub(crate) fn request_visible_rows(
        &self,
        visible_range: Range<usize>,
    ) -> Option<VisibleLineLoadRequest<usize>> {
        self.source.request_visible_rows(visible_range)
    }

    pub(crate) fn stage_visible_rows(
        &self,
        visible_range: Range<usize>,
    ) -> Option<StagedVisibleLineLoadRequest<usize>> {
        self.source.stage_visible_rows(visible_range)
    }

    pub(crate) fn visible_document(&self) -> Arc<LogDocument> {
        self.source.document.clone()
    }

    pub(crate) fn visible_line_revision(&self) -> u64 {
        self.source.visible_lines.revision()
    }

    pub(crate) fn install_visible_lines(&self, loaded: VisibleLineLoadResult<usize>) -> bool {
        self.source.visible_lines.install_loaded(loaded)
    }

    pub(crate) fn install_staged_visible_lines(&self, loaded: StagedVisibleLineLoadResult<usize>) {
        self.source.visible_lines.install_staged(loaded);
    }

    pub(crate) fn reset_visible_line_owner(&mut self) {
        self.visible_line_task = None;
        self.source.visible_lines.invalidate_window();
    }

    pub(crate) fn clear_visible_lines(&mut self) {
        self.visible_line_task = None;
        self.source.visible_lines.clear();
    }

    fn schedule_visible_rows(
        &mut self,
        visible_range: Range<usize>,
        cx: &mut Context<TableState<Self>>,
    ) {
        let Some(request) = self.request_visible_rows(visible_range) else {
            return;
        };
        let document = self.visible_document();
        self.visible_line_task = Some(cx.spawn(async move |table, cx| {
            let loaded = cx
                .background_spawn(async move {
                    let mut reader = LinePreviewReader::default();
                    request.load(|source_row, max_bytes| {
                        reader.line_preview(&document, *source_row, max_bytes)
                    })
                })
                .await;
            _ = table.update(cx, |table, cx| {
                if table.delegate().install_visible_lines(loaded) {
                    cx.notify();
                }
            });
        }));
    }

    fn line_text(&self, source_row: usize) -> Option<LogText> {
        self.source.line_text(source_row)
    }

    fn row_presentation(&self, source_row: usize) -> Option<LogRowPresentation> {
        let text = self.source.visible_lines.line(source_row)?;
        if self.source.visible_lines.source_unavailable(source_row) {
            return Some(LogRowPresentation {
                text,
                highlights: Arc::default(),
                source_unavailable: true,
            });
        }
        Some(self.presenter.present(text))
    }

    pub(crate) fn begin_pointer_selection(
        &self,
        row_ix: usize,
        control: bool,
        shift: bool,
        click_count: usize,
    ) {
        // A new gesture must not inherit line-drag suppression after a lost MouseUp.
        self.interaction.suppress_text_selection.set(false);
        self.interaction
            .row_selection
            .borrow_mut()
            .begin_pointer_selection(row_ix, control, shift, click_count);
    }

    pub(crate) fn prepare_context_selection(&self, row_ix: usize) {
        self.interaction
            .row_selection
            .borrow_mut()
            .prepare_context_selection(row_ix);
    }

    pub(crate) fn extend_keyboard_selection(&self, row_ix: usize) {
        self.interaction
            .row_selection
            .borrow_mut()
            .extend_keyboard_selection(row_ix);
    }

    pub(crate) fn extend_pointer_selection(&self, row_ix: usize) {
        self.interaction
            .row_selection
            .borrow_mut()
            .extend_pointer_selection(row_ix);
    }

    pub(crate) fn restore_pointer_selection(&self) {
        self.interaction
            .row_selection
            .borrow_mut()
            .restore_pointer_selection();
    }

    pub(crate) fn end_pointer_selection(&self) {
        self.interaction
            .row_selection
            .borrow_mut()
            .end_pointer_selection();
        self.interaction.suppress_text_selection.set(false);
    }

    pub(crate) fn is_pointer_selecting(&self) -> bool {
        self.interaction
            .row_selection
            .borrow()
            .pointer_drag_anchor
            .is_some()
    }

    pub(crate) fn pointer_drag_anchor(&self) -> Option<usize> {
        self.interaction.row_selection.borrow().pointer_drag_anchor
    }

    pub(crate) fn pointer_text_selection_allowed(&self) -> bool {
        self.interaction
            .row_selection
            .borrow()
            .pointer_text_selection_allowed
    }

    pub(crate) fn set_text_selection_suppressed(&self, suppressed: bool) {
        self.interaction.suppress_text_selection.set(suppressed);
    }

    pub(crate) fn is_text_selection_suppressed(&self) -> bool {
        self.interaction.suppress_text_selection.get()
    }

    pub(crate) fn show_line_numbers(&self) -> bool {
        self.presenter.show_line_numbers
    }

    pub(crate) fn show_row_separators(&self) -> bool {
        self.presenter.show_row_separators
    }

    pub(crate) fn line_number_width(&self) -> u16 {
        self.presenter.line_number_width
    }

    pub(crate) fn line_number_text_color(&self, cx: &App) -> Hsla {
        self.presenter
            .line_number_text_color
            .unwrap_or_else(|| ui_theme::palette(cx).line_number)
    }

    pub(crate) fn line_number_background_color(&self, cx: &App) -> Hsla {
        self.presenter
            .line_number_background_color
            .unwrap_or_else(|| ui_theme::palette(cx).line_number_background)
    }

    pub(crate) fn show_line_number_row_separators(&self) -> bool {
        self.presenter.show_line_number_row_separators
    }

    pub(crate) fn log_font_size(&self) -> u16 {
        self.presenter.log_font_size
    }

    pub fn settle_table_selection(&self, row_ix: usize) -> Option<usize> {
        self.interaction
            .row_selection
            .borrow_mut()
            .settle_table_selection(row_ix);
        self.source_row(row_ix)
    }

    pub fn clear_row_selection(&self) {
        self.interaction.row_selection.borrow_mut().clear();
    }

    #[cfg(test)]
    pub fn selected_source_rows(&self) -> Vec<usize> {
        self.selected_source_rows_compressed().iter().collect()
    }

    pub(crate) fn selected_source_rows_compressed(&self) -> CompressedRows {
        let selection = self.interaction.row_selection.borrow();
        self.source
            .selected_source_rows(selection.selected_ranges())
    }

    pub fn selected_rows_count(&self) -> usize {
        self.interaction
            .row_selection
            .borrow()
            .count()
            .min(self.row_count())
    }

    pub(crate) fn is_row_selected(&self, row_ix: usize) -> bool {
        self.interaction.row_selection.borrow().contains(row_ix)
    }

    pub fn select_all_rows(&self) {
        let row_count = self.row_count();
        if row_count == 0 {
            self.interaction.row_selection.borrow_mut().clear();
        } else {
            let mut selection = self.interaction.row_selection.borrow_mut();
            selection.replace_with(0, row_count - 1);
            selection.anchor = Some(0);
        }
    }

    pub(crate) fn row_count(&self) -> usize {
        self.source.row_count()
    }
}

impl LogTableCursor for LogTableDelegate {
    fn active_log_row(&self) -> Option<usize> {
        self.interaction.active_row.get()
    }

    fn set_active_log_row(&self, row_ix: Option<usize>) {
        self.interaction.active_row.set(row_ix);
    }

    fn suppress_next_table_clear(&self) {
        self.interaction.suppress_table_clear.set(true);
    }

    fn take_suppressed_table_clear(&self) -> bool {
        self.interaction.suppress_table_clear.replace(false)
    }
}

impl LogTableRows for LogTableDelegate {
    fn reset_visible_log_row_owner(&mut self) {
        self.reset_visible_line_owner();
    }

    fn schedule_visible_log_rows(
        &mut self,
        visible_range: Range<usize>,
        cx: &mut Context<TableState<Self>>,
    ) {
        self.schedule_visible_rows(visible_range, cx);
    }
}

impl TableDelegate for LogTableDelegate {
    fn columns_count(&self, _: &App) -> usize {
        if self.presenter.show_line_numbers {
            3
        } else {
            2
        }
    }

    fn rows_count(&self, _: &App) -> usize {
        self.row_count()
    }

    fn column(&self, col_ix: usize, cx: &App) -> Column {
        let base = px(self.presenter.log_font_size as f32);
        if col_ix == 0 {
            let width = line_marker_column_width();
            Column::new("marker", crate::tr!("标记", "Mark"))
                .p_0()
                .width(width)
                .min_width(width)
                .max_width(width)
                .resizable(false)
                .fixed_left()
                .movable(false)
        } else if self.presenter.show_line_numbers && col_ix == 1 {
            let width = px(self.presenter.line_number_width as f32);
            Column::new("line-number", crate::tr!("行", "Line"))
                .p_0()
                .width(width)
                .min_width(width)
                .max_width(width)
                .resizable(false)
                .fixed_left()
                .movable(false)
                .text_right()
        } else if col_ix == 1 + usize::from(self.presenter.show_line_numbers) {
            Column::new("message", crate::tr!("日志", "Log"))
                .p_0()
                .width(message_column_width(
                    self.source.document.metadata().longest_line_columns,
                    self.resolved_font_family(cx),
                    self.presenter.log_font_size,
                    cx,
                ))
                .min_width(base * 24.)
                .movable(false)
        } else {
            unreachable!("log table exposes marker, message, and optional line-number columns")
        }
    }

    fn render_header(
        &mut self,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> Stateful<Div> {
        div().id("header").hidden()
    }

    fn render_tr(
        &mut self,
        row_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> Stateful<Div> {
        let _performance_scope = crate::ui_performance::scope("LogTableDelegate::render_tr");
        let source_row = self.source_row(row_ix);
        let source_row_ix = source_row.unwrap_or(row_ix);
        let severity = source_row
            .filter(|_| self.presenter.highlight_log_levels)
            .and_then(|source_row| self.line_text(source_row))
            .and_then(|line| line.severity())
            .map(|severity| severity_style(severity, cx));
        let row_bounds = self.interaction.row_bounds.clone();
        div()
            .id(format!(
                "document-{}-row-{source_row_ix}",
                self.source.document_id
            ))
            .border_0()
            .on_prepaint(move |bounds, _, _| {
                row_bounds.borrow_mut().insert(row_ix, bounds);
            })
            .when_some(severity, |row, style| row.bg(style.background))
            .when(source_row.is_some(), |row| {
                row.on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |table, event: &MouseDownEvent, window, cx| {
                        if event.modifiers.control
                            || event.modifiers.shift
                            || event.click_count >= 3
                        {
                            GlobalState::suppress_text_selection(cx);
                            TextSelection::clear(window, cx);
                        }
                        table.delegate().begin_pointer_selection(
                            row_ix,
                            event.modifiers.control,
                            event.modifiers.shift,
                            event.click_count,
                        );
                        let table = cx.entity();
                        window.defer(cx, move |_, cx| {
                            table.update(cx, |table, cx| {
                                table.set_active_log_row(row_ix, cx);
                            });
                        });
                        cx.notify();
                    }),
                )
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |table, _: &MouseDownEvent, _, cx| {
                        table.delegate().prepare_context_selection(row_ix);
                        table.set_active_log_row(row_ix, cx);
                        cx.notify();
                    }),
                )
            })
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let _performance_scope = crate::ui_performance::scope("LogTableDelegate::render_td");
        let source_row = self.source_row(row_ix).unwrap_or(row_ix);
        let selected = self.interaction.row_selection.borrow().contains(row_ix);
        let line_height = log_line_height(
            self.presenter.log_font_size,
            self.presenter.log_line_spacing,
        );
        if col_ix == 0 {
            let marked = self.presenter.marked_rows.contains(source_row);
            let matched = self.presenter.matched_rows.contains(source_row);
            let severity_accent = self
                .presenter
                .highlight_log_levels
                .then(|| self.line_text(source_row))
                .flatten()
                .and_then(|line| line.severity())
                .map(|severity| severity_style(severity, cx))
                .map(|style| style.accent);
            h_flex()
                .relative()
                .size_full()
                .justify_center()
                .when_some(severity_accent, |cell, accent| {
                    cell.child(severity_accent_overlay(accent))
                })
                .child(line_marker(marked, matched, cx))
                .into_any_element()
        } else if self.presenter.show_line_numbers && col_ix == 1 {
            log_line_number_cell(
                source_row,
                self.presenter.log_font_size,
                line_height,
                self.line_number_text_color(cx),
                self.line_number_background_color(cx),
                self.show_line_number_row_separators(),
                cx,
            )
            .size_full()
            .into_any_element()
        } else if col_ix == 1 + usize::from(self.presenter.show_line_numbers) {
            let presentation =
                self.row_presentation(source_row)
                    .unwrap_or_else(|| LogRowPresentation {
                        text: LogText::default(),
                        highlights: Arc::default(),
                        source_unavailable: false,
                    });
            let source_unavailable = presentation.source_unavailable;
            let text = presentation.text;
            let highlights = presentation
                .highlights
                .iter()
                .cloned()
                .map(|(range, highlight)| (range, text_highlight_style(highlight, cx)))
                .collect::<Vec<_>>();
            let styled_text = StyledText::new(text.display().clone()).with_highlights(highlights);
            let selection = self
                .interaction
                .text_selections
                .handle(source_row, &text, window, cx);
            h_flex()
                .relative()
                .size_full()
                .overflow_hidden()
                .px(log_cell_horizontal_padding(cx))
                .when(selected, |cell| {
                    cell.bg(log_row_selection_color(cx))
                        .child(log_row_selection_overlay(
                            row_ix == 0
                                || !self.interaction.row_selection.borrow().contains(row_ix - 1),
                            row_ix + 1 >= self.row_count()
                                || !self.interaction.row_selection.borrow().contains(row_ix + 1),
                            cx,
                        ))
                })
                .text_size(px(self.presenter.log_font_size as f32))
                .line_height(line_height)
                .font_family(self.resolved_font_family(cx))
                .when(source_unavailable, |cell| {
                    cell.text_color(cx.theme().danger)
                })
                .when(self.presenter.show_row_separators && !selected, |cell| {
                    cell.border_b_1().border_color(cx.theme().border)
                })
                .child(
                    SelectableLogText::new(
                        selection,
                        source_row as u64,
                        text,
                        styled_text,
                        ui_theme::text_selection_highlight(cx),
                    )
                    .suppress_selection(self.is_text_selection_suppressed())
                    .word_boundary_characters(self.interaction.word_boundary_characters.clone()),
                )
                .into_any_element()
        } else {
            unreachable!("log table exposes marker, message, and optional line-number columns")
        }
    }

    fn render_empty(
        &mut self,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_2()
            .text_color(cx.theme().muted_foreground)
            .child(self.presenter.empty_message.clone())
    }

    fn visible_rows_changed(
        &mut self,
        visible_range: Range<usize>,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) {
        self.interaction
            .row_bounds
            .borrow_mut()
            .retain(|row_ix, _| visible_range.contains(row_ix));
        self.schedule_visible_rows(visible_range, cx);
    }

    fn cell_text(&self, row_ix: usize, col_ix: usize, _: &App) -> String {
        let Some(source_row) = self.source_row(row_ix) else {
            return String::new();
        };
        if self.presenter.show_line_numbers && col_ix == 0 {
            (source_row + 1).to_string()
        } else if col_ix == usize::from(self.presenter.show_line_numbers) {
            self.source
                .visible_lines
                .line(source_row)
                .map(|text| text.display().to_string())
                .unwrap_or_default()
        } else {
            String::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_selection_finds_sparse_ranges_at_binary_boundaries() {
        let selection = RowSelection {
            ranges: (0..100_000).map(|index| (index * 3, index * 3)).collect(),
            selected_count: 100_000,
            ..Default::default()
        };

        assert!(selection.contains(0));
        assert!(selection.contains(150_000));
        assert!(selection.contains(299_997));
        assert!(!selection.contains(1));
        assert!(!selection.contains(299_998));
        assert!(!selection.contains(usize::MAX));
        assert_eq!(selection.count(), 100_000);
    }

    #[test]
    fn row_selection_keeps_cached_count_in_sync() {
        let mut selection = RowSelection::default();
        selection.replace_ranges_with_anchor([(1, 3), (10, 12)], None);
        assert_eq!(selection.count(), 6);

        selection.add_range(3, 10);
        assert_eq!(
            selection.selected_ranges().collect::<Vec<_>>(),
            vec![(1, 12)]
        );
        assert_eq!(selection.count(), 12);

        selection.toggle(5);
        assert_eq!(selection.count(), 11);
        selection.toggle(5);
        assert_eq!(selection.count(), 12);

        selection.begin_pointer_selection(20, true, false, 1);
        assert_eq!(selection.count(), 13);
        selection.extend_pointer_selection(22);
        assert_eq!(selection.count(), 15);
        selection.restore_pointer_selection();
        assert_eq!(selection.count(), 13);

        selection.clear();
        assert_eq!(selection.count(), 0);
    }

    #[test]
    fn message_width_cannot_exceed_the_visible_line_preview() {
        assert_eq!(bounded_message_columns(120), 120);
        assert_eq!(
            bounded_message_columns(usize::MAX),
            MAX_VISIBLE_LINE_COLUMNS
        );
    }

    #[test]
    fn highlight_sweep_preserves_quick_color_and_search_priority() {
        let color_rules = crate::color_labels::resolve_color_rules(
            &[crate::color_labels::KeywordColorRule {
                label_id: None,
                keyword: "bcde".to_string(),
                color: 0xff0000,
                alpha: u8::MAX,
                case_sensitive: true,
                enabled: true,
            }],
            &[],
        );
        let search = SearchMatcher::literal_phrase("cdef")
            .expect("search matcher should compile")
            .expect("search text is non-empty");
        let quick = SearchMatcher::literal_phrase("d")
            .expect("quick-find matcher should compile")
            .expect("quick-find text is non-empty");

        let highlights = combined_match_ranges("abcdef", &color_rules, Some(&search), Some(&quick));

        assert_eq!(
            highlights
                .iter()
                .map(|(range, highlight)| (
                    range.clone(),
                    matches!(highlight, TextHighlight::QuickFind),
                    matches!(highlight, TextHighlight::Search)
                ))
                .collect::<Vec<_>>(),
            vec![
                (1..3, false, false),
                (3..4, true, false),
                (4..5, false, false),
                (5..6, false, true),
            ]
        );
    }

    #[test]
    fn presentation_changes_do_not_invalidate_decoded_lines() {
        let document = Arc::new(LogDocument::placeholder("presentation-state.log"));
        let mut delegate = LogTableDelegate::all(1, document);
        delegate.source.visible_lines.prepare_visible_rows(
            0..1,
            1,
            |_| Some(7),
            |_, _| Some(LinePreview::new("cached line", false)),
        );
        let cached = delegate
            .source
            .visible_lines
            .line(7)
            .expect("the test line should be cached");
        assert!(
            delegate
                .row_presentation(7)
                .expect("the cached line should be presentable")
                .highlights
                .is_empty()
        );

        delegate.set_view_options(false, true);
        delegate.set_highlight_log_levels(true);
        delegate.set_search_matcher(
            SearchMatcher::literal_phrase("line").expect("the search matcher should compile"),
        );
        delegate.set_quick_find_matcher(None);
        delegate.set_color_rules(Arc::default());
        delegate.set_marked_rows([7].into_iter().collect());
        delegate.set_appearance(&AppSettings::default());

        let reused = delegate
            .source
            .visible_lines
            .line(7)
            .expect("the cached line should remain available");
        assert_eq!(reused.source(), cached.source());
        assert_eq!(reused.source().as_ref(), "cached line");
        assert!(
            delegate
                .row_presentation(7)
                .expect("the cached line should reflect current presentation state")
                .highlights
                .iter()
                .any(|(_, highlight)| *highlight == TextHighlight::Search)
        );
    }

    #[test]
    fn unavailable_source_rows_bypass_log_highlighting() {
        let document = Arc::new(LogDocument::placeholder("unavailable-source.log"));
        let mut delegate = LogTableDelegate::all(1, document);
        delegate.set_search_matcher(
            SearchMatcher::literal_phrase("source").expect("搜索匹配器应能编译"),
        );
        delegate
            .source
            .visible_lines
            .prepare_visible_rows(0..1, 1, |_| Some(7), |_, _| None);

        let presentation = delegate
            .row_presentation(7)
            .expect("不可用源行应有显式呈现");
        assert!(presentation.source_unavailable);
        assert!(presentation.highlights.is_empty());
        assert!(!presentation.text.display().is_empty());
        assert!(delegate.line_text(7).is_none());
    }

    #[test]
    fn visible_line_owner_reset_reissues_the_same_window() {
        let document = Arc::new(LogDocument::placeholder("visible-owner-reset.log"));
        let mut delegate = LogTableDelegate::projected(1, document, [7].into_iter().collect());
        let stale = delegate
            .request_visible_rows(0..1)
            .expect("首个后端应请求可见行")
            .load(|_, _| Some(LinePreview::new("stale", false)));

        delegate.reset_visible_line_owner();

        assert!(delegate.request_visible_rows(0..1).is_some());
        assert!(!delegate.install_visible_lines(stale));
    }

    #[test]
    fn staged_tab_frame_revision_detects_projection_changes() {
        let document = Arc::new(LogDocument::placeholder("staged-tab-frame.log"));
        let mut delegate = LogTableDelegate::projected(1, document, [3, 7].into_iter().collect());
        let frame_revision = delegate.visible_line_revision();
        let _staged = delegate
            .stage_visible_rows(0..2)
            .expect("非空标签帧应产生预加载请求");

        delegate.set_row_projection([7, 9].into_iter().collect());

        assert_ne!(delegate.visible_line_revision(), frame_revision);
    }

    #[test]
    fn projected_rows_map_virtual_positions_to_requested_source_rows() {
        let document = Arc::new(LogDocument::placeholder("projected-rows.log"));
        let delegate = LogTableDelegate::projected(1, document, [3, 9, 27].into_iter().collect());

        assert_eq!(delegate.row_count(), 3);
        assert_eq!(delegate.source_row(0), Some(3));
        assert_eq!(delegate.source_row(1), Some(9));
        assert_eq!(delegate.source_row(2), Some(27));
        assert_eq!(delegate.source_row(3), None);
        assert_eq!(
            delegate.row_ix_for_key(LogRowKey::Row {
                document_id: 1,
                source_row: 27,
            }),
            Some(2)
        );
        assert_eq!(
            delegate.row_ix_for_key(LogRowKey::Row {
                document_id: 1,
                source_row: 8,
            }),
            None
        );
        assert_eq!(
            delegate
                .projected_rows()
                .expect("projected delegate must own the visible rows")
                .iter()
                .collect::<Vec<_>>(),
            vec![3, 9, 27]
        );
    }

    #[test]
    fn projection_updates_migrate_selection_by_stable_row_key() {
        let document = Arc::new(LogDocument::placeholder("stable-projection.log"));
        let mut delegate =
            LogTableDelegate::projected(7, document, [3, 9, 27].into_iter().collect());
        assert_eq!(delegate.settle_table_selection(1), Some(9));
        delegate.set_active_log_row(Some(1));

        delegate.set_row_projection([1, 3, 9, 30].into_iter().collect());

        assert_eq!(delegate.selected_source_rows(), vec![9]);
        assert_eq!(delegate.active_log_row(), Some(2));
        assert_eq!(
            delegate.interaction.row_selection.borrow().anchor(),
            Some(2)
        );

        delegate.set_row_projection([1, 30].into_iter().collect());

        assert!(delegate.selected_source_rows().is_empty());
        assert_eq!(delegate.active_log_row(), None);
    }

    #[test]
    fn projection_updates_do_not_select_new_rows_between_selected_rows() {
        let document = Arc::new(LogDocument::placeholder("exact-stable-projection.log"));
        let mut delegate = LogTableDelegate::projected(7, document, [2, 9].into_iter().collect());
        assert_eq!(delegate.settle_table_selection(0), Some(2));
        delegate.extend_keyboard_selection(1);

        delegate.set_row_projection([2, 5, 9].into_iter().collect());

        assert_eq!(delegate.selected_source_rows(), [2, 9]);
        assert_eq!(
            delegate
                .interaction
                .row_selection
                .borrow()
                .selected_ranges()
                .collect::<Vec<_>>(),
            [(0, 0), (2, 2)]
        );
    }

    #[test]
    fn select_all_snapshot_keeps_large_result_projection_compressed() {
        let document = Arc::new(LogDocument::placeholder("large-selection.log"));
        let rows = CompressedRows::from_inclusive_ranges([(0, 999_999)]);
        let delegate = LogTableDelegate::projected(7, document, rows);

        delegate.select_all_rows();
        let selected = delegate.selected_source_rows_compressed();

        assert_eq!(selected.len(), 1_000_000);
    }

    #[test]
    fn equivalent_projection_preserves_the_prepared_virtual_window() {
        let document = Arc::new(LogDocument::placeholder("stable-result-window.log"));
        let mut delegate = LogTableDelegate::projected(7, document, [3, 9].into_iter().collect());
        delegate.source.visible_lines.prepare_visible_rows(
            0..1,
            2,
            |row_ix| [3, 9].get(row_ix).copied(),
            |source_row, _| Some(LinePreview::new(format!("line {source_row}"), false)),
        );
        delegate
            .interaction
            .row_bounds
            .borrow_mut()
            .insert(0, Bounds::default());

        delegate.set_row_projection([3, 9].into_iter().collect());

        assert!(delegate.interaction.row_bounds.borrow().contains_key(&0));
        let reused = delegate
            .source
            .visible_lines
            .line(3)
            .expect("the prepared row should remain cached");
        assert_eq!(reused.source().as_ref(), "line 3");
    }

    #[test]
    fn projection_updates_release_decoded_rows_that_are_no_longer_reachable() {
        let document = Arc::new(LogDocument::placeholder("pruned-result-window.log"));
        let mut delegate = LogTableDelegate::projected(7, document, [3, 9].into_iter().collect());
        delegate.source.visible_lines.prepare_visible_rows(
            0..2,
            2,
            |row_ix| [3, 9].get(row_ix).copied(),
            |source_row, _| Some(LinePreview::new(format!("line {source_row}"), false)),
        );

        delegate.set_row_projection([9].into_iter().collect());
        assert_eq!(delegate.source.visible_lines.cached_keys(), [9]);

        delegate.set_row_projection(CompressedRows::default());
        assert!(delegate.source.visible_lines.cached_keys().is_empty());
    }

    #[test]
    fn document_replacement_advances_content_revision() {
        let document = Arc::new(LogDocument::placeholder("first.log"));
        let mut delegate = LogTableDelegate::all(1, document.clone());
        let initial_revision = delegate.content_revision();

        delegate.replace_with_all(document);
        assert_eq!(delegate.content_revision(), initial_revision);

        delegate.replace_with_all(Arc::new(LogDocument::placeholder("second.log")));
        assert!(delegate.content_revision() > initial_revision);
    }

    #[test]
    fn document_replacement_drops_source_bound_geometry_and_pointer_gesture() {
        let mut delegate = LogTableDelegate::projected(
            1,
            Arc::new(LogDocument::placeholder("first.log")),
            [3, 9].into_iter().collect(),
        );
        delegate.begin_pointer_selection(0, false, false, 1);
        delegate
            .interaction
            .row_bounds
            .borrow_mut()
            .insert(0, Bounds::default());
        assert!(delegate.is_pointer_selecting());

        delegate.replace_with_rows(
            Arc::new(LogDocument::placeholder("second.log")),
            [3, 9].into_iter().collect(),
        );

        assert!(!delegate.is_pointer_selecting());
        assert!(delegate.interaction.row_bounds.borrow().is_empty());
    }

    #[test]
    fn log_line_height_includes_configured_spacing() {
        assert_eq!(log_line_height(13, 14), px(27.));
        assert_eq!(log_line_height(u16::MAX, 1), px(u16::MAX as f32));
    }

    #[test]
    fn line_numbers_track_log_text_within_the_reference_clamp() {
        assert_eq!(line_number_font_size(13), px(11.));
        assert_eq!(line_number_font_size(8), px(8.));
        assert_eq!(line_number_font_size(40), px(18.));
    }

    #[test]
    fn exact_row_position_replaces_a_deferred_table_scroll() {
        let handle = gpui::UniformListScrollHandle::new();
        handle.scroll_to_item(80, gpui::ScrollStrategy::Center);

        scroll_uniform_log_row_to_viewport_y(&handle, 40, px(7.), px(20.));

        assert!(handle.0.borrow().deferred_scroll_to_item.is_none());
        assert_eq!(
            px(20.) * 40. + handle.0.borrow().base_handle.offset().y,
            px(7.)
        );
    }

    #[test]
    fn refreshing_result_rows_preserves_drag_cleanup_state() {
        let document = Arc::new(LogDocument::placeholder("selection-refresh.log"));
        let mut delegate = LogTableDelegate::projected(1, document, Default::default());
        delegate.begin_pointer_selection(0, false, false, 1);
        delegate.set_text_selection_suppressed(true);

        delegate.set_row_projection(Default::default());

        assert!(!delegate.is_pointer_selecting());
        assert!(delegate.is_text_selection_suppressed());

        delegate.end_pointer_selection();
        assert!(!delegate.is_text_selection_suppressed());
    }

    #[test]
    fn ending_or_restarting_pointer_selection_restores_text_selection() {
        let document = Arc::new(LogDocument::placeholder("selection-recovery.log"));
        let delegate = LogTableDelegate::projected(1, document, Default::default());
        delegate.set_text_selection_suppressed(true);

        delegate.end_pointer_selection();
        assert!(!delegate.is_text_selection_suppressed());

        delegate.set_text_selection_suppressed(true);
        delegate.begin_pointer_selection(0, false, false, 2);
        assert!(!delegate.is_text_selection_suppressed());
    }
}
