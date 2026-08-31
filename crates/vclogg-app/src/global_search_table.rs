use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet},
    ops::Range,
    path::PathBuf,
    rc::Rc,
    sync::Arc,
};

use gpui::{
    AnyElement, App, Bounds, Context, Div, Element, GlobalElementId, Hsla, InspectorElementId,
    InteractiveElement as _, IntoElement, LayoutId, MouseButton, MouseDownEvent,
    ParentElement as _, Pixels, Point, RenderOnce, SharedString, Stateful, Styled as _, StyledText,
    Window, div, prelude::FluentBuilder as _, px, svg,
};
use gpui_base::{GlobalState, TextSelection};
use gpui_component::{
    ActiveTheme as _, ElementExt as _, Icon, IconName, StyledExt as _, h_flex,
    table::{Column, TableDelegate, TableState},
    theme::try_parse_color,
    v_flex,
};
use vclogg_core::{CompressedRows, LogDocument, SearchMatcher};

use crate::color_labels::ResolvedColorRule;
use crate::log_table::{
    LogTableCursor, LogTableStateExt, RowSelection, combined_match_ranges, line_marker,
    line_marker_column_width, log_cell_horizontal_padding, log_line_height, log_line_number_cell,
    log_row_selection_color, log_row_selection_overlay, message_column_width,
    severity_accent_overlay, severity_style,
};
use crate::selectable_log_text::{LogText, SelectableLogText, TextSelectionCache};
use crate::state_store::{AppSettings, DEFAULT_WORD_BOUNDARY_CHARACTERS, LogFontFamily};
use crate::ui_theme;

#[derive(Clone)]
pub struct GlobalSearchGroup {
    pub document_id: u64,
    pub title: SharedString,
    pub path: PathBuf,
    pub document: Arc<LogDocument>,
    pub rows: CompressedRows,
    pub matched_rows: CompressedRows,
    pub marked_rows: Arc<BTreeSet<usize>>,
    pub truncated: bool,
    pub failure: Option<SharedString>,
    pub collapsed: bool,
    pub color_rules: Arc<[ResolvedColorRule]>,
}

#[derive(Clone)]
pub struct GlobalQuickFindGroup {
    pub view_start: usize,
    pub document: Arc<LogDocument>,
    pub rows: CompressedRows,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GlobalSearchRow {
    Group { document_id: u64 },
    Match { document_id: u64, source_row: usize },
}

#[derive(Clone, Copy)]
enum FlatRow {
    Group { group_ix: usize },
    Match { group_ix: usize, source_row: usize },
}

pub struct GlobalSearchTableDelegate {
    groups: Vec<GlobalSearchGroup>,
    group_starts: Vec<usize>,
    rows_len: usize,
    matcher: Option<SearchMatcher>,
    quick_find_matcher: Option<SearchMatcher>,
    log_font_family: LogFontFamily,
    log_font_size: u16,
    log_line_spacing: u16,
    line_number_width: u16,
    line_number_text_color: Option<Hsla>,
    line_number_background_color: Option<Hsla>,
    show_line_number_row_separators: bool,
    show_row_separators: bool,
    highlight_log_levels: bool,
    word_boundary_characters: SharedString,
    text_selections: TextSelectionCache<(u64, usize)>,
    suppress_text_selection: bool,
    row_selection: Rc<RefCell<RowSelection>>,
    active_row: Cell<Option<usize>>,
    suppress_table_clear: Cell<bool>,
    overscan: usize,
    row_cache: RefCell<BTreeMap<(u64, usize), CachedGlobalRowPresentation>>,
    row_cache_window: Cell<Option<(usize, usize)>>,
    row_bounds: Rc<RefCell<BTreeMap<usize, Bounds<Pixels>>>>,
}

#[derive(Clone)]
struct CachedGlobalRowPresentation {
    text: LogText,
    highlights: Arc<[(Range<usize>, crate::log_table::TextHighlight)]>,
}

pub(crate) enum WrappedGlobalRow {
    Group {
        document_id: u64,
        title: SharedString,
        path: PathBuf,
        result_count: usize,
        truncated: bool,
        failure: Option<SharedString>,
        collapsed: bool,
    },
    Match {
        document_id: u64,
        source_row: usize,
        text: LogText,
        selected: bool,
        marked: bool,
        matched: bool,
        highlight_severity: bool,
        highlights: Arc<[(Range<usize>, crate::log_table::TextHighlight)]>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GlobalSearchGroupIcon {
    Collapsed,
    Expanded,
    NoResults,
}

#[derive(Debug, Eq, PartialEq)]
struct GlobalSearchGroupHeaderPresentation {
    count_label: String,
    state_label: Option<String>,
    state_failed: bool,
    icon: GlobalSearchGroupIcon,
}

/// Defers a spanning table row until after DataTable's fixed-column chrome while preserving
/// the table viewport's content mask. The stock deferred element intentionally escapes clipping
/// for popovers, which would let a partially visible virtual row paint outside the result region.
struct DeferredTableRow {
    child: Option<AnyElement>,
}

impl DeferredTableRow {
    fn new(child: impl IntoElement) -> Self {
        Self {
            child: Some(child.into_any_element()),
        }
    }
}

impl Element for DeferredTableRow {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let _performance_scope = crate::ui_performance::scope("DeferredTableRow::request_layout");
        (self.child.as_mut().unwrap().request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        _: &mut App,
    ) -> Self::PrepaintState {
        let _performance_scope = crate::ui_performance::scope("DeferredTableRow::prepaint");
        let child = self.child.take().unwrap();
        let element_offset = window.element_offset();
        let content_mask = window.content_mask();
        window.defer_draw(child, element_offset, 0, Some(content_mask));
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        _: &mut Window,
        _: &mut App,
    ) {
        let _performance_scope = crate::ui_performance::scope("DeferredTableRow::paint");
    }
}

impl IntoElement for DeferredTableRow {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

fn format_group_result_count(result_count: usize) -> String {
    let digits = result_count.to_string();
    let mut formatted = String::with_capacity(digits.len() + digits.len().saturating_sub(1) / 3);
    for (ix, digit) in digits.chars().enumerate() {
        if ix > 0 && (digits.len() - ix).is_multiple_of(3) {
            formatted.push(',');
        }
        formatted.push(digit);
    }
    formatted
}

fn global_search_group_title(title: &str, path: &std::path::Path) -> String {
    let displayed_path = path.to_string_lossy();
    let file_name = displayed_path.rsplit(['/', '\\']).next();
    if file_name == Some(title) {
        displayed_path.into_owned()
    } else {
        format!("{title} — {displayed_path}")
    }
}

fn global_search_group_header_presentation(
    result_count: usize,
    truncated: bool,
    failure: Option<&str>,
    collapsed: bool,
) -> GlobalSearchGroupHeaderPresentation {
    GlobalSearchGroupHeaderPresentation {
        count_label: crate::tr_args!(
            "{} 个结果",
            "{} results",
            format_group_result_count(result_count)
        ),
        state_label: failure
            .map(|failure| crate::tr_args!("搜索失败 · {failure}", "Search failed · {failure}"))
            .or_else(|| truncated.then(|| crate::tr!("已截断", "Truncated").to_string())),
        state_failed: failure.is_some(),
        icon: if result_count == 0 {
            GlobalSearchGroupIcon::NoResults
        } else if collapsed {
            GlobalSearchGroupIcon::Collapsed
        } else {
            GlobalSearchGroupIcon::Expanded
        },
    }
}

#[derive(IntoElement)]
pub(crate) struct GlobalSearchGroupHeader {
    title: SharedString,
    path: PathBuf,
    result_count: usize,
    truncated: bool,
    failure: Option<SharedString>,
    collapsed: bool,
    font_family: SharedString,
    font_size: u16,
}

impl GlobalSearchGroupHeader {
    pub(crate) fn new(
        title: SharedString,
        path: PathBuf,
        result_count: usize,
        font_family: SharedString,
        font_size: u16,
    ) -> Self {
        Self {
            title,
            path,
            result_count,
            truncated: false,
            failure: None,
            collapsed: false,
            font_family,
            font_size,
        }
    }

    pub(crate) fn truncated(mut self, truncated: bool) -> Self {
        self.truncated = truncated;
        self
    }

    pub(crate) fn failure(mut self, failure: Option<SharedString>) -> Self {
        self.failure = failure;
        self
    }

    pub(crate) fn collapsed(mut self, collapsed: bool) -> Self {
        self.collapsed = collapsed;
        self
    }
}

impl RenderOnce for GlobalSearchGroupHeader {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let _performance_scope = crate::ui_performance::scope("GlobalSearchGroupHeader::render");
        let presentation = global_search_group_header_presentation(
            self.result_count,
            self.truncated,
            self.failure.as_deref(),
            self.collapsed,
        );
        let icon_color = if presentation.icon == GlobalSearchGroupIcon::NoResults {
            cx.theme().muted_foreground
        } else {
            cx.theme().primary
        };
        let icon = match presentation.icon {
            GlobalSearchGroupIcon::Collapsed => Icon::new(IconName::ChevronRight)
                .size_4()
                .text_color(icon_color)
                .into_any_element(),
            GlobalSearchGroupIcon::Expanded => Icon::new(IconName::ChevronDown)
                .size_4()
                .text_color(icon_color)
                .into_any_element(),
            GlobalSearchGroupIcon::NoResults => svg()
                .data(include_bytes!(
                    "../assets/icons/document-dismiss-16-regular.svg"
                ))
                .size_4()
                .text_color(icon_color)
                .into_any_element(),
        };

        h_flex()
            .size_full()
            .min_w_0()
            .bg(cx.theme().muted.opacity(0.45))
            .border_t_1()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .h_full()
                    .w_6()
                    .flex_none()
                    .justify_center()
                    .child(icon),
            )
            .child(
                h_flex()
                    .min_w_0()
                    .flex_1()
                    .gap_2()
                    .pr_3()
                    .child(
                        div()
                            .min_w_0()
                            .flex_initial()
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .font_family(self.font_family)
                            .text_size(px(self.font_size.saturating_sub(1).max(8) as f32))
                            .font_semibold()
                            .child(global_search_group_title(&self.title, &self.path)),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(presentation.count_label),
                    )
                    .when_some(presentation.state_label, |header, state_label| {
                        header.child(
                            div()
                                .min_w_0()
                                .overflow_hidden()
                                .text_ellipsis()
                                .whitespace_nowrap()
                                .text_xs()
                                .text_color(if presentation.state_failed {
                                    cx.theme().danger
                                } else {
                                    cx.theme().muted_foreground
                                })
                                .child(state_label),
                        )
                    }),
            )
    }
}

impl GlobalSearchTableDelegate {
    pub fn new() -> Self {
        Self {
            groups: Vec::new(),
            group_starts: Vec::new(),
            rows_len: 0,
            matcher: None,
            quick_find_matcher: None,
            log_font_family: LogFontFamily::default(),
            log_font_size: 13,
            log_line_spacing: 6,
            line_number_width: 60,
            line_number_text_color: None,
            line_number_background_color: None,
            show_line_number_row_separators: false,
            show_row_separators: false,
            highlight_log_levels: false,
            word_boundary_characters: DEFAULT_WORD_BOUNDARY_CHARACTERS.into(),
            text_selections: TextSelectionCache::default(),
            suppress_text_selection: false,
            row_selection: Rc::default(),
            active_row: Cell::default(),
            suppress_table_clear: Cell::default(),
            overscan: 12,
            row_cache: RefCell::default(),
            row_cache_window: Cell::default(),
            row_bounds: Rc::default(),
        }
    }

    pub fn set_groups(&mut self, groups: Vec<GlobalSearchGroup>, matcher: Option<SearchMatcher>) {
        self.groups = groups;
        self.matcher = matcher;
        self.row_cache.get_mut().clear();
        self.row_cache_window.set(None);
        self.row_bounds.borrow_mut().clear();
        self.row_selection.borrow_mut().clear();
        self.active_row.set(None);
        self.rebuild_layout();
    }

    pub fn set_quick_find_matcher(&mut self, matcher: Option<SearchMatcher>) {
        self.quick_find_matcher = matcher;
        self.row_cache.get_mut().clear();
        self.row_cache_window.set(None);
    }

    pub fn set_appearance(&mut self, settings: &AppSettings) {
        self.log_font_family = settings.log_font_family;
        self.log_font_size = settings.log_font_size.clamp(8, 32);
        self.log_line_spacing = settings.log_line_spacing.clamp(1, 40);
        self.line_number_width = settings.line_number_width.clamp(40, 160);
        self.line_number_text_color = settings
            .line_number_text_color
            .as_deref()
            .and_then(|value| try_parse_color(value).ok());
        self.line_number_background_color = settings
            .line_number_background_color
            .as_deref()
            .and_then(|value| try_parse_color(value).ok());
        self.show_line_number_row_separators = settings.show_line_number_row_separators;
        self.show_row_separators = settings.default_show_row_separators;
        self.overscan = usize::from(settings.viewer_overscan.clamp(4, 40));
        self.row_cache_window.set(None);
    }

    pub fn set_word_boundary_characters(&mut self, characters: impl Into<SharedString>) {
        self.word_boundary_characters = characters.into();
    }

    pub fn set_highlight_log_levels(&mut self, enabled: bool) {
        self.highlight_log_levels = enabled;
    }

    pub(crate) fn resolved_font_family(&self, cx: &App) -> SharedString {
        match self.log_font_family {
            LogFontFamily::CascadiaMono => "Cascadia Mono".into(),
            LogFontFamily::JetBrainsMono => "JetBrains Mono".into(),
            LogFontFamily::Consolas => "Consolas".into(),
            LogFontFamily::SystemMonospace => cx.theme().mono_font_family.clone(),
        }
    }

    pub(crate) fn line_number_width(&self) -> u16 {
        self.line_number_width
    }

    pub(crate) fn line_number_text_color(&self, cx: &App) -> Hsla {
        self.line_number_text_color
            .unwrap_or(cx.theme().muted_foreground)
    }

    pub(crate) fn line_number_background_color(&self, cx: &App) -> Hsla {
        self.line_number_background_color
            .unwrap_or_else(|| cx.theme().muted.opacity(0.45))
    }

    pub(crate) fn show_line_number_row_separators(&self) -> bool {
        self.show_line_number_row_separators
    }

    pub(crate) fn show_row_separators(&self) -> bool {
        self.show_row_separators
    }

    pub fn row(&self, row_ix: usize) -> Option<GlobalSearchRow> {
        match self.flat_row(row_ix)? {
            FlatRow::Group { group_ix } => Some(GlobalSearchRow::Group {
                document_id: self.groups.get(group_ix)?.document_id,
            }),
            FlatRow::Match {
                group_ix,
                source_row,
            } => Some(GlobalSearchRow::Match {
                document_id: self.groups.get(group_ix)?.document_id,
                source_row,
            }),
        }
    }

    pub fn row_ix(&self, target: GlobalSearchRow) -> Option<usize> {
        self.groups
            .iter()
            .enumerate()
            .find_map(|(group_ix, group)| match target {
                GlobalSearchRow::Group { document_id } if document_id == group.document_id => {
                    self.group_starts.get(group_ix).copied()
                }
                GlobalSearchRow::Match {
                    document_id,
                    source_row,
                } if document_id == group.document_id && !group.collapsed => {
                    group.rows.position(source_row).and_then(|position| {
                        self.group_starts
                            .get(group_ix)
                            .copied()
                            .and_then(|start| start.checked_add(position.saturating_add(1)))
                    })
                }
                _ => None,
            })
    }

    pub fn collapsed_document_ids(&self) -> BTreeSet<u64> {
        self.groups
            .iter()
            .filter(|group| group.collapsed)
            .map(|group| group.document_id)
            .collect()
    }

    pub(crate) fn restore_collapsed_document_ids(&mut self, document_ids: &BTreeSet<u64>) {
        for group in &mut self.groups {
            group.collapsed = document_ids.contains(&group.document_id);
        }
        self.rebuild_layout();
    }

    pub fn toggle_group(&mut self, document_id: u64) {
        let Some(group) = self
            .groups
            .iter_mut()
            .find(|group| group.document_id == document_id)
        else {
            return;
        };
        if group.rows.is_empty() {
            return;
        }
        group.collapsed = !group.collapsed;
        self.rebuild_layout();
    }

    pub fn group_has_results(&self, document_id: u64) -> bool {
        self.groups
            .iter()
            .find(|group| group.document_id == document_id)
            .is_some_and(|group| !group.rows.is_empty())
    }

    pub fn groups_count(&self) -> usize {
        self.groups.len()
    }

    pub fn results_count(&self) -> usize {
        self.groups.iter().map(|group| group.rows.len()).sum()
    }

    pub fn has_truncated_results(&self) -> bool {
        self.groups.iter().any(|group| group.truncated)
    }

    pub fn rows_len(&self) -> usize {
        self.rows_len
    }

    pub fn quick_find_groups(&self) -> Vec<GlobalQuickFindGroup> {
        self.groups
            .iter()
            .enumerate()
            .filter(|(_, group)| !group.collapsed && !group.rows.is_empty())
            .filter_map(|(group_ix, group)| {
                self.group_starts
                    .get(group_ix)
                    .copied()
                    .and_then(|start| start.checked_add(1))
                    .map(|view_start| GlobalQuickFindGroup {
                        view_start,
                        document: group.document.clone(),
                        rows: group.rows.clone(),
                    })
            })
            .collect()
    }

    pub(crate) fn log_font_size(&self) -> u16 {
        self.log_font_size
    }

    pub(crate) fn wrapped_row(&self, row_ix: usize) -> Option<WrappedGlobalRow> {
        match self.flat_row(row_ix)? {
            FlatRow::Group { group_ix } => {
                let group = self.groups.get(group_ix)?;
                Some(WrappedGlobalRow::Group {
                    document_id: group.document_id,
                    title: group.title.clone(),
                    path: group.path.clone(),
                    result_count: group.rows.len(),
                    truncated: group.truncated,
                    failure: group.failure.clone(),
                    collapsed: group.collapsed,
                })
            }
            FlatRow::Match {
                group_ix,
                source_row,
            } => {
                let group = self.groups.get(group_ix)?;
                let presentation = self.cached_presentation(group_ix, source_row)?;
                Some(WrappedGlobalRow::Match {
                    document_id: group.document_id,
                    source_row,
                    selected: self.row_selection.borrow().contains(row_ix),
                    marked: group.marked_rows.contains(&source_row),
                    matched: group.matched_rows.contains(source_row),
                    highlights: presentation.highlights,
                    text: presentation.text,
                    highlight_severity: self.highlight_log_levels,
                })
            }
        }
    }

    pub(crate) fn prefetch_rows(&self, visible_range: Range<usize>) {
        let start = visible_range.start.saturating_sub(self.overscan);
        let end = visible_range
            .end
            .saturating_add(self.overscan)
            .min(self.rows_len);
        if self.row_cache_window.replace(Some((start, end))) == Some((start, end)) {
            return;
        }
        let desired_rows = (start..end)
            .filter_map(|row_ix| match self.flat_row(row_ix) {
                Some(FlatRow::Match {
                    group_ix,
                    source_row,
                }) => Some((group_ix, source_row)),
                _ => None,
            })
            .collect::<Vec<_>>();
        let desired_keys = desired_rows
            .iter()
            .filter_map(|(group_ix, source_row)| {
                self.groups
                    .get(*group_ix)
                    .map(|group| (group.document_id, *source_row))
            })
            .collect::<BTreeSet<_>>();
        self.row_cache
            .borrow_mut()
            .retain(|key, _| desired_keys.contains(key));
        for (group_ix, source_row) in desired_rows {
            let Some(group) = self.groups.get(group_ix) else {
                continue;
            };
            if !self
                .row_cache
                .borrow()
                .contains_key(&(group.document_id, source_row))
            {
                _ = self.cached_presentation(group_ix, source_row);
            }
        }
    }

    fn cached_line(&self, group_ix: usize, source_row: usize) -> Option<SharedString> {
        self.cached_presentation(group_ix, source_row)
            .map(|presentation| presentation.text.source().clone())
    }

    fn cached_presentation(
        &self,
        group_ix: usize,
        source_row: usize,
    ) -> Option<CachedGlobalRowPresentation> {
        let group = self.groups.get(group_ix)?;
        if let Some(presentation) = self
            .row_cache
            .borrow()
            .get(&(group.document_id, source_row))
            .cloned()
        {
            return Some(presentation);
        }
        let line: SharedString = group.document.line(source_row)?.into();
        let source_highlights = combined_match_ranges(
            &line,
            &group.color_rules,
            self.matcher.as_ref(),
            self.quick_find_matcher.as_ref(),
        );
        let text = LogText::new(line);
        let presentation = CachedGlobalRowPresentation {
            highlights: source_highlights
                .into_iter()
                .filter_map(|(range, highlight)| {
                    text.display_range(range).map(|range| (range, highlight))
                })
                .collect(),
            text,
        };
        self.row_cache
            .borrow_mut()
            .insert((group.document_id, source_row), presentation.clone());
        Some(presentation)
    }

    pub(crate) fn begin_pointer_selection(
        &self,
        row_ix: usize,
        control: bool,
        shift: bool,
        click_count: usize,
    ) {
        self.row_selection.borrow_mut().begin_pointer_selection(
            row_ix,
            control,
            shift,
            click_count,
        );
    }

    pub(crate) fn prepare_context_selection(&self, row_ix: usize) {
        self.row_selection
            .borrow_mut()
            .prepare_context_selection(row_ix);
    }

    pub(crate) fn extend_pointer_selection(&self, row_ix: usize) {
        self.row_selection
            .borrow_mut()
            .extend_pointer_selection(row_ix);
    }

    pub(crate) fn restore_pointer_selection(&self) {
        self.row_selection.borrow_mut().restore_pointer_selection();
    }

    pub(crate) fn end_pointer_selection(&self) {
        self.row_selection.borrow_mut().end_pointer_selection();
    }

    pub(crate) fn is_pointer_selecting(&self) -> bool {
        self.row_selection.borrow().is_pointer_selecting()
    }

    pub(crate) fn pointer_drag_anchor(&self) -> Option<usize> {
        self.row_selection.borrow().pointer_drag_anchor()
    }

    pub(crate) fn pointer_text_selection_allowed(&self) -> bool {
        self.row_selection.borrow().is_text_selection_allowed()
    }

    pub(crate) fn row_at_position(&self, position: Point<Pixels>) -> Option<usize> {
        self.row_bounds
            .borrow()
            .iter()
            .find_map(|(row_ix, bounds)| bounds.contains(&position).then_some(*row_ix))
    }

    pub(crate) fn visible_row_edge(&self, after: bool) -> Option<usize> {
        let bounds = self.row_bounds.borrow();
        if after {
            bounds.keys().next_back().copied()
        } else {
            bounds.keys().next().copied()
        }
    }

    pub(crate) fn set_text_selection_suppressed(&mut self, suppressed: bool) {
        self.suppress_text_selection = suppressed;
    }

    pub(crate) fn nearest_match_row(&self, row_ix: usize, prefer_after: bool) -> Option<usize> {
        if matches!(self.flat_row(row_ix), Some(FlatRow::Match { .. })) {
            return Some(row_ix);
        }
        let before = (0..row_ix)
            .rev()
            .find(|candidate| matches!(self.flat_row(*candidate), Some(FlatRow::Match { .. })));
        let after = (row_ix.saturating_add(1)..self.rows_len)
            .find(|candidate| matches!(self.flat_row(*candidate), Some(FlatRow::Match { .. })));
        if prefer_after {
            after.or(before)
        } else {
            before.or(after)
        }
    }

    pub(crate) fn extend_keyboard_selection(&self, row_ix: usize) {
        self.row_selection
            .borrow_mut()
            .extend_keyboard_selection(row_ix);
    }

    pub(crate) fn settle_table_selection(&self, row_ix: usize) {
        self.row_selection
            .borrow_mut()
            .settle_table_selection(row_ix);
    }

    pub(crate) fn clear_row_selection(&self) {
        self.row_selection.borrow_mut().clear();
    }

    pub(crate) fn select_all_rows(&self) {
        self.row_selection.borrow_mut().select_all(self.rows_len);
    }

    pub(crate) fn selected_rows_count(&self) -> usize {
        let selection = self.row_selection.borrow();
        selection
            .selected_ranges()
            .map(|(start, end)| {
                let end = end.min(self.rows_len.saturating_sub(1));
                let row_count = end.saturating_sub(start).saturating_add(1);
                let group_count = self.group_starts.partition_point(|row| *row <= end)
                    - self.group_starts.partition_point(|row| *row < start);
                row_count.saturating_sub(group_count)
            })
            .sum()
    }

    pub(crate) fn is_row_selected(&self, row_ix: usize) -> bool {
        matches!(self.flat_row(row_ix), Some(FlatRow::Match { .. }))
            && self.row_selection.borrow().contains(row_ix)
    }

    pub(crate) fn selected_matches(&self) -> Vec<(u64, usize)> {
        let selection = self.row_selection.borrow();
        selection
            .selected_indices(self.rows_len)
            .filter_map(|row_ix| match self.flat_row(row_ix)? {
                FlatRow::Group { .. } => None,
                FlatRow::Match {
                    group_ix,
                    source_row,
                } => Some((self.groups[group_ix].document_id, source_row)),
            })
            .collect()
    }

    pub(crate) fn selection_snapshot(&self) -> BTreeMap<u64, CompressedRows> {
        let mut rows_by_document = BTreeMap::<u64, Vec<usize>>::new();
        let selection = self.row_selection.borrow();
        for row_ix in selection.selected_indices(self.rows_len) {
            let Some(FlatRow::Match {
                group_ix,
                source_row,
            }) = self.flat_row(row_ix)
            else {
                continue;
            };
            let document_id = self.groups[group_ix].document_id;
            rows_by_document
                .entry(document_id)
                .or_default()
                .push(source_row);
        }
        rows_by_document
            .into_iter()
            .map(|(document_id, rows)| (document_id, rows.into_iter().collect()))
            .collect()
    }

    pub(crate) fn restore_selection(&self, snapshot: &BTreeMap<u64, CompressedRows>) {
        let selected_indices = (0..self.rows_len).filter(|row_ix| {
            let Some(FlatRow::Match {
                group_ix,
                source_row,
            }) = self.flat_row(*row_ix)
            else {
                return false;
            };
            snapshot
                .get(&self.groups[group_ix].document_id)
                .is_some_and(|rows| rows.contains(source_row))
        });
        self.row_selection
            .borrow_mut()
            .replace_indices(selected_indices);
    }

    fn rebuild_layout(&mut self) {
        self.group_starts.clear();
        self.rows_len = 0;
        for group in &self.groups {
            self.group_starts.push(self.rows_len);
            self.rows_len = self.rows_len.saturating_add(1);
            if !group.collapsed {
                self.rows_len = self.rows_len.saturating_add(group.rows.len());
            }
        }
    }

    fn flat_row(&self, row_ix: usize) -> Option<FlatRow> {
        if row_ix >= self.rows_len {
            return None;
        }
        let group_ix = self
            .group_starts
            .partition_point(|start| *start <= row_ix)
            .saturating_sub(1);
        let group_start = *self.group_starts.get(group_ix)?;
        if row_ix == group_start {
            return Some(FlatRow::Group { group_ix });
        }
        let group = self.groups.get(group_ix)?;
        if group.collapsed {
            return None;
        }
        let source_row = group.rows.get(row_ix.saturating_sub(group_start + 1))?;
        Some(FlatRow::Match {
            group_ix,
            source_row,
        })
    }
}

impl LogTableCursor for GlobalSearchTableDelegate {
    fn active_log_row(&self) -> Option<usize> {
        self.active_row.get()
    }

    fn set_active_log_row(&self, row_ix: Option<usize>) {
        self.active_row.set(row_ix);
    }

    fn suppress_next_table_clear(&self) {
        self.suppress_table_clear.set(true);
    }

    fn take_suppressed_table_clear(&self) -> bool {
        self.suppress_table_clear.replace(false)
    }
}

impl TableDelegate for GlobalSearchTableDelegate {
    fn columns_count(&self, _: &App) -> usize {
        3
    }

    fn rows_count(&self, _: &App) -> usize {
        self.rows_len
    }

    fn column(&self, col_ix: usize, cx: &App) -> Column {
        let base = px(self.log_font_size as f32);
        match col_ix {
            0 => {
                let width = line_marker_column_width();
                Column::new("global-marker", crate::tr!("标记", "Mark"))
                    .p_0()
                    .width(width)
                    .min_width(width)
                    .max_width(width)
                    .resizable(false)
                    .fixed_left()
                    .movable(false)
            }
            1 => {
                let width = px(self.line_number_width as f32);
                Column::new("global-line-number", crate::tr!("行", "Line"))
                    .p_0()
                    .width(width)
                    .min_width(width)
                    .max_width(width)
                    .resizable(false)
                    .fixed_left()
                    .movable(false)
                    .text_right()
            }
            2 => Column::new("global-message", crate::tr!("文件与日志", "File & log"))
                .p_0()
                .width(message_column_width(
                    self.groups
                        .iter()
                        .map(|group| group.document.metadata().longest_line_columns)
                        .max()
                        .unwrap_or_default(),
                    self.resolved_font_family(cx),
                    self.log_font_size,
                    cx,
                ))
                .min_width(base * 24.)
                .movable(false),
            _ => unreachable!("global results expose marker, line-number, and message columns"),
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
        let _performance_scope =
            crate::ui_performance::scope("GlobalSearchTableDelegate::render_tr");
        let row_bounds = self.row_bounds.clone();
        match self.flat_row(row_ix) {
            Some(FlatRow::Group { group_ix }) => {
                let group = &self.groups[group_ix];
                let document_id = group.document_id;
                let header = GlobalSearchGroupHeader::new(
                    group.title.clone(),
                    group.path.clone(),
                    group.rows.len(),
                    self.resolved_font_family(cx),
                    self.log_font_size,
                )
                .truncated(group.truncated)
                .failure(group.failure.clone())
                .collapsed(group.collapsed);
                div()
                    .id(("global-search-group", document_id))
                    .relative()
                    .on_prepaint(move |bounds, _, _| {
                        row_bounds.borrow_mut().insert(row_ix, bounds);
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |table, _: &MouseDownEvent, _, cx| {
                            table.delegate().clear_row_selection();
                            table.set_active_log_row(row_ix, cx);
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|table, _: &MouseDownEvent, _, cx| {
                            table.delegate().clear_row_selection();
                            table.clear_selection(cx);
                        }),
                    )
                    // DataTable appends its fixed-column divider after the delegate row.
                    // Paint the spanning file header last so result-only column chrome
                    // cannot bleed through it.
                    .child(DeferredTableRow::new(
                        div().absolute().inset_0().child(header),
                    ))
            }
            Some(FlatRow::Match {
                group_ix,
                source_row,
            }) => {
                let group = &self.groups[group_ix];
                let severity = self
                    .highlight_log_levels
                    .then(|| self.cached_line(group_ix, source_row))
                    .flatten()
                    .and_then(|line| severity_style(&line, cx));
                div()
                    .id(format!(
                        "global-search-result-{}-{source_row}",
                        group.document_id
                    ))
                    .border_0()
                    .on_prepaint(move |bounds, _, _| {
                        row_bounds.borrow_mut().insert(row_ix, bounds);
                    })
                    .on_mouse_down(MouseButton::Left, |event: &MouseDownEvent, window, cx| {
                        if event.click_count >= 3 {
                            GlobalState::suppress_text_selection(cx);
                            TextSelection::clear(window, cx);
                        }
                    })
                    .when_some(severity, |row, style| row.bg(style.background))
                    .on_mouse_down(
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
            }
            None => div().id(("global-search-empty-row", row_ix)),
        }
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let _performance_scope =
            crate::ui_performance::scope("GlobalSearchTableDelegate::render_td");
        let Some(row) = self.flat_row(row_ix) else {
            return div().into_any_element();
        };
        let line_height = log_line_height(self.log_font_size, self.log_line_spacing);
        match row {
            FlatRow::Group { .. } => div().into_any_element(),
            FlatRow::Match {
                group_ix,
                source_row,
            } => {
                let group = &self.groups[group_ix];
                let selected = self.row_selection.borrow().contains(row_ix);
                if col_ix == 0 {
                    let marked = group.marked_rows.contains(&source_row);
                    let matched = group.matched_rows.contains(source_row);
                    let severity_accent = self
                        .highlight_log_levels
                        .then(|| self.cached_line(group_ix, source_row))
                        .flatten()
                        .and_then(|line| severity_style(&line, cx))
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
                } else if col_ix == 1 {
                    log_line_number_cell(
                        source_row,
                        self.log_font_size,
                        line_height,
                        self.line_number_text_color(cx),
                        self.line_number_background_color(cx),
                        self.show_line_number_row_separators,
                        cx,
                    )
                    .size_full()
                    .into_any_element()
                } else if col_ix == 2 {
                    let presentation = self
                        .cached_presentation(group_ix, source_row)
                        .unwrap_or_else(|| CachedGlobalRowPresentation {
                            text: LogText::default(),
                            highlights: Arc::default(),
                        });
                    let text = presentation.text;
                    let highlights = presentation
                        .highlights
                        .iter()
                        .cloned()
                        .map(|(range, highlight)| {
                            (range, crate::log_table::text_highlight_style(highlight, cx))
                        })
                        .collect::<Vec<_>>();
                    let styled_text =
                        StyledText::new(text.display().clone()).with_highlights(highlights);
                    let selection = self.text_selections.handle(
                        (group.document_id, source_row),
                        &text,
                        window,
                        cx,
                    );
                    h_flex()
                        .relative()
                        .size_full()
                        .overflow_hidden()
                        .px(log_cell_horizontal_padding(cx))
                        .when(selected, |cell| {
                            cell.bg(log_row_selection_color(cx))
                                .child(log_row_selection_overlay(
                                    row_ix == 0 || !self.is_row_selected(row_ix - 1),
                                    row_ix + 1 >= self.rows_len
                                        || !self.is_row_selected(row_ix + 1),
                                    cx,
                                ))
                        })
                        .text_size(px(self.log_font_size as f32))
                        .line_height(line_height)
                        .font_family(self.resolved_font_family(cx))
                        .when(self.show_row_separators && !selected, |cell| {
                            cell.border_b_1().border_color(cx.theme().border)
                        })
                        .child(
                            SelectableLogText::new(
                                selection,
                                group.document_id.rotate_left(32) ^ source_row as u64,
                                text,
                                styled_text,
                                ui_theme::text_selection_highlight(cx),
                            )
                            .word_boundary_characters(self.word_boundary_characters.clone())
                            .suppress_selection(self.suppress_text_selection),
                        )
                        .into_any_element()
                } else {
                    div().into_any_element()
                }
            }
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
            .child(crate::tr!("尚未执行全局搜索", "Global search has not run"))
    }

    fn visible_rows_changed(
        &mut self,
        visible_range: Range<usize>,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) {
        self.row_bounds
            .borrow_mut()
            .retain(|row_ix, _| visible_range.contains(row_ix));
        self.prefetch_rows(visible_range);
    }

    fn cell_text(&self, row_ix: usize, col_ix: usize, _: &App) -> String {
        let Some(row) = self.flat_row(row_ix) else {
            return String::new();
        };
        match row {
            FlatRow::Group { group_ix } if col_ix == 2 => {
                self.groups[group_ix].path.display().to_string()
            }
            FlatRow::Match { source_row, .. } if col_ix == 1 => (source_row + 1).to_string(),
            FlatRow::Match {
                group_ix,
                source_row,
            } if col_ix == 2 => self
                .cached_line(group_ix, source_row)
                .unwrap_or_default()
                .to_string(),
            _ => String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        GlobalSearchGroupIcon, format_group_result_count, global_search_group_header_presentation,
        global_search_group_title,
    };

    #[test]
    fn group_header_uses_document_dismiss_for_zero_results() {
        let presentation = global_search_group_header_presentation(0, false, None, false);

        assert_eq!(presentation.count_label, "0 个结果");
        assert_eq!(presentation.icon, GlobalSearchGroupIcon::NoResults);
    }

    #[test]
    fn group_header_preserves_disclosure_and_state_metadata() {
        let collapsed = global_search_group_header_presentation(1_274, true, None, true);
        let expanded = global_search_group_header_presentation(635, false, None, false);
        let failed = global_search_group_header_presentation(0, false, Some("无法读取"), false);

        assert_eq!(format_group_result_count(1_274), "1,274");
        assert_eq!(collapsed.count_label, "1,274 个结果");
        assert_eq!(collapsed.state_label.as_deref(), Some("已截断"));
        assert_eq!(collapsed.icon, GlobalSearchGroupIcon::Collapsed);
        assert_eq!(expanded.icon, GlobalSearchGroupIcon::Expanded);
        assert_eq!(failed.state_label.as_deref(), Some("搜索失败 · 无法读取"));
        assert!(failed.state_failed);
    }

    #[test]
    fn group_header_uses_the_path_without_repeating_the_default_file_name() {
        let path = std::path::Path::new(r"F:\logs\camera.txt");

        assert_eq!(
            global_search_group_title("camera.txt", path),
            r"F:\logs\camera.txt"
        );
        assert_eq!(
            global_search_group_title("Camera startup", path),
            r"Camera startup — F:\logs\camera.txt"
        );
    }
}
