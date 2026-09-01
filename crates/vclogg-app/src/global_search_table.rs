use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet},
    ops::Range,
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
};

use gpui::{
    AnyElement, App, Bounds, Context, Div, Element, GlobalElementId, Hsla, InspectorElementId,
    InteractiveElement as _, IntoElement, LayoutId, MouseButton, MouseDownEvent,
    ParentElement as _, Pixels, RenderOnce, SharedString, Stateful, Styled as _, StyledText,
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
    LogTableCursor, LogTableRows, LogTableStateExt, RowSelection, combined_match_ranges,
    line_marker, line_marker_column_width, log_cell_horizontal_padding, log_line_height,
    log_line_number_cell, log_row_selection_color, log_row_selection_overlay, message_column_width,
    severity_accent_overlay, severity_style,
};
use crate::selectable_log_text::{LogText, SelectableLogText, TextSelectionCache};
use crate::state_store::{AppSettings, DEFAULT_WORD_BOUNDARY_CHARACTERS, LogFontFamily};
use crate::ui_theme;
use crate::virtual_log_lines::{LogRowKey, VisibleLineStore};

#[derive(Clone)]
pub struct GlobalSearchGroup {
    pub source: GlobalSearchGroupSource,
    pub projection: GlobalSearchGroupProjection,
    pub presentation: GlobalSearchGroupPresentation,
}

#[derive(Clone)]
pub struct GlobalSearchGroupSource {
    pub document_id: u64,
    pub title: SharedString,
    pub path: PathBuf,
    pub document: Arc<LogDocument>,
}

#[derive(Clone)]
pub struct GlobalSearchGroupProjection {
    pub rows: CompressedRows,
}

#[derive(Clone)]
pub struct GlobalSearchGroupPresentation {
    pub matched_rows: CompressedRows,
    pub marked_rows: CompressedRows,
    pub truncated: bool,
    pub failure: Option<SharedString>,
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
    projection: GlobalSearchProjectionState,
    presenter: GlobalRowPresenter,
    visible_lines: VisibleLineStore<(u64, usize)>,
    interaction: GlobalInteractionState,
}

struct GlobalSearchProjectionState {
    groups: Vec<GlobalSearchGroup>,
    group_by_document: BTreeMap<u64, usize>,
    content_revision: u64,
    layout_revision: u64,
    group_starts: Vec<usize>,
    rows_len: usize,
    max_line_columns: usize,
}

struct GlobalRowPresenter {
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
}

struct GlobalInteractionState {
    word_boundary_characters: SharedString,
    text_selections: TextSelectionCache<(u64, usize)>,
    suppress_text_selection: Cell<bool>,
    row_selection: Rc<RefCell<RowSelection>>,
    active_row: Cell<Option<usize>>,
    suppress_table_clear: Cell<bool>,
    row_bounds: Rc<RefCell<BTreeMap<usize, Bounds<Pixels>>>>,
    collapsed_documents: BTreeSet<u64>,
}

impl Default for GlobalSearchProjectionState {
    fn default() -> Self {
        Self {
            groups: Vec::new(),
            group_by_document: BTreeMap::new(),
            content_revision: 1,
            layout_revision: 1,
            group_starts: Vec::new(),
            rows_len: 0,
            max_line_columns: 0,
        }
    }
}

impl Default for GlobalRowPresenter {
    fn default() -> Self {
        Self {
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
        }
    }
}

impl GlobalRowPresenter {
    fn present(&self, text: LogText, color_rules: &[ResolvedColorRule]) -> GlobalRowPresentation {
        let source_highlights = combined_match_ranges(
            text.source(),
            color_rules,
            self.matcher.as_ref(),
            self.quick_find_matcher.as_ref(),
        );
        GlobalRowPresentation {
            highlights: source_highlights
                .into_iter()
                .filter_map(|(range, highlight)| {
                    text.display_range(range).map(|range| (range, highlight))
                })
                .collect(),
            text,
        }
    }
}

impl Default for GlobalInteractionState {
    fn default() -> Self {
        Self {
            word_boundary_characters: DEFAULT_WORD_BOUNDARY_CHARACTERS.into(),
            text_selections: TextSelectionCache::default(),
            suppress_text_selection: Cell::default(),
            row_selection: Rc::default(),
            active_row: Cell::default(),
            suppress_table_clear: Cell::default(),
            row_bounds: Rc::default(),
            collapsed_documents: BTreeSet::new(),
        }
    }
}

#[derive(Clone)]
struct GlobalRowPresentation {
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
            projection: GlobalSearchProjectionState::default(),
            presenter: GlobalRowPresenter::default(),
            visible_lines: VisibleLineStore::default(),
            interaction: GlobalInteractionState::default(),
        }
    }

    pub(crate) fn has_same_virtual_content(&self, groups: &[GlobalSearchGroup]) -> bool {
        groups.len() == self.projection.groups.len()
            && groups
                .iter()
                .zip(&self.projection.groups)
                .all(|(next, current)| {
                    next.source.document_id == current.source.document_id
                        && Arc::ptr_eq(&next.source.document, &current.source.document)
                        && next.projection.rows == current.projection.rows
                })
    }

    pub fn set_groups(&mut self, groups: Vec<GlobalSearchGroup>) {
        self.interaction
            .row_selection
            .borrow_mut()
            .end_pointer_selection();
        let projection_changed = groups.len() != self.projection.groups.len()
            || groups
                .iter()
                .zip(&self.projection.groups)
                .any(|(next, current)| {
                    next.source.document_id != current.source.document_id
                        || next.projection.rows != current.projection.rows
                });
        let stable_interaction = projection_changed.then(|| self.stable_interaction_rows());
        let documents_changed = groups.len() != self.projection.groups.len()
            || groups.iter().any(|next| {
                self.projection
                    .group_by_document
                    .get(&next.source.document_id)
                    .and_then(|group_ix| self.projection.groups.get(*group_ix))
                    .is_none_or(|current| {
                        !Arc::ptr_eq(&current.source.document, &next.source.document)
                    })
            });
        if documents_changed {
            let reusable_documents = groups
                .iter()
                .filter(|next| {
                    self.projection
                        .group_by_document
                        .get(&next.source.document_id)
                        .and_then(|group_ix| self.projection.groups.get(*group_ix))
                        .is_some_and(|current| {
                            Arc::ptr_eq(&current.source.document, &next.source.document)
                        })
                })
                .map(|group| group.source.document_id)
                .collect::<BTreeSet<_>>();
            self.visible_lines
                .retain(|(document_id, _)| reusable_documents.contains(document_id));
            self.interaction.text_selections.clear();
            self.projection.content_revision = self.projection.content_revision.saturating_add(1);
        }
        if projection_changed {
            let document_ids = groups
                .iter()
                .map(|group| group.source.document_id)
                .collect::<BTreeSet<_>>();
            self.interaction
                .collapsed_documents
                .retain(|document_id| document_ids.contains(document_id));
            self.projection.group_by_document = groups
                .iter()
                .enumerate()
                .map(|(group_ix, group)| (group.source.document_id, group_ix))
                .collect();
            debug_assert_eq!(self.projection.group_by_document.len(), groups.len());
        }
        self.projection.groups = groups;
        self.projection.max_line_columns = self
            .projection
            .groups
            .iter()
            .map(|group| group.source.document.metadata().longest_line_columns)
            .max()
            .unwrap_or_default();
        if let Some((selected_rows, active_row, selection_anchor)) = stable_interaction {
            self.interaction.row_bounds.borrow_mut().clear();
            self.rebuild_layout();
            self.restore_stable_interaction_rows(selected_rows, active_row, selection_anchor);
        }
    }

    pub fn set_search_matcher(&mut self, matcher: Option<SearchMatcher>) {
        self.presenter.matcher = matcher;
    }

    pub fn update_color_rules(
        &mut self,
        mut rules_for: impl FnMut(&GlobalSearchGroupSource) -> Arc<[ResolvedColorRule]>,
    ) {
        for group in &mut self.projection.groups {
            group.presentation.color_rules = rules_for(&group.source);
        }
    }

    pub fn update_group_title(&mut self, document_id: u64, title: SharedString) {
        let Some(group_ix) = self.projection.group_by_document.get(&document_id) else {
            return;
        };
        self.projection.groups[*group_ix].source.title = title;
    }

    pub fn set_quick_find_matcher(&mut self, matcher: Option<SearchMatcher>) {
        self.presenter.quick_find_matcher = matcher;
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
        self.presenter.show_row_separators = settings.default_show_row_separators;
        self.visible_lines
            .set_overscan(usize::from(settings.viewer_overscan.clamp(4, 40)));
    }

    pub fn set_word_boundary_characters(&mut self, characters: impl Into<SharedString>) {
        self.interaction.word_boundary_characters = characters.into();
    }

    pub fn set_highlight_log_levels(&mut self, enabled: bool) {
        self.presenter.highlight_log_levels = enabled;
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
        self.projection.content_revision
    }

    pub(crate) fn layout_revision(&self) -> u64 {
        self.projection.layout_revision
    }

    pub(crate) fn line_number_width(&self) -> u16 {
        self.presenter.line_number_width
    }

    pub(crate) fn line_number_text_color(&self, cx: &App) -> Hsla {
        self.presenter
            .line_number_text_color
            .unwrap_or(cx.theme().muted_foreground)
    }

    pub(crate) fn line_number_background_color(&self, cx: &App) -> Hsla {
        self.presenter
            .line_number_background_color
            .unwrap_or_else(|| cx.theme().muted.opacity(0.45))
    }

    pub(crate) fn show_line_number_row_separators(&self) -> bool {
        self.presenter.show_line_number_row_separators
    }

    pub(crate) fn show_row_separators(&self) -> bool {
        self.presenter.show_row_separators
    }

    pub fn row(&self, row_ix: usize) -> Option<GlobalSearchRow> {
        match self.row_key(row_ix)? {
            LogRowKey::FileGroup { document_id } => Some(GlobalSearchRow::Group { document_id }),
            LogRowKey::Row {
                document_id,
                source_row,
            } => Some(GlobalSearchRow::Match {
                document_id,
                source_row,
            }),
        }
    }

    pub fn row_ix(&self, target: GlobalSearchRow) -> Option<usize> {
        let key = match target {
            GlobalSearchRow::Group { document_id } => LogRowKey::FileGroup { document_id },
            GlobalSearchRow::Match {
                document_id,
                source_row,
            } => LogRowKey::Row {
                document_id,
                source_row,
            },
        };
        self.row_ix_for_key(key)
    }

    pub(crate) fn row_key(&self, row_ix: usize) -> Option<LogRowKey> {
        match self.flat_row(row_ix)? {
            FlatRow::Group { group_ix } => Some(LogRowKey::FileGroup {
                document_id: self.projection.groups.get(group_ix)?.source.document_id,
            }),
            FlatRow::Match {
                group_ix,
                source_row,
            } => Some(LogRowKey::Row {
                document_id: self.projection.groups.get(group_ix)?.source.document_id,
                source_row,
            }),
        }
    }

    pub(crate) fn row_ix_for_key(&self, key: LogRowKey) -> Option<usize> {
        let document_id = match key {
            LogRowKey::FileGroup { document_id } | LogRowKey::Row { document_id, .. } => {
                document_id
            }
        };
        let group_ix = *self.projection.group_by_document.get(&document_id)?;
        match key {
            LogRowKey::FileGroup { .. } => self.projection.group_starts.get(group_ix).copied(),
            LogRowKey::Row { source_row, .. }
                if !self.interaction.collapsed_documents.contains(&document_id) =>
            {
                self.projection.groups[group_ix]
                    .projection
                    .rows
                    .position(source_row)
                    .and_then(|position| {
                        self.projection
                            .group_starts
                            .get(group_ix)
                            .copied()
                            .and_then(|start| start.checked_add(position.saturating_add(1)))
                    })
            }
            LogRowKey::Row { .. } => None,
        }
    }

    pub(crate) fn row_bounds_handle(&self) -> Rc<RefCell<BTreeMap<usize, Bounds<Pixels>>>> {
        self.interaction.row_bounds.clone()
    }

    fn selected_position_ranges_by_document(&self) -> BTreeMap<u64, Vec<(usize, usize)>> {
        let selection = self.interaction.row_selection.borrow();
        let mut ranges_by_document = BTreeMap::<u64, Vec<(usize, usize)>>::new();
        for (group_ix, group) in self.projection.groups.iter().enumerate() {
            if self
                .interaction
                .collapsed_documents
                .contains(&group.source.document_id)
                || group.projection.rows.is_empty()
            {
                continue;
            }
            let Some(match_start) = self
                .projection
                .group_starts
                .get(group_ix)
                .and_then(|start| start.checked_add(1))
            else {
                continue;
            };
            let match_end = match_start.saturating_add(group.projection.rows.len() - 1);
            let selected_ranges = selection
                .selected_ranges()
                .filter_map(|(first, last)| {
                    let start = first.min(last).max(match_start);
                    let end = first.max(last).min(match_end);
                    (start <= end).then(|| (start - match_start, end - match_start))
                })
                .collect::<Vec<_>>();
            if !selected_ranges.is_empty() {
                ranges_by_document.insert(group.source.document_id, selected_ranges);
            }
        }
        ranges_by_document
    }

    fn position_ranges_for_selected_rows(
        &self,
        selected_rows: &BTreeMap<u64, CompressedRows>,
    ) -> Vec<(usize, usize)> {
        self.projection
            .groups
            .iter()
            .enumerate()
            .flat_map(|(group_ix, group)| {
                let selected = selected_rows.get(&group.source.document_id);
                let group_start = self.projection.group_starts.get(group_ix).copied();
                selected
                    .zip(group_start)
                    .filter(|_| {
                        !self
                            .interaction
                            .collapsed_documents
                            .contains(&group.source.document_id)
                    })
                    .into_iter()
                    .flat_map(move |(selected, group_start)| {
                        group
                            .projection
                            .rows
                            .position_ranges_for_subset(selected)
                            .into_iter()
                            .map(move |(start, end)| {
                                (
                                    group_start.saturating_add(start).saturating_add(1),
                                    group_start.saturating_add(end).saturating_add(1),
                                )
                            })
                    })
            })
            .collect()
    }

    fn stable_interaction_rows(
        &self,
    ) -> (
        BTreeMap<u64, CompressedRows>,
        Option<LogRowKey>,
        Option<LogRowKey>,
    ) {
        let selected_rows = self
            .selected_position_ranges_by_document()
            .into_iter()
            .filter_map(|(document_id, ranges)| {
                let group_ix = *self.projection.group_by_document.get(&document_id)?;
                let rows = self.projection.groups[group_ix]
                    .projection
                    .rows
                    .rows_at_position_ranges(ranges);
                (!rows.is_empty()).then_some((document_id, rows))
            })
            .collect();
        let selection = self.interaction.row_selection.borrow();
        let selection_anchor = selection.anchor().and_then(|row_ix| self.row_key(row_ix));
        let active_row = self
            .interaction
            .active_row
            .get()
            .and_then(|row_ix| self.row_key(row_ix));
        (selected_rows, active_row, selection_anchor)
    }

    fn restore_stable_interaction_rows(
        &self,
        selected_rows: BTreeMap<u64, CompressedRows>,
        active_row: Option<LogRowKey>,
        selection_anchor: Option<LogRowKey>,
    ) {
        let selected_ranges = self.position_ranges_for_selected_rows(&selected_rows);
        let anchor = selection_anchor.and_then(|key| self.row_ix_for_key(key));
        self.interaction
            .row_selection
            .borrow_mut()
            .replace_ranges_with_anchor(selected_ranges, anchor);
        self.interaction
            .active_row
            .set(active_row.and_then(|key| self.row_ix_for_key(key)));
    }

    pub fn collapsed_document_ids(&self) -> BTreeSet<u64> {
        self.interaction.collapsed_documents.clone()
    }

    pub(crate) fn restore_collapsed_document_ids(&mut self, document_ids: &BTreeSet<u64>) {
        let collapsed_documents = self
            .projection
            .groups
            .iter()
            .map(|group| group.source.document_id)
            .filter(|document_id| document_ids.contains(document_id))
            .collect();
        if self.interaction.collapsed_documents == collapsed_documents {
            return;
        }
        self.interaction.collapsed_documents = collapsed_documents;
        self.rebuild_layout();
    }

    pub fn toggle_group(&mut self, document_id: u64) {
        let Some(group_ix) = self.projection.group_by_document.get(&document_id).copied() else {
            return;
        };
        if self.projection.groups[group_ix].projection.rows.is_empty() {
            return;
        }
        if !self.interaction.collapsed_documents.remove(&document_id) {
            self.interaction.collapsed_documents.insert(document_id);
        }
        self.rebuild_layout();
    }

    pub fn group_has_results(&self, document_id: u64) -> bool {
        self.projection
            .group_by_document
            .get(&document_id)
            .and_then(|group_ix| self.projection.groups.get(*group_ix))
            .is_some_and(|group| !group.projection.rows.is_empty())
    }

    pub fn groups_count(&self) -> usize {
        self.projection.groups.len()
    }

    pub fn results_count(&self) -> usize {
        self.projection
            .groups
            .iter()
            .map(|group| group.projection.rows.len())
            .sum()
    }

    /// Result content currently installed in the virtual list, independent of
    /// collapse, selection, highlighting, and other presentation state.
    pub(crate) fn projected_result_groups(
        &self,
    ) -> impl Iterator<Item = (&Path, &Arc<LogDocument>, &CompressedRows)> {
        self.projection.groups.iter().map(|group| {
            (
                group.source.path.as_path(),
                &group.source.document,
                &group.projection.rows,
            )
        })
    }

    pub fn has_truncated_results(&self) -> bool {
        self.projection
            .groups
            .iter()
            .any(|group| group.presentation.truncated)
    }

    pub fn rows_len(&self) -> usize {
        self.projection.rows_len
    }

    pub fn quick_find_groups(&self) -> Vec<GlobalQuickFindGroup> {
        self.projection
            .groups
            .iter()
            .enumerate()
            .filter(|(_, group)| {
                !self
                    .interaction
                    .collapsed_documents
                    .contains(&group.source.document_id)
                    && !group.projection.rows.is_empty()
            })
            .filter_map(|(group_ix, group)| {
                self.projection
                    .group_starts
                    .get(group_ix)
                    .copied()
                    .and_then(|start| start.checked_add(1))
                    .map(|view_start| GlobalQuickFindGroup {
                        view_start,
                        document: group.source.document.clone(),
                        rows: group.projection.rows.clone(),
                    })
            })
            .collect()
    }

    pub(crate) fn log_font_size(&self) -> u16 {
        self.presenter.log_font_size
    }

    pub(crate) fn wrapped_row(&self, row_ix: usize) -> Option<WrappedGlobalRow> {
        match self.flat_row(row_ix)? {
            FlatRow::Group { group_ix } => {
                let group = self.projection.groups.get(group_ix)?;
                Some(WrappedGlobalRow::Group {
                    document_id: group.source.document_id,
                    title: group.source.title.clone(),
                    path: group.source.path.clone(),
                    result_count: group.projection.rows.len(),
                    truncated: group.presentation.truncated,
                    failure: group.presentation.failure.clone(),
                    collapsed: self
                        .interaction
                        .collapsed_documents
                        .contains(&group.source.document_id),
                })
            }
            FlatRow::Match {
                group_ix,
                source_row,
            } => {
                let group = self.projection.groups.get(group_ix)?;
                let presentation = self.row_presentation(group_ix, source_row)?;
                Some(WrappedGlobalRow::Match {
                    document_id: group.source.document_id,
                    source_row,
                    selected: self.interaction.row_selection.borrow().contains(row_ix),
                    marked: group.presentation.marked_rows.contains(source_row),
                    matched: group.presentation.matched_rows.contains(source_row),
                    highlights: presentation.highlights,
                    text: presentation.text,
                    highlight_severity: self.presenter.highlight_log_levels,
                })
            }
        }
    }

    pub(crate) fn prepare_visible_rows(&self, visible_range: Range<usize>) {
        self.visible_lines.prepare_visible_rows(
            visible_range,
            self.projection.rows_len,
            |row_ix| match self.flat_row(row_ix) {
                Some(FlatRow::Match {
                    group_ix,
                    source_row,
                }) => self
                    .projection
                    .groups
                    .get(group_ix)
                    .map(|group| (group.source.document_id, source_row)),
                _ => None,
            },
            |(document_id, source_row), max_bytes| {
                self.projection
                    .group_by_document
                    .get(document_id)
                    .and_then(|group_ix| self.projection.groups.get(*group_ix))
                    .and_then(|group| group.source.document.line_preview(*source_row, max_bytes))
            },
        );
    }

    fn line_text(&self, group_ix: usize, source_row: usize) -> Option<LogText> {
        let group = self.projection.groups.get(group_ix)?;
        self.visible_lines
            .line((group.source.document_id, source_row), |max_bytes| {
                group.source.document.line_preview(source_row, max_bytes)
            })
    }

    fn row_presentation(
        &self,
        group_ix: usize,
        source_row: usize,
    ) -> Option<GlobalRowPresentation> {
        let group = self.projection.groups.get(group_ix)?;
        let text = self.line_text(group_ix, source_row)?;
        Some(
            self.presenter
                .present(text, &group.presentation.color_rules),
        )
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
            .is_pointer_selecting()
    }

    pub(crate) fn pointer_drag_anchor(&self) -> Option<usize> {
        self.interaction
            .row_selection
            .borrow()
            .pointer_drag_anchor()
    }

    pub(crate) fn pointer_text_selection_allowed(&self) -> bool {
        self.interaction
            .row_selection
            .borrow()
            .is_text_selection_allowed()
    }

    pub(crate) fn set_text_selection_suppressed(&self, suppressed: bool) {
        self.interaction.suppress_text_selection.set(suppressed);
    }

    pub(crate) fn is_text_selection_suppressed(&self) -> bool {
        self.interaction.suppress_text_selection.get()
    }

    pub(crate) fn nearest_match_row(&self, row_ix: usize, prefer_after: bool) -> Option<usize> {
        if matches!(self.flat_row(row_ix), Some(FlatRow::Match { .. })) {
            return Some(row_ix);
        }
        let before = (0..row_ix)
            .rev()
            .find(|candidate| matches!(self.flat_row(*candidate), Some(FlatRow::Match { .. })));
        let after = (row_ix.saturating_add(1)..self.projection.rows_len)
            .find(|candidate| matches!(self.flat_row(*candidate), Some(FlatRow::Match { .. })));
        if prefer_after {
            after.or(before)
        } else {
            before.or(after)
        }
    }

    pub(crate) fn extend_keyboard_selection(&self, row_ix: usize) {
        self.interaction
            .row_selection
            .borrow_mut()
            .extend_keyboard_selection(row_ix);
    }

    pub(crate) fn settle_table_selection(&self, row_ix: usize) {
        self.interaction
            .row_selection
            .borrow_mut()
            .settle_table_selection(row_ix);
    }

    pub(crate) fn clear_row_selection(&self) {
        self.interaction.row_selection.borrow_mut().clear();
    }

    pub(crate) fn select_all_rows(&self) {
        let ranges = self
            .projection
            .groups
            .iter()
            .enumerate()
            .filter_map(|(group_ix, group)| {
                if self
                    .interaction
                    .collapsed_documents
                    .contains(&group.source.document_id)
                    || group.projection.rows.is_empty()
                {
                    return None;
                }
                let start = self
                    .projection
                    .group_starts
                    .get(group_ix)?
                    .saturating_add(1);
                Some((start, start.saturating_add(group.projection.rows.len() - 1)))
            })
            .collect::<Vec<_>>();
        self.interaction
            .row_selection
            .borrow_mut()
            .replace_ranges_with_anchor(ranges, None);
    }

    pub(crate) fn selected_rows_count(&self) -> usize {
        let selection = self.interaction.row_selection.borrow();
        selection
            .selected_ranges()
            .map(|(start, end)| {
                let end = end.min(self.projection.rows_len.saturating_sub(1));
                let row_count = end.saturating_sub(start).saturating_add(1);
                let group_count = self
                    .projection
                    .group_starts
                    .partition_point(|row| *row <= end)
                    - self
                        .projection
                        .group_starts
                        .partition_point(|row| *row < start);
                row_count.saturating_sub(group_count)
            })
            .sum()
    }

    pub(crate) fn is_row_selected(&self, row_ix: usize) -> bool {
        matches!(self.flat_row(row_ix), Some(FlatRow::Match { .. }))
            && self.interaction.row_selection.borrow().contains(row_ix)
    }

    pub(crate) fn selected_matches(&self) -> Vec<(u64, usize)> {
        let selection = self.interaction.row_selection.borrow();
        selection
            .selected_indices(self.projection.rows_len)
            .filter_map(|row_ix| match self.flat_row(row_ix)? {
                FlatRow::Group { .. } => None,
                FlatRow::Match {
                    group_ix,
                    source_row,
                } => Some((
                    self.projection.groups[group_ix].source.document_id,
                    source_row,
                )),
            })
            .collect()
    }

    pub(crate) fn selection_snapshot(&self) -> BTreeMap<u64, CompressedRows> {
        self.selected_position_ranges_by_document()
            .into_iter()
            .filter_map(|(document_id, ranges)| {
                let group_ix = *self.projection.group_by_document.get(&document_id)?;
                let rows = self.projection.groups[group_ix]
                    .projection
                    .rows
                    .rows_at_position_ranges(ranges);
                (!rows.is_empty()).then_some((document_id, rows))
            })
            .collect()
    }

    pub(crate) fn restore_selection(&self, snapshot: &BTreeMap<u64, CompressedRows>) {
        let selected_ranges = self.position_ranges_for_selected_rows(snapshot);
        self.interaction
            .row_selection
            .borrow_mut()
            .replace_ranges_with_anchor(selected_ranges, None);
    }

    fn rebuild_layout(&mut self) {
        self.visible_lines.invalidate_window();
        self.projection.layout_revision = self.projection.layout_revision.saturating_add(1);
        self.projection.group_starts.clear();
        self.projection.rows_len = 0;
        for group in &self.projection.groups {
            self.projection.group_starts.push(self.projection.rows_len);
            self.projection.rows_len = self.projection.rows_len.saturating_add(1);
            if !self
                .interaction
                .collapsed_documents
                .contains(&group.source.document_id)
            {
                self.projection.rows_len = self
                    .projection
                    .rows_len
                    .saturating_add(group.projection.rows.len());
            }
        }
    }

    fn flat_row(&self, row_ix: usize) -> Option<FlatRow> {
        if row_ix >= self.projection.rows_len {
            return None;
        }
        let group_ix = self
            .projection
            .group_starts
            .partition_point(|start| *start <= row_ix)
            .saturating_sub(1);
        let group_start = *self.projection.group_starts.get(group_ix)?;
        if row_ix == group_start {
            return Some(FlatRow::Group { group_ix });
        }
        let group = self.projection.groups.get(group_ix)?;
        if self
            .interaction
            .collapsed_documents
            .contains(&group.source.document_id)
        {
            return None;
        }
        let source_row = group
            .projection
            .rows
            .get(row_ix.saturating_sub(group_start + 1))?;
        Some(FlatRow::Match {
            group_ix,
            source_row,
        })
    }
}

impl LogTableCursor for GlobalSearchTableDelegate {
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

impl LogTableRows for GlobalSearchTableDelegate {
    fn prepare_visible_log_rows(&self, visible_range: Range<usize>) {
        self.prepare_visible_rows(visible_range);
    }
}

impl TableDelegate for GlobalSearchTableDelegate {
    fn columns_count(&self, _: &App) -> usize {
        3
    }

    fn rows_count(&self, _: &App) -> usize {
        self.projection.rows_len
    }

    fn column(&self, col_ix: usize, cx: &App) -> Column {
        let base = px(self.presenter.log_font_size as f32);
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
                let width = px(self.presenter.line_number_width as f32);
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
                    self.projection.max_line_columns,
                    self.resolved_font_family(cx),
                    self.presenter.log_font_size,
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
        let row_bounds = self.interaction.row_bounds.clone();
        match self.flat_row(row_ix) {
            Some(FlatRow::Group { group_ix }) => {
                let group = &self.projection.groups[group_ix];
                let document_id = group.source.document_id;
                let header = GlobalSearchGroupHeader::new(
                    group.source.title.clone(),
                    group.source.path.clone(),
                    group.projection.rows.len(),
                    self.resolved_font_family(cx),
                    self.presenter.log_font_size,
                )
                .truncated(group.presentation.truncated)
                .failure(group.presentation.failure.clone())
                .collapsed(self.interaction.collapsed_documents.contains(&document_id));
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
                let group = &self.projection.groups[group_ix];
                let severity = self
                    .presenter
                    .highlight_log_levels
                    .then(|| self.line_text(group_ix, source_row))
                    .flatten()
                    .and_then(|line| severity_style(line.source(), cx));
                div()
                    .id(format!(
                        "global-search-result-{}-{source_row}",
                        group.source.document_id
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
        let line_height = log_line_height(
            self.presenter.log_font_size,
            self.presenter.log_line_spacing,
        );
        match row {
            FlatRow::Group { .. } => div().into_any_element(),
            FlatRow::Match {
                group_ix,
                source_row,
            } => {
                let group = &self.projection.groups[group_ix];
                let selected = self.interaction.row_selection.borrow().contains(row_ix);
                if col_ix == 0 {
                    let marked = group.presentation.marked_rows.contains(source_row);
                    let matched = group.presentation.matched_rows.contains(source_row);
                    let severity_accent = self
                        .presenter
                        .highlight_log_levels
                        .then(|| self.line_text(group_ix, source_row))
                        .flatten()
                        .and_then(|line| severity_style(line.source(), cx))
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
                        self.presenter.log_font_size,
                        line_height,
                        self.line_number_text_color(cx),
                        self.line_number_background_color(cx),
                        self.show_line_number_row_separators(),
                        cx,
                    )
                    .size_full()
                    .into_any_element()
                } else if col_ix == 2 {
                    let presentation =
                        self.row_presentation(group_ix, source_row)
                            .unwrap_or_else(|| GlobalRowPresentation {
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
                    let selection = self.interaction.text_selections.handle(
                        (group.source.document_id, source_row),
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
                                    row_ix + 1 >= self.projection.rows_len
                                        || !self.is_row_selected(row_ix + 1),
                                    cx,
                                ))
                        })
                        .text_size(px(self.presenter.log_font_size as f32))
                        .line_height(line_height)
                        .font_family(self.resolved_font_family(cx))
                        .when(self.presenter.show_row_separators && !selected, |cell| {
                            cell.border_b_1().border_color(cx.theme().border)
                        })
                        .child(
                            SelectableLogText::new(
                                selection,
                                group.source.document_id.rotate_left(32) ^ source_row as u64,
                                text,
                                styled_text,
                                ui_theme::text_selection_highlight(cx),
                            )
                            .word_boundary_characters(
                                self.interaction.word_boundary_characters.clone(),
                            )
                            .suppress_selection(self.is_text_selection_suppressed()),
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
        self.interaction
            .row_bounds
            .borrow_mut()
            .retain(|row_ix, _| visible_range.contains(row_ix));
        self.prepare_visible_rows(visible_range);
    }

    fn cell_text(&self, row_ix: usize, col_ix: usize, _: &App) -> String {
        let Some(row) = self.flat_row(row_ix) else {
            return String::new();
        };
        match row {
            FlatRow::Group { group_ix } if col_ix == 2 => self.projection.groups[group_ix]
                .source
                .path
                .display()
                .to_string(),
            FlatRow::Match { source_row, .. } if col_ix == 1 => (source_row + 1).to_string(),
            FlatRow::Match {
                group_ix,
                source_row,
            } if col_ix == 2 => self
                .line_text(group_ix, source_row)
                .map(|text| text.display().to_string())
                .unwrap_or_default(),
            _ => String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeMap, BTreeSet},
        path::PathBuf,
        sync::Arc,
    };

    use gpui::Bounds;
    use vclogg_core::{CompressedRows, LogDocument};

    use crate::log_table::LogTableCursor;

    use super::{
        GlobalSearchGroup, GlobalSearchGroupIcon, GlobalSearchGroupPresentation,
        GlobalSearchGroupProjection, GlobalSearchGroupSource, GlobalSearchTableDelegate, LogRowKey,
        format_group_result_count, global_search_group_header_presentation,
        global_search_group_title,
    };

    fn test_group(document: Arc<LogDocument>) -> GlobalSearchGroup {
        GlobalSearchGroup {
            source: GlobalSearchGroupSource {
                document_id: 1,
                title: "test.log".into(),
                path: PathBuf::from("test.log"),
                document,
            },
            projection: GlobalSearchGroupProjection {
                rows: [0].into_iter().collect(),
            },
            presentation: GlobalSearchGroupPresentation {
                matched_rows: [0].into_iter().collect(),
                marked_rows: CompressedRows::default(),
                truncated: false,
                failure: None,
                color_rules: Arc::default(),
            },
        }
    }

    #[test]
    fn presentation_only_group_updates_reuse_decoded_lines() {
        let document = Arc::new(LogDocument::placeholder("global-presentation.log"));
        let mut delegate = GlobalSearchTableDelegate::new();
        let group = test_group(document.clone());
        delegate.set_groups(vec![group.clone()]);
        let initial_revision = delegate.content_revision();
        let initial_layout_revision = delegate.layout_revision();
        delegate.visible_lines.prepare_visible_rows(
            0..1,
            1,
            |_| Some((1, 0)),
            |_, _| Some(vclogg_core::LinePreview::new("cached global line", false)),
        );

        let mut changed_presentation = group.clone();
        changed_presentation.projection.rows = [0].into_iter().collect();
        changed_presentation.presentation.marked_rows = [0].into_iter().collect();
        assert!(delegate.has_same_virtual_content(std::slice::from_ref(&changed_presentation)));
        delegate
            .interaction
            .row_bounds
            .borrow_mut()
            .insert(1, Bounds::default());
        delegate.set_groups(vec![changed_presentation]);
        assert_eq!(delegate.content_revision(), initial_revision);
        assert_eq!(delegate.layout_revision(), initial_layout_revision);
        assert!(delegate.interaction.row_bounds.borrow().contains_key(&1));

        let reused = delegate
            .visible_lines
            .line((1, 0), |_| {
                Some(vclogg_core::LinePreview::new("reloaded global line", false))
            })
            .expect("the cached line should remain available");
        assert_eq!(reused.source().as_ref(), "cached global line");

        delegate.update_group_title(1, "renamed.log".into());
        assert_eq!(delegate.projection.groups[0].source.title, "renamed.log");
        let reused = delegate
            .visible_lines
            .line((1, 0), |_| {
                panic!("a title change must not reload log text")
            })
            .expect("the cached line should remain available");
        assert_eq!(reused.source().as_ref(), "cached global line");

        let replacement = test_group(Arc::new(LogDocument::placeholder("replacement.log")));
        assert!(!delegate.has_same_virtual_content(std::slice::from_ref(&replacement)));
        delegate.set_groups(vec![replacement]);
        assert!(delegate.content_revision() > initial_revision);
        assert_eq!(delegate.layout_revision(), initial_layout_revision);
        assert!(
            delegate
                .visible_lines
                .line((1, 0), |_| {
                    panic!("an invalidated virtual window must not read the replacement document")
                })
                .is_none()
        );
        delegate.visible_lines.prepare_visible_rows(
            0..1,
            1,
            |_| Some((1, 0)),
            |_, _| Some(vclogg_core::LinePreview::new("reloaded global line", false)),
        );
        let reloaded = delegate
            .visible_lines
            .line((1, 0), |_| panic!("the prepared line must be cached"))
            .expect("the prepared replacement line should be available");
        assert_eq!(reloaded.source().as_ref(), "reloaded global line");
    }

    #[test]
    fn group_collapse_is_owned_by_interaction_state() {
        let document = Arc::new(LogDocument::placeholder("global-collapse.log"));
        let mut delegate = GlobalSearchTableDelegate::new();
        let mut group = test_group(document);
        group.projection.rows = [2, 5].into_iter().collect();
        delegate.set_groups(vec![group.clone()]);
        delegate.toggle_group(1);

        group.presentation.marked_rows = [5].into_iter().collect();
        delegate.set_groups(vec![group]);

        assert_eq!(delegate.collapsed_document_ids(), BTreeSet::from([1]));
        assert_eq!(delegate.rows_len(), 1);
        assert_eq!(
            delegate
                .projected_result_groups()
                .map(|(path, _, rows)| (path.to_path_buf(), rows.clone()))
                .collect::<Vec<_>>(),
            vec![(PathBuf::from("test.log"), [2, 5].into_iter().collect())]
        );

        delegate.set_groups(Vec::new());
        assert!(delegate.collapsed_document_ids().is_empty());
    }

    #[test]
    fn group_projection_updates_migrate_selection_by_stable_row_key() {
        let document = Arc::new(LogDocument::placeholder("stable-global-projection.log"));
        let mut delegate = GlobalSearchTableDelegate::new();
        let mut group = test_group(document);
        group.projection.rows = [2, 5, 9].into_iter().collect();
        delegate.set_groups(vec![group.clone()]);
        let initial_layout_revision = delegate.layout_revision();
        delegate.settle_table_selection(2);
        delegate.set_active_log_row(Some(2));

        group.projection.rows = [1, 2, 5, 10].into_iter().collect();
        delegate.set_groups(vec![group.clone()]);

        assert!(delegate.layout_revision() > initial_layout_revision);
        assert_eq!(delegate.selected_matches(), vec![(1, 5)]);
        assert_eq!(delegate.active_log_row(), Some(3));
        assert_eq!(
            delegate.interaction.row_selection.borrow().anchor(),
            Some(3)
        );

        group.projection.rows = [1, 10].into_iter().collect();
        delegate.set_groups(vec![group]);

        assert!(delegate.selected_matches().is_empty());
        assert_eq!(delegate.active_log_row(), None);
    }

    #[test]
    fn group_projection_does_not_select_new_rows_between_selected_rows() {
        let document = Arc::new(LogDocument::placeholder("exact-global-projection.log"));
        let mut delegate = GlobalSearchTableDelegate::new();
        let mut group = test_group(document);
        group.projection.rows = [2, 9].into_iter().collect();
        delegate.set_groups(vec![group.clone()]);
        delegate.settle_table_selection(1);
        delegate.extend_keyboard_selection(2);

        group.projection.rows = [2, 5, 9].into_iter().collect();
        delegate.set_groups(vec![group]);

        assert_eq!(delegate.selected_matches(), [(1, 2), (1, 9)]);
        assert_eq!(
            delegate
                .interaction
                .row_selection
                .borrow()
                .selected_ranges()
                .collect::<Vec<_>>(),
            [(1, 1), (3, 3)]
        );
    }

    #[test]
    fn document_index_tracks_reordered_groups() {
        let mut first = test_group(Arc::new(LogDocument::placeholder("first.log")));
        first.source.document_id = 11;
        let mut second = test_group(Arc::new(LogDocument::placeholder("second.log")));
        second.source.document_id = 22;
        let mut delegate = GlobalSearchTableDelegate::new();

        delegate.set_groups(vec![first.clone(), second.clone()]);
        assert_eq!(
            delegate.row_ix_for_key(LogRowKey::FileGroup { document_id: 22 }),
            Some(2)
        );

        delegate.set_groups(vec![second, first]);
        assert_eq!(
            delegate.row_ix_for_key(LogRowKey::FileGroup { document_id: 22 }),
            Some(0)
        );
        assert_eq!(
            delegate.row_ix_for_key(LogRowKey::Row {
                document_id: 11,
                source_row: 0,
            }),
            Some(3)
        );
    }

    #[test]
    fn reordered_groups_restore_selection_in_visual_order() {
        let mut first = test_group(Arc::new(LogDocument::placeholder("first.log")));
        first.source.document_id = 11;
        let mut second = test_group(Arc::new(LogDocument::placeholder("second.log")));
        second.source.document_id = 22;
        let mut delegate = GlobalSearchTableDelegate::new();

        delegate.set_groups(vec![first.clone(), second.clone()]);
        delegate.select_all_rows();
        delegate.set_groups(vec![second, first]);

        assert_eq!(delegate.selected_matches(), vec![(22, 0), (11, 0)]);
        assert_eq!(
            delegate
                .interaction
                .row_selection
                .borrow()
                .selected_ranges()
                .collect::<Vec<_>>(),
            vec![(1, 1), (3, 3)]
        );
    }

    #[test]
    fn select_all_snapshot_keeps_large_result_groups_compressed() {
        let document = Arc::new(LogDocument::placeholder("large-selection.log"));
        let mut group = test_group(document);
        group.projection.rows = CompressedRows::from_inclusive_ranges([(0, 999_999)]);
        let mut delegate = GlobalSearchTableDelegate::new();
        delegate.set_groups(vec![group]);

        delegate.select_all_rows();
        let selected = delegate.selection_snapshot();

        assert_eq!(selected.get(&1).map(CompressedRows::len), Some(1_000_000));
    }

    #[test]
    fn selection_restore_scales_with_selected_rows_and_keeps_display_order() {
        let mut first = test_group(Arc::new(LogDocument::placeholder("first.log")));
        first.source.document_id = 20;
        first.projection.rows = [1, 3].into_iter().collect();
        let mut second = test_group(Arc::new(LogDocument::placeholder("second.log")));
        second.source.document_id = 10;
        second.projection.rows = [2, 4].into_iter().collect();
        let mut delegate = GlobalSearchTableDelegate::new();
        delegate.set_groups(vec![first, second]);

        delegate.restore_selection(&BTreeMap::from([
            (10, [4].into_iter().collect()),
            (20, [1].into_iter().collect()),
            (99, [7].into_iter().collect()),
        ]));

        assert_eq!(delegate.selected_matches(), vec![(20, 1), (10, 4)]);
    }

    #[test]
    fn refreshing_global_results_preserves_drag_cleanup_state() {
        let mut delegate = GlobalSearchTableDelegate::new();
        delegate.begin_pointer_selection(0, false, false, 1);
        delegate.set_text_selection_suppressed(true);

        delegate.set_groups(Vec::new());

        assert!(!delegate.is_pointer_selecting());
        assert!(delegate.is_text_selection_suppressed());

        delegate.end_pointer_selection();
        assert!(!delegate.is_text_selection_suppressed());
    }

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
