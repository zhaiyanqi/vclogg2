use chrono::{DateTime, Local};
use gpui_base::Button as BaseButton;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use gpui::{
    AnyElement, AppContext as _, ClipboardItem, Context, ElementId, Entity, EventEmitter,
    FocusHandle, Focusable as _, InteractiveElement as _, IntoElement, ParentElement as _,
    PathPromptOptions, Pixels, Render, ScrollHandle, SharedString, Size,
    StatefulInteractiveElement as _, Styled as _, Subscription, Task, WeakEntity, Window, div,
    point, prelude::FluentBuilder as _, px, rems, size,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Selectable as _, Sizable as _,
    StyledExt as _, ThemeStyled as _, WindowExt as _,
    button::{Button, ButtonVariant, ButtonVariants as _},
    checkbox::Checkbox,
    dialog::{Cancel, Confirm, DialogButtonProps, DialogFooter},
    h_flex,
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu as _, PopupMenuItem},
    scroll::{ScrollableElement as _, Scrollbar, ScrollbarMode},
    switch::Switch,
    tab::{Tab, TabBar},
    v_flex,
};

use crate::{
    cloud_filters::{
        CloudClient, CloudConnectionProfile, CloudDirectoryPage, CloudFilterItem,
        CloudFilterRevision, CloudFilterRevisionSummary, CloudFilterShareItem, CloudFilterUpdate,
        cloud_error,
    },
    predefined_filters::{
        CloudFilterLocalStatus, FilterBranchId, FilterField, FilterMergeConflict, FilterSnapshot,
        PredefinedFilter, RemoteFilterRelation, attach_published_reference,
        cloud_filter_local_status, create_local_filter_from_cloud, detach_cloud_reference,
        export_filter_json, find_local_filter_by_cloud_id, fork_local_filter,
        keep_local_filter_at_cloud_revision, merge_cloud_filter, merge_filter_collections,
        merge_filter_snapshots, parse_filter_import, remote_deleted_status,
        remote_revision_anomaly, resolve_filter_conflict,
    },
    state_store::CloudSettings,
};

const FILTER_HEADER_HEIGHT_REMS: f32 = 2.4;
const FILTER_ROW_HEIGHT_REMS: f32 = 3.33;
const FILTER_INDEX_WIDTH_REMS: f32 = 3.25;
const LOCAL_NAME_WIDTH_REMS: f32 = 16.5;
const CLOUD_NAME_WIDTH_REMS: f32 = 14.5;
const FILTER_TYPE_WIDTH_REMS: f32 = 5.2;
const LOCAL_ACTIONS_WIDTH_REMS: f32 = 5.5;
const CLOUD_ACTIONS_WIDTH_REMS: f32 = 11.;
const FILTER_DIALOG_MAX_WIDTH_REMS: f32 = 72.;
const FILTER_DIALOG_MAX_HEIGHT_REMS: f32 = 46.;

pub(crate) fn predefined_filters_dialog_size(window: &Window) -> Size<Pixels> {
    let viewport = window.viewport_size();
    size(
        (viewport.width - window.rem_size() * 2.)
            .min(window.rem_size() * FILTER_DIALOG_MAX_WIDTH_REMS)
            .max(px(0.)),
        (viewport.height - window.rem_size() * 4.)
            .min(window.rem_size() * FILTER_DIALOG_MAX_HEIGHT_REMS)
            .max(px(0.)),
    )
}

fn format_cloud_timestamp(timestamp: i64) -> String {
    DateTime::from_timestamp_millis(timestamp)
        .map(|value| {
            value
                .with_timezone(&Local)
                .format("%Y/%m/%d %H:%M:%S")
                .to_string()
        })
        .unwrap_or_else(|| crate::tr!("时间未知", "Time unknown").to_string())
}

fn filter_table_scrollbar(id: &'static str, scroll_handle: &ScrollHandle) -> impl IntoElement {
    div()
        .relative()
        .h_full()
        .w(Scrollbar::width())
        .flex_none()
        .child(
            Scrollbar::vertical(scroll_handle)
                .id(id)
                .mode(ScrollbarMode::Always)
                .viewport_from_layout(),
        )
}

fn outline_icon_button(
    id: impl Into<ElementId>,
    icon: IconName,
    accessibility_label: impl Into<SharedString>,
    disabled: bool,
    focus: Option<&FocusHandle>,
    cx: &gpui::App,
) -> BaseButton {
    BaseButton::new(id)
        .accessibility_label(accessibility_label)
        .when_some(focus.cloned(), |button, focus| button.track_focus(&focus))
        .size_8()
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(cx.theme().radius)
        .border_1()
        .border_color(cx.theme().input)
        .bg(cx.theme().input_background())
        .text_color(cx.theme().button_foreground)
        .when(!disabled, |button| {
            button
                .hover(|button| button.bg(cx.theme().tokens.button_hover))
                .active(|button| button.bg(cx.theme().tokens.button_active))
        })
        .focus_visible(|style| style.border_color(cx.theme().ring))
        .styles(|styles| styles.disabled(|style| style.opacity(0.5)))
        .disabled(disabled)
        .child(Icon::new(icon).small())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DialogTab {
    Local,
    Cloud,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FilterDetailTab {
    Details,
    Revisions,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CloudSort {
    Newest,
    Downloads,
    Likes,
}

fn filter_field_label(field: FilterField) -> &'static str {
    match field {
        FilterField::Name => crate::tr!("名称", "Name"),
        FilterField::Value => crate::tr!("匹配的值", "Match value"),
        FilterField::UseRegex => crate::tr!("正则类型", "Match type"),
        FilterField::Note => crate::tr!("备注", "Note"),
        FilterField::Collaborative => crate::tr!("共创状态", "Collaboration"),
    }
}

fn filter_field_value(snapshot: &FilterSnapshot, field: FilterField) -> String {
    match field {
        FilterField::Name => snapshot.name.clone(),
        FilterField::Value => snapshot.value.clone(),
        FilterField::UseRegex => if snapshot.use_regex {
            crate::tr!("正则", "Regex")
        } else {
            crate::tr!("文本", "Text")
        }
        .to_string(),
        FilterField::Note => {
            if snapshot.note.is_empty() {
                crate::tr!("（空）", "(empty)").to_string()
            } else {
                snapshot.note.clone()
            }
        }
        FilterField::Collaborative => if snapshot.collaborative {
            crate::tr!("允许共创", "Collaborative")
        } else {
            crate::tr!("仅自己维护", "Owner only")
        }
        .to_string(),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum FilterSecondaryRoute {
    LocalDetail(FilterBranchId),
    CloudDetail(String),
    Share,
    Conflict,
}

enum CloudPushResult {
    Updated(
        CloudFilterItem,
        crate::cloud_filters::CloudFilterRevisionPage,
    ),
    RevisionConflict {
        current: CloudFilterItem,
        current_revision: Option<u32>,
    },
}

impl CloudSort {
    fn value(self) -> &'static str {
        match self {
            Self::Newest => "newest",
            Self::Downloads => "downloads",
            Self::Likes => "likes",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Newest => crate::tr!("最新", "Newest"),
            Self::Downloads => crate::tr!("下载最多", "Most downloaded"),
            Self::Likes => crate::tr!("点赞最多", "Most liked"),
        }
    }
}

#[derive(Clone)]
pub enum PredefinedFiltersDialogEvent {
    Filters(Vec<PredefinedFilter>),
    CloudSettings(CloudSettings),
    CloudConnection(Option<CloudConnectionProfile>),
}

struct FilterDraft {
    filter: PredefinedFilter,
    focus: FocusHandle,
    delete_focus: FocusHandle,
    name: Entity<InputState>,
    value: Entity<InputState>,
    note: Entity<InputState>,
}

pub struct PredefinedFiltersDialog {
    rows: Vec<FilterDraft>,
    local_selected: Option<FilterBranchId>,
    local_bulk_selected: BTreeSet<FilterBranchId>,
    secondary_route: Option<FilterSecondaryRoute>,
    local_table_focus: FocusHandle,
    local_scroll: ScrollHandle,
    active_tab: DialogTab,
    cloud_client: Option<CloudClient>,
    cloud_client_error: Option<String>,
    server_url: Entity<InputState>,
    display_name: Entity<InputState>,
    cloud_query: Entity<InputState>,
    cloud_connection: Option<CloudConnectionProfile>,
    cloud_items: Vec<CloudFilterItem>,
    cloud_selected: BTreeSet<String>,
    cloud_row_focus: BTreeMap<String, FocusHandle>,
    cloud_table_focus: FocusHandle,
    cloud_scroll: ScrollHandle,
    cloud_share_selected: BTreeSet<FilterBranchId>,
    cloud_sort: CloudSort,
    cloud_page: u32,
    cloud_page_size: u32,
    cloud_total: u64,
    cloud_offline: bool,
    cloud_cached_at: Option<i64>,
    cloud_detail: Option<CloudFilterItem>,
    cloud_detail_tab: FilterDetailTab,
    cloud_detail_name: Entity<InputState>,
    cloud_detail_value: Entity<InputState>,
    cloud_detail_note: Entity<InputState>,
    cloud_detail_delete_focus: FocusHandle,
    cloud_detail_use_regex: bool,
    cloud_detail_collaborative: bool,
    cloud_revisions: Vec<CloudFilterRevisionSummary>,
    cloud_revision_page: u32,
    cloud_revision_page_size: u32,
    cloud_revision_total: u64,
    cloud_revision: Option<CloudFilterRevision>,
    merge_conflicts: Vec<FilterMergeConflict>,
    conflict_resolution: Option<FilterSnapshot>,
    cloud_message: Option<String>,
    cloud_task: Option<Task<()>>,
    io_task: Option<Task<()>>,
    subscriptions: Vec<Subscription>,
}

struct PredefinedFilterSecondarySurface {
    dialog: WeakEntity<PredefinedFiltersDialog>,
    _dialog_subscription: Subscription,
}

impl PredefinedFilterSecondarySurface {
    fn new(dialog: Entity<PredefinedFiltersDialog>, cx: &mut Context<Self>) -> Self {
        let dialog_subscription = cx.observe(&dialog, |_, _, cx| cx.notify());
        Self {
            dialog: dialog.downgrade(),
            _dialog_subscription: dialog_subscription,
        }
    }
}

impl EventEmitter<PredefinedFiltersDialogEvent> for PredefinedFiltersDialog {}

impl PredefinedFiltersDialog {
    pub fn new(
        filters: Vec<PredefinedFilter>,
        cloud_settings: CloudSettings,
        cloud_client: Option<CloudClient>,
        cloud_connection: Option<CloudConnectionProfile>,
        cloud_client_error: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let server_url = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("https://filters.example.com")
                .default_value(cloud_settings.server_url)
        });
        let display_name = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(crate::tr!("工号或昵称", "Employee ID or nickname"))
                .default_value(cloud_settings.display_name)
        });
        let cloud_query = cx.new(|cx| {
            InputState::new(window, cx).placeholder(crate::tr!(
                "搜索名称、关键词、备注或分享者",
                "Search names, keywords, notes, or owners"
            ))
        });
        let cloud_detail_name = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(crate::tr!("云端过滤器名称", "Cloud filter name"))
        });
        let cloud_detail_value = cx.new(|cx| {
            InputState::new(window, cx).placeholder(crate::tr!(
                "关键词或正则表达式",
                "Keyword or regular expression"
            ))
        });
        let cloud_detail_note = cx.new(|cx| {
            InputState::new(window, cx).placeholder(crate::tr!("备注（可选）", "Note (optional)"))
        });
        let mut this = Self {
            rows: Vec::with_capacity(filters.len()),
            local_selected: None,
            local_bulk_selected: BTreeSet::new(),
            secondary_route: None,
            local_table_focus: cx.focus_handle(),
            local_scroll: ScrollHandle::new(),
            active_tab: DialogTab::Local,
            cloud_client,
            cloud_client_error,
            server_url,
            display_name,
            cloud_query,
            cloud_connection,
            cloud_items: Vec::new(),
            cloud_selected: BTreeSet::new(),
            cloud_row_focus: BTreeMap::new(),
            cloud_table_focus: cx.focus_handle(),
            cloud_scroll: ScrollHandle::new(),
            cloud_share_selected: BTreeSet::new(),
            cloud_sort: CloudSort::Newest,
            cloud_page: 1,
            cloud_page_size: 30,
            cloud_total: 0,
            cloud_offline: false,
            cloud_cached_at: None,
            cloud_detail: None,
            cloud_detail_tab: FilterDetailTab::Details,
            cloud_detail_name,
            cloud_detail_value,
            cloud_detail_note,
            cloud_detail_delete_focus: cx.focus_handle(),
            cloud_detail_use_regex: false,
            cloud_detail_collaborative: false,
            cloud_revisions: Vec::new(),
            cloud_revision_page: 1,
            cloud_revision_page_size: 30,
            cloud_revision_total: 0,
            cloud_revision: None,
            merge_conflicts: Vec::new(),
            conflict_resolution: None,
            cloud_message: None,
            cloud_task: None,
            io_task: None,
            subscriptions: Vec::new(),
        };
        for input in [
            &this.server_url,
            &this.display_name,
            &this.cloud_query,
            &this.cloud_detail_name,
            &this.cloud_detail_value,
            &this.cloud_detail_note,
        ] {
            this.subscriptions
                .push(cx.subscribe(input, |_, _, _: &InputEvent, cx| cx.notify()));
        }
        let cloud_query = this.cloud_query.clone();
        this.subscriptions.push(cx.subscribe_in(
            &cloud_query,
            window,
            |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. })
                    && this.active_tab == DialogTab::Cloud
                    && this.cloud_connection.is_some()
                {
                    this.load_cloud_page(1, window, cx);
                }
            },
        ));
        for filter in filters {
            this.push_filter(filter, window, cx);
        }
        this.local_selected = this.rows.first().map(|row| row.filter.id);
        this
    }

    pub fn accepts_confirm(&self) -> bool {
        self.active_tab == DialogTab::Local
            && self.secondary_route.is_none()
            && self.io_task.is_none()
    }

    fn set_cloud_detail_draft(
        &mut self,
        detail: &CloudFilterItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cloud_detail_name.update(cx, |input, cx| {
            input.set_value(detail.name.clone(), window, cx)
        });
        self.cloud_detail_value.update(cx, |input, cx| {
            input.set_value(detail.value.clone(), window, cx)
        });
        self.cloud_detail_note.update(cx, |input, cx| {
            input.set_value(detail.note.clone(), window, cx)
        });
        self.cloud_detail_use_regex = detail.use_regex;
        self.cloud_detail_collaborative = detail.collaborative;
    }

    fn local_filter_has_publish_changes(filter: &PredefinedFilter, server_url: &str) -> bool {
        filter.tracking_reference(server_url).is_none()
    }

    pub fn filters(&self, cx: &gpui::App) -> Result<Vec<PredefinedFilter>, String> {
        let filters = self.draft_filters(cx);
        for filter in &filters {
            if filter.name.is_empty() {
                return Err(
                    crate::tr!("过滤器名称不能为空", "Filter name can’t be empty").to_string(),
                );
            }
            if filter.value.is_empty() {
                return Err(crate::tr_args!(
                    "“{}”的匹配值不能为空",
                    "The match value for “{}” can’t be empty",
                    filter.name
                ));
            }
        }
        Ok(filters)
    }

    fn draft_filters(&self, cx: &gpui::App) -> Vec<PredefinedFilter> {
        self.rows
            .iter()
            .map(|row| {
                let mut filter = row.filter.clone();
                filter.name = row.name.read(cx).value().trim().to_string();
                filter.value = row.value.read(cx).value().trim().to_string();
                filter.note = row.note.read(cx).value().trim().to_string();
                filter
            })
            .collect()
    }

    fn push_filter(
        &mut self,
        filter: PredefinedFilter,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(crate::tr!("过滤器名称", "Filter name"))
                .default_value(filter.name.clone())
        });
        let value = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(crate::tr!(
                    "关键词或正则表达式",
                    "Keyword or regular expression"
                ))
                .default_value(filter.value.clone())
        });
        let note = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(crate::tr!("备注（可选）", "Note (optional)"))
                .default_value(filter.note.clone())
        });
        for input in [&name, &value, &note] {
            self.subscriptions
                .push(cx.subscribe(input, |_, _, _: &InputEvent, cx| cx.notify()));
        }
        self.rows.push(FilterDraft {
            filter,
            focus: cx.focus_handle().tab_stop(true),
            delete_focus: cx.focus_handle(),
            name,
            value,
            note,
        });
    }

    fn add_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let existing = self
            .rows
            .iter()
            .map(|row| row.filter.clone())
            .collect::<Vec<_>>();
        self.push_filter(PredefinedFilter::new(&existing), window, cx);
        if let Some(row) = self.rows.last() {
            self.local_selected = Some(row.filter.id);
            self.secondary_route = Some(FilterSecondaryRoute::LocalDetail(row.filter.id));
            let focus = row.name.focus_handle(cx);
            let row_focus = row.focus.clone();
            let scroll = self.local_scroll.clone();
            row_focus.focus(window, cx);
            self.open_secondary_dialog(
                crate::tr!("本地过滤器详情", "Local filter details").into(),
                window,
                cx,
            );
            window.defer(cx, move |window, cx| {
                scroll.scroll_to_bottom();
                focus.focus(window, cx);
            });
        }
        cx.notify();
    }

    fn open_local_detail(
        &mut self,
        id: FilterBranchId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.local_selected = Some(id);
        self.secondary_route = Some(FilterSecondaryRoute::LocalDetail(id));
        let focus = self
            .rows
            .iter()
            .find(|row| row.filter.id == id)
            .map(|row| (row.focus.clone(), row.name.focus_handle(cx)));
        if let Some((row_focus, _)) = focus.as_ref() {
            row_focus.focus(window, cx);
        }
        self.open_secondary_dialog(
            crate::tr!("本地过滤器详情", "Local filter details").into(),
            window,
            cx,
        );
        if let Some((_, detail_focus)) = focus {
            Self::defer_focus(detail_focus, window, cx);
        }
        cx.notify();
    }

    fn close_local_detail(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.secondary_route = None;
        window.close_dialog(cx);
        cx.notify();
    }

    fn dismiss_secondary_dialog(&mut self, cx: &mut Context<Self>) {
        if self.secondary_route == Some(FilterSecondaryRoute::Conflict) {
            self.merge_conflicts.clear();
            self.conflict_resolution = None;
        }
        self.secondary_route = None;
        self.cloud_detail = None;
        self.cloud_share_selected.clear();
        self.cloud_detail_tab = FilterDetailTab::Details;
        self.cloud_revision = None;
        self.cloud_revisions.clear();
        self.cloud_revision_page = 1;
        self.cloud_revision_total = 0;
        cx.notify();
    }

    fn open_secondary_dialog(
        &mut self,
        title: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let owner = cx.entity();
        let detail_surface = cx.new(|cx| PredefinedFilterSecondarySurface::new(owner.clone(), cx));
        let dialog_size = predefined_filters_dialog_size(window);
        let margin_top = window.rem_size() * 2.;
        window.open_dialog(cx, move |dialog, _, _| {
            let content = detail_surface.clone();
            let owner = owner.clone();
            dialog
                .w(dialog_size.width)
                .h(dialog_size.height)
                .margin_top(margin_top)
                .title(title.clone())
                .content(move |container, _, _| {
                    container
                        .p_0()
                        .min_h_0()
                        .overflow_hidden()
                        .child(content.clone())
                })
                .on_ok(|_, _, _| false)
                .on_cancel(move |_, _, cx| {
                    owner.update(cx, |this, cx| this.dismiss_secondary_dialog(cx));
                    true
                })
        });
    }

    fn defer_focus(focus: FocusHandle, window: &mut Window, cx: &mut Context<Self>) {
        window.defer(cx, move |window, cx| focus.focus(window, cx));
    }

    fn show_merge_conflicts(
        &mut self,
        conflicts: Vec<FilterMergeConflict>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if conflicts.is_empty() {
            return;
        }
        let replacing_surface = self.secondary_route.is_some();
        self.merge_conflicts.extend(conflicts);
        self.prepare_current_conflict();
        self.secondary_route = Some(FilterSecondaryRoute::Conflict);
        if replacing_surface {
            window.close_dialog(cx);
        }
        self.open_secondary_dialog(
            crate::tr!("解决过滤器冲突", "Resolve filter conflicts").into(),
            window,
            cx,
        );
        cx.notify();
    }

    fn prepare_current_conflict(&mut self) {
        self.conflict_resolution = self.merge_conflicts.first().map(|conflict| {
            merge_filter_snapshots(conflict.base.as_ref(), &conflict.local, &conflict.incoming)
                .snapshot
        });
    }

    fn choose_conflict_field(
        &mut self,
        field: FilterField,
        use_remote: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(conflict) = self.merge_conflicts.first() else {
            return;
        };
        let Some(resolution) = self.conflict_resolution.as_mut() else {
            return;
        };
        let source = if use_remote {
            &conflict.incoming
        } else {
            &conflict.local
        };
        match field {
            FilterField::Name => resolution.name = source.name.clone(),
            FilterField::Value => resolution.value = source.value.clone(),
            FilterField::UseRegex => resolution.use_regex = source.use_regex,
            FilterField::Note => resolution.note = source.note.clone(),
            FilterField::Collaborative => resolution.collaborative = source.collaborative,
        }
        cx.notify();
    }

    fn choose_all_conflict_fields(&mut self, use_remote: bool, cx: &mut Context<Self>) {
        let Some(conflict) = self.merge_conflicts.first() else {
            return;
        };
        self.conflict_resolution = Some(if use_remote {
            conflict.incoming.clone()
        } else {
            conflict.local.clone()
        });
        cx.notify();
    }

    fn finish_current_conflict(
        &mut self,
        apply: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.merge_conflicts.is_empty() {
            return;
        }
        let conflict = self.merge_conflicts.remove(0);
        if apply
            && let Some(resolution) = self.conflict_resolution.take()
            && let Ok(mut filters) = self.filters(cx)
            && let Some(index) = filters.iter().position(|filter| filter.id == conflict.id)
        {
            filters[index] = resolve_filter_conflict(&filters[index], &conflict, resolution);
            self.replace_and_save_filters(filters, window, cx);
        }
        if self.merge_conflicts.is_empty() {
            self.conflict_resolution = None;
            self.secondary_route = None;
            window.close_dialog(cx);
        } else {
            self.prepare_current_conflict();
            self.secondary_route = Some(FilterSecondaryRoute::Conflict);
        }
        cx.notify();
    }

    fn save_filters(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.filters(cx) {
            Ok(filters) => cx.emit(PredefinedFiltersDialogEvent::Filters(filters)),
            Err(error) => window.push_notification(error, cx),
        }
    }

    fn move_filter(&mut self, id: FilterBranchId, direction: isize, cx: &mut Context<Self>) {
        let Some(index) = self.rows.iter().position(|row| row.filter.id == id) else {
            return;
        };
        let target = index.saturating_add_signed(direction);
        if target >= self.rows.len() || target == index {
            return;
        }
        self.rows.swap(index, target);
        cx.notify();
    }

    fn retain_local_bulk_selection(&mut self) {
        let existing_ids = self
            .rows
            .iter()
            .map(|row| row.filter.id)
            .collect::<BTreeSet<_>>();
        self.local_bulk_selected
            .retain(|id| existing_ids.contains(id));
    }

    fn select_all_local_filters(&mut self, selected: bool, cx: &mut Context<Self>) {
        if selected {
            self.local_bulk_selected = self.rows.iter().map(|row| row.filter.id).collect();
        } else {
            self.local_bulk_selected.clear();
        }
        cx.notify();
    }

    fn confirm_remove_selected_filters(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.local_bulk_selected.is_empty() {
            return;
        }
        let selected_ids = self.local_bulk_selected.clone();
        let selected_count = selected_ids.len();
        let dialog = cx.entity();
        window.open_alert_dialog(cx, move |alert, _, cx| {
            let dialog = dialog.clone();
            let selected_ids = selected_ids.clone();
            alert
                .icon(Icon::new(IconName::Info).text_color(cx.theme().danger))
                .title(crate::tr_args!("删除 {selected_count} 个本地过滤器？", "Delete {selected_count} local filters?"))
                .description(crate::tr!("这只会删除所选的本地过滤器；已经发布的云端过滤器不会被删除。", "This deletes only the selected local filters. Published cloud filters won’t be deleted."))
                .button_props(
                    DialogButtonProps::default()
                        .ok_variant(ButtonVariant::Danger)
                        .ok_text(crate::tr!("删除所选", "Delete selected"))
                        .cancel_text(crate::tr!("取消", "Cancel"))
                        .show_cancel(true),
                )
                .on_ok(move |_, _, cx| {
                    dialog.update(cx, |this, cx| {
                        this.rows
                            .retain(|row| !selected_ids.contains(&row.filter.id));
                        this.local_bulk_selected.clear();
                        if this
                            .local_selected
                            .is_some_and(|id| selected_ids.contains(&id))
                        {
                            this.local_selected = this.rows.first().map(|row| row.filter.id);
                        }
                        if matches!(
                            this.secondary_route,
                            Some(FilterSecondaryRoute::LocalDetail(id))
                                if selected_ids.contains(&id)
                        ) {
                            this.secondary_route = None;
                        }
                        cx.notify();
                    });
                    true
                })
        });
    }

    fn confirm_remove_filter(
        &mut self,
        id: FilterBranchId,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(trigger_focus) = self
            .rows
            .iter()
            .find(|row| row.filter.id == id)
            .map(|row| row.delete_focus.clone())
        else {
            return;
        };
        trigger_focus.focus(window, cx);
        let dialog = cx.entity();
        window.open_alert_dialog(cx, move |alert, _, cx| {
            let dialog = dialog.clone();
            let id = id;
            alert
                .icon(Icon::new(IconName::Info).text_color(cx.theme().danger))
                .title(crate::tr!("删除本地过滤器？", "Delete local filter?"))
                .description(crate::tr_args!(
                    "确定删除本地过滤器“{name}”吗？",
                    "Delete the local filter “{name}”?"
                ))
                .button_props(
                    DialogButtonProps::default()
                        .ok_variant(ButtonVariant::Danger)
                        .ok_text(crate::tr!("删除过滤器", "Delete filter"))
                        .cancel_text(crate::tr!("取消", "Cancel"))
                        .show_cancel(true),
                )
                .on_ok(move |_, _, cx| {
                    dialog.update(cx, |this, cx| {
                        let Some(index) = this.rows.iter().position(|row| row.filter.id == id)
                        else {
                            return;
                        };
                        let focus = this.rows[index].delete_focus.clone();
                        let fallback_index = if index + 1 < this.rows.len() {
                            Some(index + 1)
                        } else {
                            index.checked_sub(1)
                        };
                        let fallback_id = fallback_index
                            .map(|fallback_index| this.rows[fallback_index].filter.id);
                        if let Some(fallback_index) = fallback_index {
                            this.rows[fallback_index].focus = focus;
                        } else {
                            this.local_table_focus = focus;
                        }
                        this.rows.remove(index);
                        this.local_bulk_selected.remove(&id);
                        if this.local_selected == Some(id) {
                            this.local_selected = fallback_id;
                        }
                        if this.secondary_route == Some(FilterSecondaryRoute::LocalDetail(id)) {
                            this.secondary_route = None;
                        }
                        cx.notify();
                    });
                    true
                })
        });
    }

    fn import_filters(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.io_task.is_some() {
            return;
        }
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(crate::tr!("导入预定义过滤器", "Import predefined filters").into()),
        });
        self.io_task = Some(cx.spawn_in(window, async move |this, cx| {
            let path = prompt
                .await
                .ok()
                .and_then(Result::ok)
                .flatten()
                .and_then(|mut paths| paths.pop());
            let Some(path) = path else {
                _ = this.update_in(cx, |this, _, cx| {
                    this.io_task = None;
                    cx.notify();
                });
                return;
            };
            let result = cx
                .background_spawn(async move {
                    let text = std::fs::read_to_string(&path)?;
                    parse_filter_import(&text)
                })
                .await;
            _ = this.update_in(cx, |this, window, cx| {
                this.io_task = None;
                match result {
                    Ok(imported) => {
                        let current = this.draft_filters(cx);
                        let merged = merge_filter_collections(&current, imported);
                        let conflicts = merged.conflicts;
                        this.rows.clear();
                        for filter in merged.filters {
                            this.push_filter(filter, window, cx);
                        }
                        this.retain_local_bulk_selection();
                        this.local_selected = this.rows.first().map(|row| row.filter.id);
                        this.secondary_route = None;
                        this.local_scroll.set_offset(point(px(0.), px(0.)));
                        if conflicts.is_empty() {
                            window.push_notification(
                                crate::tr_args!(
                                    "已按 UUID 合并导入，当前共 {} 个过滤器",
                                    "Merged by UUID; {} filters are now available",
                                    this.rows.len()
                                ),
                                cx,
                            );
                        } else {
                            window.push_notification(
                                crate::tr_args!(
                                    "已合入无冲突内容；另有 {} 个同 UUID 冲突保持本地版本",
                                    "Imported non-conflicting content; {} UUID conflicts kept their local versions",
                                    conflicts.len()
                                ),
                                cx,
                            );
                            this.show_merge_conflicts(conflicts, window, cx);
                        }
                    }
                    Err(error) => window.push_notification(crate::tr_args!("导入过滤器失败：{error}", "Couldn’t import filters: {error}"), cx),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn export_filters(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.io_task.is_some() {
            return;
        }
        let filters = match self.filters(cx) {
            Ok(filters) => filters,
            Err(error) => {
                window.push_notification(error, cx);
                return;
            }
        };
        let directory = dirs::document_dir().unwrap_or_else(|| PathBuf::from("."));
        let prompt = cx.prompt_for_new_path(&directory, Some("vclogg2-predefined-filters.json"));
        self.io_task = Some(cx.spawn_in(window, async move |this, cx| {
            let selected_path = prompt.await;
            let result = match selected_path {
                Ok(Ok(Some(path))) => Some(
                    cx.background_spawn(async move {
                        let json = export_filter_json(&filters)?;
                        std::fs::write(&path, json)?;
                        Ok::<_, anyhow::Error>(path)
                    })
                    .await,
                ),
                Ok(Ok(None)) => None,
                Ok(Err(error)) => Some(Err(error)),
                Err(error) => Some(Err(anyhow::anyhow!(error))),
            };
            _ = this.update_in(cx, |this, window, cx| {
                this.io_task = None;
                match result {
                    Some(Ok(path)) => window.push_notification(
                        crate::tr_args!(
                            "已导出预定义过滤器到 {}",
                            "Exported predefined filters to {}",
                            path.display()
                        ),
                        cx,
                    ),
                    Some(Err(error)) => window.push_notification(
                        crate::tr_args!(
                            "导出过滤器失败：{error}",
                            "Couldn’t export filters: {error}"
                        ),
                        cx,
                    ),
                    None => {}
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn cloud_settings(&self, cx: &gpui::App) -> CloudSettings {
        CloudSettings {
            server_url: self.server_url.read(cx).value().trim().to_string(),
            display_name: self.display_name.read(cx).value().trim().to_string(),
        }
    }

    fn replace_and_save_filters(
        &mut self,
        filters: Vec<PredefinedFilter>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.rows.clear();
        for filter in filters {
            self.push_filter(filter, window, cx);
        }
        self.retain_local_bulk_selection();
        self.local_selected = self.rows.first().map(|row| row.filter.id);
        self.local_scroll.set_offset(point(px(0.), px(0.)));
        self.save_filters(window, cx);
        cx.notify();
    }

    fn connect_cloud(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.cloud_task.is_some() {
            return;
        }
        let Some(client) = self.cloud_client.clone() else {
            self.cloud_message = Some(self.cloud_client_error.clone().unwrap_or_else(|| {
                crate::tr!("云端客户端不可用", "Cloud client is unavailable").to_string()
            }));
            cx.notify();
            return;
        };
        let settings = self.cloud_settings(cx);
        if settings.server_url.is_empty() || settings.display_name.is_empty() {
            self.cloud_message = Some(
                crate::tr!(
                    "请填写服务器地址和工号或昵称",
                    "Enter the server address and your employee ID or nickname"
                )
                .to_string(),
            );
            cx.notify();
            return;
        }
        let query = self.cloud_query.read(cx).value().to_string();
        let sort = self.cloud_sort.value().to_string();
        let page_size = self.cloud_page_size;
        self.cloud_message = None;
        self.cloud_task = Some(cx.spawn_in(window, async move |this, cx| {
            let connect_settings = settings.clone();
            let result = cx
                .background_spawn(async move {
                    match client
                        .connect(&connect_settings.server_url, &connect_settings.display_name)
                    {
                        Ok(connection) => {
                            let directory =
                                client.list_filters_resilient(&query, &sort, 1, page_size)?;
                            Ok::<_, anyhow::Error>((connection, directory, None))
                        }
                        Err(connect_error) => {
                            let directory = client.cached_filters(
                                &connect_settings.server_url,
                                &query,
                                &sort,
                                1,
                                page_size,
                            )?;
                            let connection = CloudConnectionProfile {
                                server_url: directory.server_url.clone(),
                                display_name: connect_settings.display_name.clone(),
                                identity_id: String::new(),
                                connected: false,
                                insecure: directory.server_url.starts_with("http://"),
                                default_server_url: String::new(),
                                capabilities: Vec::new(),
                            };
                            Ok((connection, directory, Some(connect_error.to_string())))
                        }
                    }
                })
                .await;
            _ = this.update_in(cx, |this, _, cx| {
                this.cloud_task = None;
                match result {
                    Ok((connection, directory, connect_error)) => {
                        this.cloud_connection = Some(connection);
                        cx.emit(PredefinedFiltersDialogEvent::CloudConnection(
                            this.cloud_connection.clone(),
                        ));
                        this.apply_cloud_directory(directory, cx);
                        this.cloud_selected.clear();
                        this.cloud_message = Some(if this.cloud_offline {
                            crate::tr_args!(
                                "服务器暂不可用，已打开只读离线目录（{}）；{}",
                                "The server is unavailable. Opened the read-only offline directory ({}); {}",
                                this.cloud_cache_age(),
                                connect_error.unwrap_or_else(|| {
                                    crate::tr!("网络请求失败", "Network request failed").to_string()
                                })
                            )
                        } else {
                            crate::tr_args!(
                                "已连接并加载 {} / {} 个云端过滤器",
                                "Connected and loaded {} of {} cloud filters",
                                this.cloud_items.len(),
                                this.cloud_total
                            )
                        });
                        cx.emit(PredefinedFiltersDialogEvent::CloudSettings(settings));
                    }
                    Err(error) => {
                        this.cloud_connection = None;
                        this.cloud_items.clear();
                        this.cloud_selected.clear();
                        this.cloud_row_focus.clear();
                        this.cloud_detail = None;
                        this.secondary_route = None;
                        this.cloud_total = 0;
                        this.cloud_offline = false;
                        this.cloud_cached_at = None;
                        this.cloud_message = Some(crate::tr_args!(
                            "连接云端服务器失败：{error}",
                            "Couldn’t connect to the cloud server: {error}"
                        ));
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn load_cloud_page(&mut self, page: u32, window: &mut Window, cx: &mut Context<Self>) {
        if self.cloud_task.is_some() || self.cloud_connection.is_none() {
            return;
        }
        let Some(client) = self.cloud_client.clone() else {
            return;
        };
        let query = self.cloud_query.read(cx).value().to_string();
        let sort = self.cloud_sort.value().to_string();
        let page_size = self.cloud_page_size;
        let offline_server_url = self
            .cloud_offline
            .then(|| {
                self.cloud_connection
                    .as_ref()
                    .map(|connection| connection.server_url.clone())
            })
            .flatten();
        self.cloud_message = None;
        self.cloud_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    if let Some(server_url) = offline_server_url {
                        client.cached_filters(&server_url, &query, &sort, page.max(1), page_size)
                    } else {
                        client.list_filters_resilient(&query, &sort, page.max(1), page_size)
                    }
                })
                .await;
            _ = this.update_in(cx, |this, _, cx| {
                this.cloud_task = None;
                match result {
                    Ok(directory) => {
                        this.apply_cloud_directory(directory, cx);
                        this.cloud_selected.clear();
                        if this.cloud_offline {
                            this.cloud_message = Some(crate::tr_args!(
                                "正在浏览只读离线目录（{}）",
                                "Browsing the read-only offline directory ({})",
                                this.cloud_cache_age()
                            ));
                        }
                    }
                    Err(error) => {
                        this.cloud_message = Some(crate::tr_args!(
                            "加载云端过滤器失败：{error}",
                            "Couldn’t load cloud filters: {error}"
                        ))
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn apply_cloud_directory(&mut self, directory: CloudDirectoryPage, cx: &mut Context<Self>) {
        self.cloud_items = directory.page.items;
        self.cloud_row_focus
            .retain(|id, _| self.cloud_items.iter().any(|item| &item.id == id));
        for item in &self.cloud_items {
            self.cloud_row_focus
                .entry(item.id.clone())
                .or_insert_with(|| cx.focus_handle().tab_stop(true));
        }
        self.cloud_page = directory.page.page;
        self.cloud_page_size = directory.page.page_size;
        self.cloud_total = directory.page.total;
        self.cloud_detail = None;
        self.cloud_revision = None;
        self.cloud_revisions.clear();
        self.cloud_revision_page = 1;
        self.cloud_revision_total = 0;
        self.cloud_offline = directory.offline;
        self.cloud_cached_at = Some(directory.cached_at);
        self.cloud_scroll.set_offset(point(px(0.), px(0.)));
        if !directory.offline
            && let Some(connection) = self.cloud_connection.as_mut()
        {
            connection.connected = true;
        }
    }

    fn cloud_cache_age(&self) -> String {
        let Some(cached_at) = self.cloud_cached_at else {
            return crate::tr!("缓存时间未知", "Cache time unknown").to_string();
        };
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis() as i64)
            .unwrap_or(cached_at);
        let minutes = now.saturating_sub(cached_at) / 60_000;
        match minutes {
            0 => crate::tr!("刚刚缓存", "Cached just now").to_string(),
            1..=59 => crate::tr_args!("{minutes} 分钟前缓存", "Cached {minutes} minutes ago"),
            60..=1_439 => crate::tr_args!("{} 小时前缓存", "Cached {} hours ago", minutes / 60),
            _ => crate::tr_args!("{} 天前缓存", "Cached {} days ago", minutes / 1_440),
        }
    }

    fn set_cloud_sort(&mut self, sort: CloudSort, window: &mut Window, cx: &mut Context<Self>) {
        if self.cloud_sort == sort {
            return;
        }
        self.cloud_sort = sort;
        self.load_cloud_page(1, window, cx);
        cx.notify();
    }

    fn download_cloud_filters(
        &mut self,
        filter_ids: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.cloud_task.is_some() {
            return;
        }
        let Some(connection) = self.cloud_connection.clone() else {
            return;
        };
        let Some(client) = self.cloud_client.clone() else {
            return;
        };
        let mut filters = match self.filters(cx) {
            Ok(filters) => filters,
            Err(error) => {
                self.cloud_message = Some(error);
                cx.notify();
                return;
            }
        };
        let items = self
            .cloud_items
            .iter()
            .filter(|item| filter_ids.contains(&item.id))
            .cloned()
            .collect::<Vec<_>>();
        let mut changed = Vec::new();
        let mut merge_conflicts = Vec::new();
        let mut protected_conflicts = 0;
        for item in &items {
            match cloud_filter_local_status(&filters, &connection.server_url, item) {
                CloudFilterLocalStatus::NotDownloaded => {
                    if let Ok(filter) =
                        create_local_filter_from_cloud(&filters, &connection.server_url, item)
                    {
                        filters.push(filter);
                        changed.push((item.id.clone(), item.revision));
                    }
                }
                CloudFilterLocalStatus::RemoteUpdated | CloudFilterLocalStatus::AutoMerge => {
                    if let Some(index) = filters
                        .iter()
                        .position(|filter| filter.id.to_string() == item.id)
                    {
                        match merge_cloud_filter(&filters[index], &connection.server_url, item) {
                            Ok((updated, _)) => {
                                filters[index] = updated;
                                changed.push((item.id.clone(), item.revision));
                            }
                            Err(conflict) => merge_conflicts.push(conflict),
                        }
                    }
                }
                CloudFilterLocalStatus::Conflict => {
                    if let Some(local) = find_local_filter_by_cloud_id(&filters, &item.id) {
                        if remote_revision_anomaly(local, &connection.server_url, item) {
                            protected_conflicts += 1;
                        } else if let Err(conflict) =
                            merge_cloud_filter(local, &connection.server_url, item)
                        {
                            merge_conflicts.push(conflict);
                        }
                    }
                }
                CloudFilterLocalStatus::Synced
                | CloudFilterLocalStatus::LocalModified
                | CloudFilterLocalStatus::RemoteDeleted
                | CloudFilterLocalStatus::ProtocolUnsupported => {}
            }
        }
        let conflicts = merge_conflicts.len() + protected_conflicts;
        if changed.is_empty() {
            self.cloud_message = Some(if conflicts > 0 {
                crate::tr_args!(
                    "检测到 {conflicts} 个本地与云端双向修改，未覆盖本地版本",
                    "Found {conflicts} filters changed both locally and remotely; local versions were preserved"
                )
            } else {
                crate::tr!(
                    "所选云端过滤器已是最新版本",
                    "The selected cloud filters are up to date"
                )
                .to_string()
            });
            if !merge_conflicts.is_empty() {
                self.show_merge_conflicts(merge_conflicts, window, cx);
            }
            cx.notify();
            return;
        }
        if self.cloud_offline {
            self.replace_and_save_filters(filters, window, cx);
            self.cloud_selected.clear();
            self.cloud_message = Some(if conflicts == 0 {
                crate::tr_args!(
                    "已从离线缓存导入或更新 {} 个过滤器",
                    "Imported or updated {} filters from the offline cache",
                    changed.len()
                )
            } else {
                crate::tr_args!(
                    "已从离线缓存导入或更新 {} 个过滤器；另有 {conflicts} 个冲突未覆盖",
                    "Imported or updated {} filters from the offline cache; {conflicts} conflicts were preserved",
                    changed.len()
                )
            });
            if !merge_conflicts.is_empty() {
                self.show_merge_conflicts(merge_conflicts, window, cx);
            }
            cx.notify();
            return;
        }
        self.cloud_task = Some(cx.spawn_in(window, async move |this, cx| {
            let report = changed.clone();
            let result = cx
                .background_spawn(async move { client.record_downloads(&report) })
                .await;
            _ = this.update_in(cx, |this, window, cx| {
                this.cloud_task = None;
                this.replace_and_save_filters(filters, window, cx);
                this.cloud_selected.clear();
                this.cloud_message = Some(match result {
                    Ok(_) if conflicts == 0 => crate::tr_args!(
                        "已下载或更新 {} 个过滤器",
                        "Downloaded or updated {} filters",
                        changed.len()
                    ),
                    Ok(_) => crate::tr_args!(
                        "已下载或更新 {} 个过滤器；另有 {conflicts} 个冲突未覆盖",
                        "Downloaded or updated {} filters; {conflicts} conflicts were preserved",
                        changed.len()
                    ),
                    Err(error) => crate::tr_args!(
                        "已更新本地过滤器，但下载统计上报失败：{error}",
                        "Local filters were updated, but download reporting failed: {error}"
                    ),
                });
                if !merge_conflicts.is_empty() {
                    this.show_merge_conflicts(merge_conflicts, window, cx);
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn open_cloud_share(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(connection) = self.cloud_connection.as_ref() else {
            return false;
        };
        if !connection.supports_uuid_filter_branches() {
            self.cloud_message = Some(
                crate::tr!(
                    "当前服务器不支持 UUID 分支协议；仍可浏览和下载，请升级服务器后再分享",
                    "This server doesn’t support UUID branches. You can still browse and download; upgrade the server before sharing"
                )
                .to_string(),
            );
            cx.notify();
            return false;
        }
        let filters = match self.filters(cx) {
            Ok(filters) => filters,
            Err(error) => {
                self.cloud_message = Some(error);
                cx.notify();
                return false;
            }
        };
        self.cloud_share_selected = filters
            .iter()
            .filter(|filter| Self::local_filter_has_publish_changes(filter, &connection.server_url))
            .map(|filter| filter.id)
            .collect();
        self.cloud_detail = None;
        self.secondary_route = Some(FilterSecondaryRoute::Share);
        self.cloud_revision = None;
        self.cloud_revisions.clear();
        self.cloud_message = None;
        cx.notify();
        true
    }

    fn show_cloud_share(
        &mut self,
        filter_ids: Option<BTreeSet<FilterBranchId>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.open_cloud_share(cx) {
            return;
        }
        if let Some(filter_ids) = filter_ids {
            self.cloud_share_selected
                .retain(|selected| filter_ids.contains(selected));
        }
        self.open_secondary_dialog(crate::tr!("分享过滤器", "Share filters").into(), window, cx);
        cx.notify();
    }

    fn select_cloud_page(&mut self, selected: bool, cx: &mut Context<Self>) {
        if selected {
            self.cloud_selected = self
                .cloud_items
                .iter()
                .map(|item| item.id.clone())
                .collect();
        } else {
            self.cloud_selected.clear();
        }
        cx.notify();
    }

    fn share_local_filters(
        &mut self,
        filter_ids: Vec<FilterBranchId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.cloud_task.is_some() {
            return;
        }
        let Some(connection) = self.cloud_connection.clone() else {
            return;
        };
        if !connection.supports_uuid_filter_branches() {
            self.cloud_message = Some(
                crate::tr!(
                    "当前服务器不支持 UUID 分支协议；仍可浏览和下载，请升级服务器后再分享",
                    "This server doesn’t support UUID branches. You can still browse and download; upgrade the server before sharing"
                )
                .to_string(),
            );
            cx.notify();
            return;
        }
        let Some(client) = self.cloud_client.clone() else {
            return;
        };
        let all_filters = match self.filters(cx) {
            Ok(filters) if !filters.is_empty() => filters,
            Ok(_) => {
                self.cloud_message = Some(
                    crate::tr!(
                        "没有可分享的本地过滤器",
                        "There are no local filters to share"
                    )
                    .to_string(),
                );
                cx.notify();
                return;
            }
            Err(error) => {
                self.cloud_message = Some(error);
                cx.notify();
                return;
            }
        };
        let selected_filters = all_filters
            .iter()
            .filter(|filter| filter_ids.contains(&filter.id))
            .filter(|filter| Self::local_filter_has_publish_changes(filter, &connection.server_url))
            .cloned()
            .collect::<Vec<_>>();
        if selected_filters.is_empty() {
            self.cloud_message = Some(
                crate::tr!(
                    "请选择至少一个尚未发布的本地过滤器",
                    "Select at least one local filter that hasn’t been published"
                )
                .to_string(),
            );
            cx.notify();
            return;
        }
        let items = selected_filters
            .iter()
            .map(|filter| CloudFilterShareItem {
                client_filter_id: filter.id.to_string(),
                name: filter.name.clone(),
                value: filter.value.clone(),
                use_regex: filter.use_regex,
                note: filter.note.clone(),
                derived_from_filter_id: filter
                    .remote_references
                    .iter()
                    .find(|reference| {
                        reference.relation == RemoteFilterRelation::DerivedFrom
                            && reference.server_url == connection.server_url
                    })
                    .map(|reference| reference.filter_id.to_string()),
                collaborative: Some(filter.collaborative),
                base_revision: None,
            })
            .collect::<Vec<_>>();
        self.cloud_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move { client.share_filters(&items) })
                .await;
            _ = this.update_in(cx, |this, window, cx| {
                this.cloud_task = None;
                match result {
                    Ok(results) => {
                        let share_was_open =
                            this.secondary_route == Some(FilterSecondaryRoute::Share);
                        let mut updated = all_filters;
                        for result in &results {
                            if let Some(index) = updated
                                .iter()
                                .position(|filter| filter.id.to_string() == result.client_filter_id)
                            {
                                let Ok(published) = attach_published_reference(
                                    &updated[index],
                                    &connection.server_url,
                                    result.filter_id.clone(),
                                    result.revision,
                                    &connection.identity_id,
                                    &connection.display_name,
                                ) else {
                                    this.cloud_message = Some(
                                        crate::tr!(
                                            "服务器返回的 UUID 与本地过滤器不一致，未更新本地基线",
                                            "The UUID returned by the server doesn’t match the local filter; the local baseline wasn’t updated"
                                        )
                                        .to_string(),
                                    );
                                    continue;
                                };
                                updated[index] = published;
                            }
                        }
                        this.replace_and_save_filters(updated, window, cx);
                        this.secondary_route = None;
                        this.cloud_share_selected.clear();
                        let message = crate::tr_args!(
                            "已分享 {} 个过滤器",
                            "Shared {} filters",
                            results.len()
                        );
                        this.cloud_message = Some(message.clone());
                        window.push_notification(message, cx);
                        if share_was_open {
                            window.close_dialog(cx);
                        }
                    }
                    Err(error) => {
                        this.cloud_message = Some(crate::tr_args!(
                            "分享本地过滤器失败：{error}",
                            "Couldn’t share local filters: {error}"
                        ))
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn toggle_cloud_like(&mut self, filter_id: String, cx: &mut Context<Self>) {
        if self.cloud_task.is_some() {
            return;
        }
        let Some(client) = self.cloud_client.clone() else {
            return;
        };
        let Some(item) = self.cloud_items.iter().find(|item| item.id == filter_id) else {
            return;
        };
        let liked = !item.liked;
        let request_filter_id = filter_id.clone();
        self.cloud_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { client.set_liked(&request_filter_id, liked) })
                .await;
            _ = this.update(cx, |this, cx| {
                this.cloud_task = None;
                match result {
                    Ok((liked, like_count)) => {
                        if let Some(item) = this
                            .cloud_items
                            .iter_mut()
                            .find(|item| item.id == filter_id)
                        {
                            item.liked = liked;
                            item.like_count = like_count;
                        }
                    }
                    Err(error) => {
                        this.cloud_message = Some(crate::tr_args!(
                            "点赞操作失败：{error}",
                            "Couldn’t update the like: {error}"
                        ))
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn open_cloud_detail(
        &mut self,
        filter_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.cloud_task.is_some() || self.secondary_route.is_some() {
            return;
        }
        let Some(summary) = self
            .cloud_items
            .iter()
            .find(|item| item.id == filter_id)
            .cloned()
        else {
            return;
        };
        self.secondary_route = Some(FilterSecondaryRoute::CloudDetail(filter_id.clone()));
        self.cloud_share_selected.clear();
        self.cloud_row_focus
            .get(&filter_id)
            .unwrap_or(&self.cloud_table_focus)
            .focus(window, cx);
        self.set_cloud_detail_draft(&summary, window, cx);
        self.cloud_detail = Some(summary.clone());
        self.cloud_detail_tab = FilterDetailTab::Details;
        self.cloud_revisions.clear();
        self.cloud_revision = None;
        self.cloud_revision_page = 1;
        self.cloud_revision_total = 0;
        self.open_secondary_dialog(
            crate::tr!("云端过滤器详情", "Cloud filter details").into(),
            window,
            cx,
        );
        if self.cloud_offline {
            self.cloud_message = Some(
                crate::tr!(
                    "离线详情来自本机缓存；修订历史和云端修改需恢复连接后使用",
                    "Offline details come from the local cache. Reconnect to use revision history and cloud editing"
                )
                .to_string(),
            );
            cx.notify();
            return;
        }
        let Some(client) = self.cloud_client.clone() else {
            return;
        };
        let requested_filter_id = filter_id.clone();
        self.cloud_message = None;
        self.cloud_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let detail = client.get_filter(&filter_id)?;
                    let revisions = client.list_revisions(&filter_id, 1, 30)?;
                    Ok::<_, anyhow::Error>((detail, revisions))
                })
                .await;
            _ = this.update_in(cx, |this, window, cx| {
                this.cloud_task = None;
                match result {
                    Ok((detail, revisions)) => {
                        if this.cloud_detail.as_ref().map(|detail| detail.id.as_str())
                            != Some(requested_filter_id.as_str())
                        {
                            cx.notify();
                            return;
                        }
                        this.set_cloud_detail_draft(&detail, window, cx);
                        this.cloud_detail = Some(detail);
                        this.cloud_revisions = revisions.items;
                        this.cloud_revision_page = revisions.page;
                        this.cloud_revision_page_size = revisions.page_size;
                        this.cloud_revision_total = revisions.total;
                        this.cloud_revision = None;
                    }
                    Err(error) => {
                        this.cloud_message = Some(crate::tr_args!(
                            "读取云端过滤器详情失败：{error}",
                            "Couldn’t read cloud filter details: {error}"
                        ))
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn load_cloud_revision_page(&mut self, page: u32, window: &mut Window, cx: &mut Context<Self>) {
        if self.cloud_task.is_some() {
            return;
        }
        let Some(detail) = self.cloud_detail.as_ref() else {
            return;
        };
        let Some(client) = self.cloud_client.clone() else {
            return;
        };
        let filter_id = detail.id.clone();
        let page_size = self.cloud_revision_page_size;
        self.cloud_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    client.list_revisions(&filter_id, page.max(1), page_size)
                })
                .await;
            _ = this.update_in(cx, |this, _, cx| {
                this.cloud_task = None;
                match result {
                    Ok(revisions) => {
                        this.cloud_revisions = revisions.items;
                        this.cloud_revision_page = revisions.page;
                        this.cloud_revision_page_size = revisions.page_size;
                        this.cloud_revision_total = revisions.total;
                        this.cloud_revision = None;
                    }
                    Err(error) => {
                        this.cloud_message = Some(crate::tr_args!(
                            "读取云端修订列表失败：{error}",
                            "Couldn’t read the cloud revision list: {error}"
                        ))
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn load_cloud_revision(&mut self, revision: u32, window: &mut Window, cx: &mut Context<Self>) {
        if self.cloud_task.is_some() {
            return;
        }
        let Some(detail) = self.cloud_detail.as_ref() else {
            return;
        };
        let Some(client) = self.cloud_client.clone() else {
            return;
        };
        let filter_id = detail.id.clone();
        self.cloud_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move { client.get_revision(&filter_id, revision) })
                .await;
            _ = this.update_in(cx, |this, _, cx| {
                this.cloud_task = None;
                match result {
                    Ok(revision) => this.cloud_revision = Some(revision),
                    Err(error) => {
                        this.cloud_message = Some(crate::tr_args!(
                            "读取云端修订失败：{error}",
                            "Couldn’t read the cloud revision: {error}"
                        ))
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn push_local_filter_to_cloud(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.cloud_task.is_some() {
            return;
        }
        let Some(connection) = self.cloud_connection.clone() else {
            return;
        };
        if !connection.supports_uuid_filter_branches() {
            self.cloud_message = Some(
                crate::tr!(
                    "当前服务器不支持 UUID 分支协议；请升级服务器后再提交本地修改",
                    "This server doesn’t support UUID branches. Upgrade it before submitting local changes"
                )
                .to_string(),
            );
            cx.notify();
            return;
        }
        let Some(detail) = self.cloud_detail.clone() else {
            return;
        };
        let Some(client) = self.cloud_client.clone() else {
            return;
        };
        let filters = match self.filters(cx) {
            Ok(filters) => filters,
            Err(error) => {
                self.cloud_message = Some(error);
                cx.notify();
                return;
            }
        };
        let Some(local) = find_local_filter_by_cloud_id(&filters, &detail.id).cloned() else {
            self.cloud_message = Some(
                crate::tr!(
                    "本地没有关联的过滤器可提交",
                    "No linked local filter is available to submit"
                )
                .to_string(),
            );
            cx.notify();
            return;
        };
        if !detail.can_edit {
            self.cloud_message = Some(
                crate::tr!(
                    "分享者未开放共创，当前身份不能编辑该云端过滤器",
                    "The owner hasn’t enabled collaboration, so this identity can’t edit the cloud filter"
                )
                .to_string(),
            );
            cx.notify();
            return;
        }
        let Some(base_revision) = local
            .tracking_reference(&connection.server_url)
            .map(|reference| reference.revision)
        else {
            self.cloud_message = Some(
                crate::tr!(
                    "本地过滤器没有可用的远程共同基线",
                    "The local filter has no shared remote baseline"
                )
                .to_string(),
            );
            cx.notify();
            return;
        };
        let update = CloudFilterUpdate {
            name: local.name.clone(),
            value: local.value.clone(),
            use_regex: local.use_regex,
            note: local.note.clone(),
            collaborative: detail.can_delete.then_some(local.collaborative),
            base_revision: Some(base_revision),
        };
        let filter_id = detail.id.clone();
        let open_filter_id = filter_id.clone();
        self.cloud_task = Some(cx.spawn_in(window, async move |this, cx| {
            let request_filter_id = filter_id.clone();
            let result = cx
                .background_spawn(async move {
                    match client.update_filter(&request_filter_id, &update) {
                        Ok(_) => {
                            let updated = client.get_filter(&request_filter_id)?;
                            let revisions = client.list_revisions(&request_filter_id, 1, 30)?;
                            Ok::<_, anyhow::Error>(CloudPushResult::Updated(updated, revisions))
                        }
                        Err(error)
                            if cloud_error(&error).and_then(|error| error.code())
                                == Some("revision_conflict") =>
                        {
                            let current_revision =
                                cloud_error(&error).and_then(|error| error.current_revision());
                            let current = client.get_filter(&request_filter_id)?;
                            Ok(CloudPushResult::RevisionConflict {
                                current,
                                current_revision,
                            })
                        }
                        Err(error) => Err(error),
                    }
                })
                .await;
            _ = this.update_in(cx, |this, window, cx| {
                this.cloud_task = None;
                let detail_is_open = this
                    .cloud_detail
                    .as_ref()
                    .is_some_and(|detail| detail.id == open_filter_id);
                match result {
                    Ok(CloudPushResult::Updated(updated_item, revisions)) => {
                        let mut updated_filters = filters;
                        if let Some(index) = updated_filters
                            .iter()
                            .position(|filter| filter.id == local.id)
                        {
                            let Ok(updated_local) = keep_local_filter_at_cloud_revision(
                                &updated_filters[index],
                                &connection.server_url,
                                &updated_item,
                            ) else {
                                this.cloud_message = Some(
                                    crate::tr!(
                                        "服务器返回了无效的过滤器 UUID",
                                        "The server returned an invalid filter UUID"
                                    )
                                    .to_string(),
                                );
                                cx.notify();
                                return;
                            };
                            updated_filters[index] = updated_local;
                        }
                        if let Some(item) = this
                            .cloud_items
                            .iter_mut()
                            .find(|item| item.id == updated_item.id)
                        {
                            *item = updated_item.clone();
                        }
                        if detail_is_open {
                            this.set_cloud_detail_draft(&updated_item, window, cx);
                            this.cloud_detail = Some(updated_item);
                            this.cloud_revisions = revisions.items;
                            this.cloud_revision_page = revisions.page;
                            this.cloud_revision_page_size = revisions.page_size;
                            this.cloud_revision_total = revisions.total;
                            this.cloud_revision = None;
                        }
                        this.replace_and_save_filters(updated_filters, window, cx);
                        this.cloud_message = Some(
                            crate::tr!(
                                "已将本地修改提交为新的云端修订",
                                "Submitted local changes as a new cloud revision"
                            )
                            .to_string(),
                        );
                    }
                    Ok(CloudPushResult::RevisionConflict {
                        current,
                        current_revision,
                    }) => {
                        if let Some(item) = this
                            .cloud_items
                            .iter_mut()
                            .find(|item| item.id == current.id)
                        {
                            *item = current.clone();
                        }
                        if detail_is_open {
                            this.set_cloud_detail_draft(&current, window, cx);
                            this.cloud_detail = Some(current.clone());
                        }
                        if let Some(index) = filters.iter().position(|filter| filter.id == local.id)
                        {
                            match merge_cloud_filter(
                                &filters[index],
                                &connection.server_url,
                                &current,
                            ) {
                                Ok((merged, _)) => {
                                    let mut updated_filters = filters;
                                    updated_filters[index] = merged;
                                    this.replace_and_save_filters(updated_filters, window, cx);
                                    this.cloud_message = Some(crate::tr_args!(
                                        "云端已更新到修订 {}，已安全合并到本地；请检查后再次提交",
                                        "The cloud filter advanced to revision {} and was safely merged locally. Review it, then submit again",
                                        current_revision.unwrap_or(current.revision)
                                    ));
                                }
                                Err(conflict) => {
                                    this.cloud_message = Some(crate::tr_args!(
                                        "云端已更新到修订 {}，本地保持不变；请先解决字段冲突",
                                        "The cloud filter advanced to revision {}. The local filter was preserved; resolve field conflicts first",
                                        current_revision.unwrap_or(current.revision)
                                    ));
                                    if remote_revision_anomaly(
                                        &filters[index],
                                        &connection.server_url,
                                        &current,
                                    ) {
                                        this.cloud_message = Some(
                                            crate::tr!(
                                                "服务器返回的修订发生倒退或同修订内容不一致；本地保持不变",
                                                "The server returned an older revision or inconsistent content for the same revision; the local filter was preserved"
                                            )
                                            .to_string(),
                                        );
                                    } else {
                                        this.show_merge_conflicts(vec![conflict], window, cx);
                                    }
                                }
                            }
                        }
                    }
                    Err(error) => {
                        this.cloud_message = Some(crate::tr_args!(
                            "同步本地修改失败：{error}",
                            "Couldn’t synchronize local changes: {error}"
                        ))
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn replace_local_with_cloud_detail(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(server_url) = self
            .cloud_connection
            .as_ref()
            .map(|connection| connection.server_url.clone())
        else {
            return;
        };
        let Some(detail) = self.cloud_detail.clone() else {
            return;
        };
        let Ok(mut filters) = self.filters(cx) else {
            return;
        };
        let Some(index) = filters.iter().position(|filter| {
            find_local_filter_by_cloud_id(std::slice::from_ref(filter), &detail.id).is_some()
        }) else {
            match create_local_filter_from_cloud(&filters, &server_url, &detail) {
                Ok(filter) => {
                    filters.push(filter);
                    self.replace_and_save_filters(filters, window, cx);
                    self.cloud_message = Some(
                        crate::tr!(
                            "已下载到本地并建立远程基线",
                            "Downloaded locally and established the remote baseline"
                        )
                        .to_string(),
                    );
                }
                Err(error) => {
                    self.cloud_message = Some(crate::tr_args!(
                        "下载到本地失败：{error}",
                        "Couldn’t download locally: {error}"
                    ))
                }
            }
            cx.notify();
            return;
        };
        match merge_cloud_filter(&filters[index], &server_url, &detail) {
            Ok((updated, _)) => {
                filters[index] = updated;
                self.replace_and_save_filters(filters, window, cx);
                self.cloud_message = Some(
                    crate::tr!(
                        "已将可安全合并的云端更新应用到本地",
                        "Applied the cloud updates that could be merged safely"
                    )
                    .to_string(),
                );
            }
            Err(conflict) => {
                if remote_revision_anomaly(&filters[index], &server_url, &detail) {
                    self.cloud_message = Some(
                        crate::tr!(
                            "服务器返回的修订发生倒退或同修订内容不一致；本地保持不变",
                            "The server returned an older revision or inconsistent content for the same revision; the local filter was preserved"
                        )
                        .to_string(),
                    );
                } else {
                    self.cloud_message = Some(
                        crate::tr!(
                            "该过滤器存在字段冲突，请逐项选择要保留的内容",
                            "This filter has field conflicts. Choose which value to keep for each field"
                        )
                        .to_string(),
                    );
                    self.show_merge_conflicts(vec![conflict], window, cx);
                }
            }
        }
        cx.notify();
    }

    fn confirm_restore_cloud_revision(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(revision) = self
            .cloud_revision
            .as_ref()
            .filter(|revision| !revision.current)
        else {
            return;
        };
        let revision_number = revision.revision;
        let dialog = cx.entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let dialog = dialog.clone();
            alert
                .title(crate::tr_args!("恢复修订 {revision_number}？", "Restore revision {revision_number}?"))
                .description(crate::tr_args!(
                    "所选历史内容会恢复到本地，形成待提交的修改；云端修订 {revision_number} 不会改变。",
                    "The selected history will be restored locally as pending changes. Cloud revision {revision_number} won’t change."
                ))
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(crate::tr!("恢复到本地", "Restore locally"))
                        .cancel_text(crate::tr!("取消", "Cancel"))
                        .show_cancel(true),
                )
                .on_ok(move |_, window, cx| {
                    dialog.update(cx, |this, cx| this.restore_cloud_revision(window, cx));
                    true
                })
        });
    }

    fn restore_cloud_revision(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(detail) = self.cloud_detail.clone() else {
            return;
        };
        let Some(revision) = self.cloud_revision.clone() else {
            return;
        };
        if revision.current {
            return;
        }
        let Some(connection) = self.cloud_connection.as_ref() else {
            return;
        };
        let Ok(mut filters) = self.filters(cx) else {
            return;
        };
        let local = if let Some(index) = filters
            .iter()
            .position(|filter| filter.id.to_string() == detail.id)
        {
            let Ok(local) = keep_local_filter_at_cloud_revision(
                &filters[index],
                &connection.server_url,
                &detail,
            ) else {
                self.cloud_message = Some(
                    crate::tr!(
                        "服务器返回了无效的过滤器 UUID",
                        "The server returned an invalid filter UUID"
                    )
                    .to_string(),
                );
                cx.notify();
                return;
            };
            filters[index] = local;
            index
        } else {
            let Ok(local) =
                create_local_filter_from_cloud(&filters, &connection.server_url, &detail)
            else {
                self.cloud_message = Some(
                    crate::tr!(
                        "服务器返回了无效的过滤器 UUID",
                        "The server returned an invalid filter UUID"
                    )
                    .to_string(),
                );
                cx.notify();
                return;
            };
            filters.push(local);
            filters.len() - 1
        };
        filters[local].apply_snapshot(FilterSnapshot {
            name: revision.name,
            value: revision.value,
            use_regex: revision.use_regex,
            note: revision.note,
            collaborative: revision.collaborative,
        });
        self.replace_and_save_filters(filters, window, cx);
        self.cloud_revision = None;
        self.cloud_message = Some(
            crate::tr!(
                "已把所选历史内容恢复到本地；检查后可显式提交",
                "Restored the selected history locally. Review it before submitting explicitly"
            )
            .to_string(),
        );
        cx.notify();
    }

    fn fork_cloud_detail_to_local(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(connection) = self.cloud_connection.as_ref() else {
            return;
        };
        let Some(detail) = self.cloud_detail.as_ref() else {
            return;
        };
        let Ok(mut filters) = self.filters(cx) else {
            return;
        };
        let source = if let Some(local) = find_local_filter_by_cloud_id(&filters, &detail.id) {
            local.clone()
        } else {
            let Ok(local) =
                create_local_filter_from_cloud(&filters, &connection.server_url, detail)
            else {
                self.cloud_message = Some(
                    crate::tr!(
                        "服务器返回了无效的过滤器 UUID",
                        "The server returned an invalid filter UUID"
                    )
                    .to_string(),
                );
                cx.notify();
                return;
            };
            local
        };
        let fork = fork_local_filter(&source);
        let fork_id = fork.id;
        filters.push(fork);
        self.replace_and_save_filters(filters, window, cx);
        self.cloud_detail = None;
        self.cloud_revision = None;
        window.close_dialog(cx);
        self.open_local_detail(fork_id, window, cx);
        self.cloud_message = Some(
            crate::tr!(
                "已另存为新的本地过滤器，并保留派生来源",
                "Saved as a new local filter and preserved its source"
            )
            .to_string(),
        );
        cx.notify();
    }

    fn confirm_delete_cloud_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(detail) = self
            .cloud_detail
            .as_ref()
            .filter(|detail| detail.can_delete)
        else {
            return;
        };
        let filter_id = detail.id.clone();
        let filter_name = detail.name.clone();
        self.cloud_detail_delete_focus.focus(window, cx);
        let dialog = cx.entity();
        window.open_alert_dialog(cx, move |alert, _, cx| {
            let dialog = dialog.clone();
            let filter_id = filter_id.clone();
            alert
                .icon(Icon::new(IconName::Info).text_color(cx.theme().danger))
                .title(crate::tr!("删除云端分享？", "Delete cloud share?"))
                .description(crate::tr_args!(
                    "确定删除自己分享的“{filter_name}”吗？删除后其他用户将无法再查找或下载。",
                    "Delete your shared filter “{filter_name}”? Other users will no longer be able to find or download it."
                ))
                .button_props(
                    DialogButtonProps::default()
                        .ok_variant(ButtonVariant::Danger)
                        .ok_text(crate::tr!("删除分享", "Delete share"))
                        .cancel_text(crate::tr!("取消", "Cancel"))
                        .show_cancel(true),
                )
                .on_ok(move |_, window, cx| {
                    dialog.update(cx, |this, cx| {
                        this.delete_cloud_filter(filter_id.clone(), window, cx)
                    });
                    true
                })
        });
    }

    fn delete_cloud_filter(
        &mut self,
        filter_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.cloud_task.is_some() {
            return;
        }
        let Some(client) = self.cloud_client.clone() else {
            return;
        };
        let server_url = self
            .cloud_connection
            .as_ref()
            .map(|connection| connection.server_url.clone())
            .unwrap_or_default();
        self.cloud_task = Some(cx.spawn_in(window, async move |this, cx| {
            let request_filter_id = filter_id.clone();
            let result = cx
                .background_spawn(async move { client.delete_filter(&request_filter_id) })
                .await;
            _ = this.update_in(cx, |this, window, cx| {
                this.cloud_task = None;
                match result {
                    Ok(()) => {
                        let detail_is_open = this
                            .cloud_detail
                            .as_ref()
                            .is_some_and(|detail| detail.id == filter_id);
                        let deleted_index = this
                            .cloud_items
                            .iter()
                            .position(|item| item.id == filter_id);
                        let fallback_id = deleted_index.and_then(|index| {
                            if index + 1 < this.cloud_items.len() {
                                Some(this.cloud_items[index + 1].id.clone())
                            } else {
                                index
                                    .checked_sub(1)
                                    .map(|index| this.cloud_items[index].id.clone())
                            }
                        });
                        let focus = std::mem::replace(
                            &mut this.cloud_detail_delete_focus,
                            cx.focus_handle(),
                        );
                        if let Some(fallback_id) = fallback_id {
                            this.cloud_row_focus.insert(fallback_id, focus);
                        } else {
                            this.cloud_table_focus = focus;
                        }
                        this.cloud_items.retain(|item| item.id != filter_id);
                        this.cloud_selected.remove(&filter_id);
                        this.cloud_row_focus.remove(&filter_id);
                        this.cloud_total = this.cloud_total.saturating_sub(1);
                        this.cloud_detail = None;
                        this.secondary_route = None;
                        this.cloud_revision = None;
                        this.cloud_revisions.clear();
                        this.cloud_revision_page = 1;
                        this.cloud_revision_total = 0;
                        let retained_local_branch = this.rows.iter().any(|row| {
                            remote_deleted_status(&row.filter, &server_url, &filter_id)
                                == Some(CloudFilterLocalStatus::RemoteDeleted)
                        });
                        for row in &mut this.rows {
                            row.filter =
                                detach_cloud_reference(&row.filter, &server_url, &filter_id);
                        }
                        if let Ok(filters) = this.filters(cx) {
                            cx.emit(PredefinedFiltersDialogEvent::Filters(filters));
                        }
                        if this.cloud_items.is_empty() && this.cloud_page > 1 {
                            this.load_cloud_page(this.cloud_page - 1, window, cx);
                        }
                        this.cloud_message = Some(if retained_local_branch {
                            crate::tr!(
                                "远端已删除；本地过滤器已保留并解除云端引用",
                                "Deleted remotely; the local filter was kept and unlinked from the cloud"
                            )
                            .to_string()
                        } else {
                            crate::tr!("云端分享已删除", "Cloud share deleted").to_string()
                        });
                        if detail_is_open {
                            window.close_dialog(cx);
                        }
                    }
                    Err(error) => {
                        this.cloud_message = Some(crate::tr_args!(
                            "删除云端分享失败：{error}",
                            "Couldn’t delete the cloud share: {error}"
                        ))
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn disconnect_cloud(&mut self, cx: &mut Context<Self>) {
        if self.cloud_task.is_some() {
            return;
        }
        let Some(client) = self.cloud_client.clone() else {
            return;
        };
        self.cloud_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { client.disconnect() })
                .await;
            _ = this.update(cx, |this, cx| {
                this.cloud_task = None;
                match result {
                    Ok(()) => {
                        this.cloud_connection = None;
                        this.cloud_items.clear();
                        this.cloud_selected.clear();
                        this.cloud_row_focus.clear();
                        this.cloud_detail = None;
                        this.cloud_revision = None;
                        this.cloud_revisions.clear();
                        this.cloud_revision_page = 1;
                        this.cloud_revision_total = 0;
                        this.cloud_message = Some(
                            crate::tr!(
                                "已关闭本次云端会话；系统凭据仍保留",
                                "Closed this cloud session; system credentials were retained"
                            )
                            .to_string(),
                        );
                        cx.emit(PredefinedFiltersDialogEvent::CloudConnection(None));
                    }
                    Err(error) => {
                        this.cloud_message = Some(crate::tr_args!(
                            "关闭云端会话失败：{error}",
                            "Couldn’t close the cloud session: {error}"
                        ))
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn render_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        TabBar::new("predefined-filter-tabs")
            .w_full()
            .flex_none()
            .large()
            .underline()
            .selected_index(match self.active_tab {
                DialogTab::Local => 0,
                DialogTab::Cloud => 1,
            })
            .px_4()
            .border_b_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .child(Tab::new().label(crate::tr!("本地过滤器", "Local filters")))
            .child(Tab::new().label(crate::tr!("云端过滤器", "Cloud filters")))
            .on_click(cx.listener(|this, index: &usize, window, cx| {
                this.active_tab = if *index == 0 {
                    DialogTab::Local
                } else {
                    DialogTab::Cloud
                };
                if this.active_tab == DialogTab::Cloud
                    && this.cloud_connection.is_some()
                    && this.cloud_items.is_empty()
                {
                    this.load_cloud_page(1, window, cx);
                } else {
                    cx.notify();
                }
            }))
    }

    fn render_type_badge(&self, use_regex: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let (background, border, foreground, label) = if use_regex {
            (
                cx.theme().primary.opacity(0.08),
                cx.theme().primary.opacity(0.35),
                cx.theme().primary,
                crate::tr!("正则", "Regex"),
            )
        } else {
            (
                cx.theme().muted.opacity(0.35),
                cx.theme().border,
                cx.theme().muted_foreground,
                crate::tr!("文本", "Text"),
            )
        };
        div()
            .flex_none()
            .px_2()
            .py_1()
            .rounded_full()
            .border_1()
            .border_color(border)
            .bg(background)
            .text_xs()
            .text_color(foreground)
            .child(label)
    }

    fn render_local_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let busy = self.io_task.is_some();
        let online = self.cloud_connection.is_some() && !self.cloud_offline;
        let protocol_supported = self
            .cloud_connection
            .as_ref()
            .is_some_and(CloudConnectionProfile::supports_uuid_filter_branches);
        let server_url = self
            .cloud_connection
            .as_ref()
            .map(|connection| connection.server_url.as_str())
            .unwrap_or_default();
        let resolved_filters = self.filters(cx);
        let drafts_valid = resolved_filters.is_ok();
        let selected_count = self.local_bulk_selected.len();
        let shareable_selected_count = resolved_filters.as_ref().map_or(0, |filters| {
            filters
                .iter()
                .filter(|filter| {
                    self.local_bulk_selected.contains(&filter.id)
                        && Self::local_filter_has_publish_changes(filter, server_url)
                })
                .count()
        });
        let selected_ids = self.local_bulk_selected.clone();
        h_flex()
            .id("predefined-filter-local-toolbar")
            .w_full()
            .flex_none()
            .justify_between()
            .gap_3()
            .px_3()
            .py_3()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        outline_icon_button(
                            "predefined-filter-add",
                            IconName::Plus,
                            crate::tr!("新增过滤器", "Add filter"),
                            busy,
                            None,
                            cx,
                        )
                        .on_click(cx.listener(|this, _, window, cx| this.add_filter(window, cx))),
                    )
                    .child(
                        Button::new("predefined-filter-share")
                            .small()
                            .outline()
                            .w_20()
                            .icon(IconName::ExternalLink)
                            .label(crate::tr!("分享", "Share"))
                            .disabled(
                                busy || !online
                                    || !protocol_supported
                                    || !drafts_valid
                                    || shareable_selected_count == 0,
                            )
                            .tooltip(if !online {
                                crate::tr!("连接云端后可分享", "Connect to the cloud to share")
                            } else if !protocol_supported {
                                crate::tr!(
                                    "当前服务器需要升级后才能分享过滤器",
                                    "Upgrade the current server before sharing filters"
                                )
                            } else if !drafts_valid {
                                crate::tr!(
                                    "请先完成并修正本地过滤器草稿",
                                    "Complete and fix local filter drafts first"
                                )
                            } else if selected_count == 0 {
                                crate::tr!(
                                    "请先勾选要分享的本地过滤器",
                                    "Select local filters to share first"
                                )
                            } else if shareable_selected_count > 0 {
                                crate::tr!(
                                    "分享所选且尚未发布到当前服务器的过滤器",
                                    "Share selected filters that aren’t published to this server"
                                )
                            } else {
                                crate::tr!(
                                    "所选过滤器均已发布到当前服务器",
                                    "All selected filters are already published to this server"
                                )
                            })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.show_cloud_share(Some(selected_ids.clone()), window, cx)
                            })),
                    )
                    .child(
                        Button::new("predefined-filter-delete-selected")
                            .small()
                            .danger()
                            .w_20()
                            .icon(IconName::Delete)
                            .label(crate::tr!("删除", "Delete"))
                            .disabled(busy || selected_count == 0)
                            .tooltip(if selected_count == 0 {
                                crate::tr!(
                                    "请先勾选要删除的本地过滤器",
                                    "Select local filters to delete first"
                                )
                            } else {
                                crate::tr!("删除所选本地过滤器", "Delete selected local filters")
                            })
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.confirm_remove_selected_filters(window, cx)
                            })),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("predefined-filter-import")
                            .large()
                            .outline()
                            .w_24()
                            .label(crate::tr!("导入", "Import"))
                            .loading(busy)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.import_filters(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("predefined-filter-export")
                            .large()
                            .outline()
                            .w_24()
                            .label(crate::tr!("导出", "Export"))
                            .disabled(busy || self.rows.is_empty() || !drafts_valid)
                            .tooltip(if !drafts_valid {
                                crate::tr!(
                                    "请先完成并修正本地过滤器草稿",
                                    "Complete and fix local filter drafts first"
                                )
                            } else {
                                crate::tr!("导出本地过滤器", "Export local filters")
                            })
                            .on_click(
                                cx.listener(|this, _, window, cx| this.export_filters(window, cx)),
                            ),
                    ),
            )
    }

    fn render_local_detail_page(
        &self,
        row: &FilterDraft,
        index: usize,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let row_id = row.filter.id;
        let copy_id = row.filter.id.to_string();
        let regex_id = row.filter.id;
        let collaborative_id = row.filter.id;
        let up_id = row.filter.id;
        let down_id = row.filter.id;
        let busy = self.io_task.is_some();
        let source = row
            .filter
            .remote_references
            .iter()
            .find(|reference| reference.relation == RemoteFilterRelation::DerivedFrom)
            .map(|source| {
                let owner = if source.owner_name.is_empty() {
                    source.owner_id.as_str()
                } else {
                    source.owner_name.as_str()
                };
                crate::tr_args!("云端下载 · {owner}", "Cloud download · {owner}")
            })
            .unwrap_or_else(|| crate::tr!("本地创建", "Created locally").to_string());
        let publish_state = row
            .filter
            .remote_references
            .iter()
            .find(|reference| reference.relation == RemoteFilterRelation::Tracking)
            .map(|published| {
                crate::tr_args!(
                    "已分享 · 修订 {}",
                    "Shared · revision {}",
                    published.revision
                )
            })
            .unwrap_or_else(|| crate::tr!("尚未分享", "Not shared").to_string());

        v_flex()
            .id(format!("predefined-filter-local-detail-{row_id}"))
            .size_full()
            .min_h_0()
            .overflow_hidden()
            .bg(cx.theme().background)
            .child(
                TabBar::new(format!("predefined-filter-detail-tabs-{row_id}"))
                    .w_full()
                    .flex_none()
                    .large()
                    .underline()
                    .selected_index(0)
                    .px_6()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(Tab::new().label(crate::tr!("详情", "Details"))),
            )
            .child(
                v_flex()
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .child(
                        h_flex()
                            .w_full()
                            .items_start()
                            .justify_center()
                            .px_8()
                            .py_6()
                            .child(
                                v_flex()
                                    .w_full()
                                    .max_w(rems(68.))
                                    .gap_6()
                                    .child(
                                        v_flex()
                                            .gap_2()
                                            .child(
                                                h_flex()
                                                    .justify_between()
                                                    .gap_4()
                                                    .child(div().font_semibold().child(crate::tr!("名称", "Name")))
                                                    .child(
                                                        h_flex()
                                                            .gap_2()
                                                            .text_sm()
                                                            .text_color(
                                                                cx.theme().muted_foreground,
                                                            )
                                                            .child(
                                                                Switch::new(format!(
                                                                    "predefined-filter-collaborative-{collaborative_id}"
                                                                ))
                                                                .small()
                                                                .checked(row.filter.collaborative)
                                                                .tooltip(crate::tr!("允许其他用户共同编辑云端版本", "Allow other users to edit the cloud version"))
                                                                .on_click(cx.listener(
                                                                    move |this,
                                                                          checked: &bool,
                                                                          _,
                                                                          cx| {
                                                                        if let Some(row) = this
                                                                            .rows
                                                                            .iter_mut()
                                                                            .find(|row| {
                                                                                row.filter.id
                                                                                    == collaborative_id
                                                                            })
                                                                        {
                                                                            row.filter.collaborative =
                                                                                *checked;
                                                                            cx.notify();
                                                                        }
                                                                    },
                                                                )),
                                                            )
                                                            .child(crate::tr!("允许共创", "Allow collaboration")),
                                                    ),
                                            )
                                            .child(Input::new(&row.name).large()),
                                    )
                                    .child(
                                        v_flex()
                                            .gap_2()
                                            .child(
                                                h_flex()
                                                    .justify_between()
                                                    .gap_4()
                                                    .child(div().font_semibold().child(crate::tr!("匹配的值", "Match value")))
                                                    .child(
                                                        h_flex()
                                                            .gap_2()
                                                            .text_sm()
                                                            .text_color(
                                                                cx.theme().muted_foreground,
                                                            )
                                                            .child(
                                                                Switch::new(format!(
                                                                    "predefined-filter-regex-{regex_id}"
                                                                ))
                                                                .small()
                                                                .checked(row.filter.use_regex)
                                                                .tooltip(crate::tr!("使用正则表达式", "Use regular expressions"))
                                                                .on_click(cx.listener(
                                                                    move |this,
                                                                          checked: &bool,
                                                                          _,
                                                                          cx| {
                                                                        if let Some(row) = this
                                                                            .rows
                                                                            .iter_mut()
                                                                            .find(|row| {
                                                                                row.filter.id
                                                                                    == regex_id
                                                                            })
                                                                        {
                                                                            row.filter.use_regex =
                                                                                *checked;
                                                                            cx.notify();
                                                                        }
                                                                    },
                                                                )),
                                                            )
                                                            .child(crate::tr!("正则", "Regex")),
                                                    ),
                                            )
                                            .child(Input::new(&row.value).large()),
                                    )
                                    .child(
                                        v_flex()
                                            .gap_2()
                                            .child(div().font_semibold().child(crate::tr!("备注", "Note")))
                                            .child(Input::new(&row.note).large()),
                                    )
                                    .child(
                                        v_flex()
                                            .gap_2()
                                            .child(div().font_semibold().child("UUID"))
                                            .child(
                                                h_flex()
                                                    .gap_2()
                                                    .child(
                                                        div()
                                                            .flex_1()
                                                            .min_w_0()
                                                            .truncate()
                                                            .font_family(
                                                                cx.theme().mono_font_family.clone(),
                                                            )
                                                            .child(copy_id.clone()),
                                                    )
                                                    .child(
                                                        Button::new(format!(
                                                            "predefined-filter-copy-uuid-{row_id}"
                                                        ))
                                                        .small()
                                                        .outline()
                                                        .label(crate::tr!("复制", "Duplicate"))
                                                        .on_click(cx.listener({
                                                            let copy_id = copy_id.clone();
                                                            move |_, _, _, cx| {
                                                                cx.write_to_clipboard(
                                                                    ClipboardItem::new_string(
                                                                        copy_id.clone(),
                                                                    ),
                                                                )
                                                            }
                                                        })),
                                                    ),
                                            ),
                                    )
                                    .child(
                                        h_flex()
                                            .w_full()
                                            .gap_6()
                                            .p_4()
                                            .rounded(cx.theme().radius_lg)
                                            .border_1()
                                            .border_color(cx.theme().border)
                                            .bg(cx.theme().group_box)
                                            .child(
                                                v_flex()
                                                    .flex_1()
                                                    .gap_1()
                                                    .child(
                                                        div()
                                                            .text_xs()
                                                            .text_color(
                                                                cx.theme().muted_foreground,
                                                            )
                                                            .child(crate::tr!("来源", "Source")),
                                                    )
                                                    .child(div().text_sm().child(source)),
                                            )
                                            .child(
                                                v_flex()
                                                    .flex_1()
                                                    .gap_1()
                                                    .child(
                                                        div()
                                                            .text_xs()
                                                            .text_color(
                                                                cx.theme().muted_foreground,
                                                            )
                                                            .child(crate::tr!("云端状态", "Cloud status")),
                                                    )
                                                    .child(div().text_sm().child(publish_state)),
                                            ),
                                    ),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .w_full()
                    .flex_none()
                    .justify_between()
                    .gap_3()
                    .px_6()
                    .py_3()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new(format!("predefined-filter-detail-up-{up_id}"))
                                    .small()
                                    .outline()
                                    .icon(IconName::ArrowUp)
                                    .label(crate::tr!("上移", "Move up"))
                                    .disabled(index == 0 || busy)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                    this.move_filter(up_id, -1, cx)
                                    })),
                            )
                            .child(
                                Button::new(format!("predefined-filter-detail-down-{down_id}"))
                                    .small()
                                    .outline()
                                    .icon(IconName::ArrowDown)
                                    .label(crate::tr!("下移", "Move down"))
                                    .disabled(index + 1 == self.rows.len() || busy)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                    this.move_filter(down_id, 1, cx)
                                    })),
                            ),
                    )
                    .child(
                        Button::new(format!("predefined-filter-detail-finish-{row_id}"))
                            .large()
                            .primary()
                            .w_24()
                            .label(crate::tr!("完成", "Done"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.close_local_detail(window, cx)
                            })),
                    ),
            )
    }

    fn render_local_row(
        &self,
        row: &FilterDraft,
        drafts_valid: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let active = self.local_selected == Some(row.filter.id);
        let checked = self.local_bulk_selected.contains(&row.filter.id);
        let select_id = row.filter.id;
        let edit_id = row.filter.id;
        let share_id = row.filter.id;
        let remove_id = row.filter.id;
        let remove_name = row.name.read(cx).value().trim().to_string();
        let display_name = if remove_name.is_empty() {
            crate::tr!("未命名过滤器", "Unnamed filter").to_string()
        } else {
            remove_name.clone()
        };
        let confirm_name = display_name.clone();
        let display_value = row.value.read(cx).value().trim().to_string();
        let display_value = if display_value.is_empty() {
            crate::tr!("尚未填写匹配值", "No match value entered").to_string()
        } else {
            display_value
        };
        let online = self.cloud_connection.is_some() && !self.cloud_offline;
        let protocol_supported = self
            .cloud_connection
            .as_ref()
            .is_some_and(CloudConnectionProfile::supports_uuid_filter_branches);
        let server_url = self
            .cloud_connection
            .as_ref()
            .map(|connection| connection.server_url.as_str())
            .unwrap_or_default();
        let mut draft_filter = row.filter.clone();
        draft_filter.name = row.name.read(cx).value().trim().to_string();
        draft_filter.value = row.value.read(cx).value().trim().to_string();
        draft_filter.note = row.note.read(cx).value().trim().to_string();
        let can_share = online
            && protocol_supported
            && drafts_valid
            && !draft_filter.name.is_empty()
            && !draft_filter.value.is_empty()
            && Self::local_filter_has_publish_changes(&draft_filter, server_url);
        let share_accessibility_label = if can_share {
            crate::tr_args!(
                "分享或同步过滤器：{display_name}",
                "Share or synchronize filter: {display_name}"
            )
        } else if online && !protocol_supported {
            crate::tr_args!(
                "无法分享过滤器“{display_name}”：服务器需要升级",
                "Can’t share filter “{display_name}”: the server must be upgraded"
            )
        } else if online && !drafts_valid {
            crate::tr_args!(
                "无法分享过滤器“{display_name}”：请先修正本地过滤器草稿",
                "Can’t share filter “{display_name}”: fix local filter drafts first"
            )
        } else if online {
            crate::tr_args!(
                "过滤器“{display_name}”的云端版本已是最新",
                "The cloud version of filter “{display_name}” is up to date"
            )
        } else {
            crate::tr_args!(
                "无法分享过滤器“{display_name}”：尚未连接云端",
                "Can’t share filter “{display_name}”: not connected to the cloud"
            )
        };
        let dialog = cx.entity();

        v_flex()
            .id(format!("predefined-filter-row-group-{}", row.filter.id))
            .w_full()
            .flex_none()
            .child(
                h_flex()
                    .id(format!("predefined-filter-row-{}", row.filter.id))
                    .relative()
                    .w_full()
                    .h(rems(FILTER_ROW_HEIGHT_REMS))
                    .flex_none()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .when(active || checked, |row| row.bg(cx.theme().list_active))
                    .when(!active && !checked, |row| {
                        row.hover(|row| row.bg(cx.theme().tokens.list_hover))
                    })
                    .when(active, |row| {
                        row.child(
                            div()
                                .absolute()
                                .left_0()
                                .top_0()
                                .bottom_0()
                                .w_1()
                                .bg(cx.theme().primary),
                        )
                    })
                    .child(
                        div()
                            .w(rems(FILTER_INDEX_WIDTH_REMS))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                Checkbox::new(format!(
                                    "predefined-filter-select-{}",
                                    row.filter.id
                                ))
                                .checked(checked)
                                .aria_label(crate::tr_args!(
                                    "选择本地过滤器：{display_name}",
                                    "Select local filter: {display_name}"
                                ))
                                .on_click(cx.listener(
                                    move |this, checked: &bool, _, cx| {
                                        if *checked {
                                            this.local_bulk_selected.insert(select_id);
                                        } else {
                                            this.local_bulk_selected.remove(&select_id);
                                        }
                                        cx.notify();
                                    },
                                )),
                            ),
                    )
                    .child(
                        BaseButton::new(format!("predefined-filter-edit-{edit_id}"))
                            .track_focus(&row.focus)
                            .accessibility_label(crate::tr_args!(
                                "打开本地过滤器详情：{display_name}",
                                "Open local filter details: {display_name}"
                            ))
                            .h_full()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .justify_start()
                            .hover(|button| button.bg(cx.theme().tokens.list_hover))
                            .focus_visible(|style| style.border_1().border_color(cx.theme().ring))
                            .child(
                                h_flex()
                                    .size_full()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .w(rems(LOCAL_NAME_WIDTH_REMS))
                                            .flex_none()
                                            .px_3()
                                            .truncate()
                                            .text_sm()
                                            .font_semibold()
                                            .child(display_name),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .px_3()
                                            .truncate()
                                            .text_xs()
                                            .font_family(cx.theme().mono_font_family.clone())
                                            .text_color(cx.theme().muted_foreground)
                                            .child(display_value),
                                    )
                                    .child(
                                        div()
                                            .w(rems(FILTER_TYPE_WIDTH_REMS))
                                            .flex_none()
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .child(
                                                self.render_type_badge(row.filter.use_regex, cx),
                                            ),
                                    ),
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_local_detail(edit_id, window, cx)
                            })),
                    )
                    .child(
                        h_flex()
                            .w(rems(LOCAL_ACTIONS_WIDTH_REMS))
                            .flex_none()
                            .justify_end()
                            .gap_1()
                            .pr_2()
                            .child(
                                outline_icon_button(
                                    format!("predefined-filter-row-share-{share_id}"),
                                    IconName::ExternalLink,
                                    share_accessibility_label,
                                    !can_share,
                                    None,
                                    cx,
                                )
                                .on_click(cx.listener(
                                    move |this, _, window, cx| {
                                        this.show_cloud_share(
                                            Some(BTreeSet::from([share_id])),
                                            window,
                                            cx,
                                        )
                                    },
                                )),
                            )
                            .child(
                                outline_icon_button(
                                    format!("predefined-filter-remove-{remove_id}"),
                                    IconName::Delete,
                                    crate::tr_args!(
                                        "删除本地过滤器：{confirm_name}",
                                        "Delete local filter: {confirm_name}"
                                    ),
                                    self.io_task.is_some(),
                                    Some(&row.delete_focus),
                                    cx,
                                )
                                .on_click(move |_, window, cx| {
                                    dialog.update(cx, |this, cx| {
                                        this.confirm_remove_filter(
                                            remove_id,
                                            confirm_name.clone(),
                                            window,
                                            cx,
                                        );
                                    });
                                }),
                            ),
                    ),
            )
    }

    fn render_local_table(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let drafts_valid = self.filters(cx).is_ok();
        let all_selected = !self.rows.is_empty()
            && self
                .rows
                .iter()
                .all(|row| self.local_bulk_selected.contains(&row.filter.id));
        let all_selected_click = all_selected;
        v_flex()
            .id("predefined-filter-local-table")
            .w_full()
            .flex_1()
            .min_h_0()
            .rounded(cx.theme().radius_lg)
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().group_box)
            .overflow_hidden()
            .child(
                h_flex()
                    .w_full()
                    .h(rems(FILTER_HEADER_HEIGHT_REMS))
                    .flex_none()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().muted.opacity(0.28))
                    .text_xs()
                    .font_semibold()
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        div()
                            .w(rems(FILTER_INDEX_WIDTH_REMS))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                Checkbox::new("predefined-filter-select-all")
                                    .checked(all_selected)
                                    .disabled(self.rows.is_empty())
                                    .aria_label(crate::tr!(
                                        "选择全部本地过滤器",
                                        "Select all local filters"
                                    ))
                                    .tooltip(crate::tr!(
                                        "全选本地过滤器",
                                        "Select all local filters"
                                    ))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.select_all_local_filters(!all_selected_click, cx)
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .w(rems(LOCAL_NAME_WIDTH_REMS))
                            .flex_none()
                            .px_3()
                            .child(crate::tr!("名称", "Name")),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .px_3()
                            .child(crate::tr!("匹配的值", "Match value")),
                    )
                    .child(
                        div()
                            .w(rems(FILTER_TYPE_WIDTH_REMS))
                            .flex_none()
                            .text_center()
                            .child(crate::tr!("类型", "Type")),
                    )
                    .child(
                        div()
                            .w(rems(LOCAL_ACTIONS_WIDTH_REMS))
                            .flex_none()
                            .text_center()
                            .child(crate::tr!("操作", "Actions")),
                    )
                    .child(div().w(Scrollbar::width()).flex_none()),
            )
            .child(
                h_flex()
                    .w_full()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .overflow_hidden()
                    .child(
                        v_flex()
                            .id("predefined-filter-local-table-viewport")
                            .h_full()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .track_focus(&self.local_table_focus)
                            .aria_label(crate::tr!("本地过滤器表格", "Local filters table"))
                            .when(self.local_table_focus.is_focused(window), |list| {
                                list.focus_ring_style(window, cx)
                            })
                            .overflow_y_scroll()
                            .track_scroll(&self.local_scroll)
                            .when(self.rows.is_empty(), |list| {
                                list.items_center().justify_center().child(
                                    v_flex()
                                        .items_center()
                                        .gap_2()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(crate::tr!(
                                            "尚未配置过滤器",
                                            "No filters configured"
                                        ))
                                        .child(div().text_sm().child(crate::tr!(
                                            "点击左上角的“+”创建第一个过滤器",
                                            "Select + in the upper-left to create your first filter"
                                        ))),
                                )
                            })
                            .children(self.rows.iter().map(|row| {
                                self.render_local_row(row, drafts_valid, cx)
                                    .into_any_element()
                            })),
                    )
                    .child(filter_table_scrollbar(
                        "predefined-filter-local-table-scrollbar",
                        &self.local_scroll,
                    )),
            )
    }

    fn render_local_panel(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .id("predefined-filter-local-panel")
            .size_full()
            .min_h_0()
            .overflow_hidden()
            .child(self.render_local_toolbar(cx))
            .child(
                v_flex()
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .p_4()
                    .overflow_hidden()
                    .child(self.render_local_table(window, cx)),
            )
    }

    fn render_cloud_connection_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let busy = self.cloud_task.is_some();
        v_flex()
            .id("cloud-filter-connection-panel")
            .w_full()
            .max_w(rems(36.))
            .gap_4()
            .p_5()
            .rounded(cx.theme().radius_lg)
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().group_box)
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_lg()
                            .font_semibold()
                            .child(crate::tr!("连接云端过滤器", "Connect cloud filters")),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(crate::tr!(
                                "连接后可搜索、分享、下载并同步团队过滤器。",
                                "Connect to search, share, download, and synchronize team filters."
                            )),
                    ),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(crate::tr!("服务器地址", "Server address")),
                    )
                    .child(Input::new(&self.server_url)),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(crate::tr!("工号或昵称", "Employee ID or nickname")),
                    )
                    .child(Input::new(&self.display_name)),
            )
            .child(
                h_flex().justify_end().child(
                    Button::new("cloud-filter-connect")
                        .large()
                        .primary()
                        .label(crate::tr!("连接", "Connect"))
                        .loading(busy)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.connect_cloud(window, cx)),
                        ),
                ),
            )
    }

    fn render_cloud_toolbar(
        &self,
        selected_ids: Vec<String>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let busy = self.cloud_task.is_some();
        let dialog = cx.entity();
        let identity_dialog = dialog.clone();
        let sort_dialog = dialog.clone();
        let identity = self
            .cloud_connection
            .as_ref()
            .map(|connection| connection.display_name.clone())
            .unwrap_or_default();
        let identity_label = if self.cloud_offline {
            crate::tr_args!("离线 · {identity}", "Offline · {identity}")
        } else {
            identity
        };
        let connection_status = self
            .cloud_connection
            .as_ref()
            .map(|connection| {
                if self.cloud_offline {
                    crate::tr!(
                        "只读缓存 · 不含账户凭据",
                        "Read-only cache · no account credentials"
                    )
                } else if connection.insecure {
                    crate::tr!("HTTP · 未加密连接", "HTTP · unencrypted connection")
                } else {
                    crate::tr!(
                        "HTTPS · 系统凭据库会话",
                        "HTTPS · system credential session"
                    )
                }
            })
            .unwrap_or_default();
        let connection_status_menu = connection_status.to_string();
        let identity_tooltip = self
            .cloud_connection
            .as_ref()
            .map(|connection| {
                format!(
                    "{}\n{}\n{}",
                    identity_label, connection_status, connection.server_url
                )
            })
            .unwrap_or_else(|| identity_label.clone());
        let selected_count = selected_ids.len();
        let cloud_sort = self.cloud_sort;

        h_flex()
            .id("cloud-filter-toolbar")
            .w_full()
            .flex_none()
            .gap_2()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                Button::new("cloud-filter-identity")
                    .small()
                    .ghost()
                    .max_w(rems(7.33))
                    .justify_start()
                    .px_0()
                    .text_xs()
                    .dropdown_caret(false)
                    .text_color(cx.theme().primary)
                    .label(identity_label)
                    .tooltip(identity_tooltip)
                    .disabled(busy)
                    .dropdown_menu(move |menu, window, _| {
                        let reconnect_dialog = identity_dialog.clone();
                        let disconnect_dialog = identity_dialog.clone();
                        let reconnect = window
                            .listener_for(&reconnect_dialog, |this, _, window, cx| {
                                this.connect_cloud(window, cx)
                            });
                        let disconnect = window
                            .listener_for(&disconnect_dialog, |this, _, _, cx| {
                                this.disconnect_cloud(cx)
                            });
                        menu.item(PopupMenuItem::new(connection_status_menu.clone()).disabled(true))
                            .item(
                                PopupMenuItem::new(crate::tr!("重新连接", "Reconnect"))
                                    .on_click(reconnect),
                            )
                            .item(
                                PopupMenuItem::new(crate::tr!(
                                    "断开本次会话",
                                    "Disconnect this session"
                                ))
                                .on_click(disconnect),
                            )
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(Input::new(&self.cloud_query)),
            )
            .child(
                Button::new("cloud-filter-sort")
                    .large()
                    .outline()
                    .w(rems(6.25))
                    .dropdown_caret(true)
                    .label(self.cloud_sort.label())
                    .disabled(busy)
                    .dropdown_menu(move |menu, window, _| {
                        let newest_dialog = sort_dialog.clone();
                        let downloads_dialog = sort_dialog.clone();
                        let likes_dialog = sort_dialog.clone();
                        let newest = window.listener_for(&newest_dialog, |this, _, window, cx| {
                            this.set_cloud_sort(CloudSort::Newest, window, cx)
                        });
                        let downloads =
                            window.listener_for(&downloads_dialog, |this, _, window, cx| {
                                this.set_cloud_sort(CloudSort::Downloads, window, cx)
                            });
                        let likes = window.listener_for(&likes_dialog, |this, _, window, cx| {
                            this.set_cloud_sort(CloudSort::Likes, window, cx)
                        });
                        menu.item(
                            PopupMenuItem::new(crate::tr!("最新", "Newest"))
                                .checked(cloud_sort == CloudSort::Newest)
                                .on_click(newest),
                        )
                        .item(
                            PopupMenuItem::new(crate::tr!("下载最多", "Most downloaded"))
                                .checked(cloud_sort == CloudSort::Downloads)
                                .on_click(downloads),
                        )
                        .item(
                            PopupMenuItem::new(crate::tr!("点赞最多", "Most liked"))
                                .checked(cloud_sort == CloudSort::Likes)
                                .on_click(likes),
                        )
                    }),
            )
            .child(
                Button::new("cloud-filter-search")
                    .large()
                    .outline()
                    .w_24()
                    .label(crate::tr!("搜索", "Search"))
                    .loading(busy)
                    .on_click(
                        cx.listener(|this, _, window, cx| this.load_cloud_page(1, window, cx)),
                    ),
            )
            .child(
                Button::new("cloud-filter-download-selected")
                    .large()
                    .outline()
                    .w(rems(8.))
                    .label(crate::tr!("下载/更新所选", "Download/update selected"))
                    .tooltip(crate::tr_args!(
                        "已选择 {selected_count} 项",
                        "{selected_count} selected"
                    ))
                    .disabled(busy || selected_ids.is_empty())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.download_cloud_filters(selected_ids.clone(), window, cx)
                    })),
            )
    }

    fn render_cloud_directory_table(
        &self,
        local_filters: &[PredefinedFilter],
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let busy = self.cloud_task.is_some();
        let server_url = self
            .cloud_connection
            .as_ref()
            .map(|connection| connection.server_url.as_str())
            .unwrap_or_default();
        let all_selected = !self.cloud_items.is_empty()
            && self
                .cloud_items
                .iter()
                .all(|item| self.cloud_selected.contains(&item.id));
        let all_selected_click = all_selected;

        v_flex()
            .id("cloud-filter-table")
            .w_full()
            .flex_1()
            .min_h_0()
            .rounded(cx.theme().radius_lg)
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().group_box)
            .overflow_hidden()
            .child(
                h_flex()
                    .w_full()
                    .h(rems(FILTER_HEADER_HEIGHT_REMS))
                    .flex_none()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().muted.opacity(0.28))
                    .text_xs()
                    .font_semibold()
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        div()
                            .w(rems(FILTER_INDEX_WIDTH_REMS))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                Checkbox::new("cloud-filter-select-page")
                                    .checked(all_selected)
                                    .disabled(self.cloud_items.is_empty())
                                    .aria_label(crate::tr!(
                                        "选择当前页的云端过滤器",
                                        "Select cloud filters on this page"
                                    ))
                                    .tooltip(crate::tr!("选择本页", "Select page"))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.select_cloud_page(!all_selected_click, cx)
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .w(rems(CLOUD_NAME_WIDTH_REMS))
                            .flex_none()
                            .px_3()
                            .child(crate::tr!("名称", "Name")),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .px_3()
                            .child(crate::tr!("匹配的值", "Match value")),
                    )
                    .child(
                        div()
                            .w(rems(FILTER_TYPE_WIDTH_REMS))
                            .flex_none()
                            .text_center()
                            .child(crate::tr!("类型", "Type")),
                    )
                    .child(
                        div()
                            .w(rems(CLOUD_ACTIONS_WIDTH_REMS))
                            .flex_none()
                            .text_center()
                            .child(crate::tr!("操作", "Actions")),
                    )
                    .child(div().w(Scrollbar::width()).flex_none()),
            )
            .child(
                h_flex()
                    .w_full()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .overflow_hidden()
                    .child(
                        v_flex()
                            .id("cloud-filter-table-viewport")
                            .h_full()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .track_focus(&self.cloud_table_focus)
                            .aria_label(crate::tr!("云端过滤器表格", "Cloud filters table"))
                            .when(self.cloud_table_focus.is_focused(window), |list| {
                                list.focus_ring_style(window, cx)
                            })
                            .overflow_y_scroll()
                            .track_scroll(&self.cloud_scroll)
                            .when(self.cloud_items.is_empty(), |list| {
                                list.items_center().justify_center().child(
                                    div()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(if busy {
                                            crate::tr!(
                                                "正在读取云端过滤器…",
                                                "Reading cloud filters…"
                                            )
                                        } else {
                                            crate::tr!(
                                                "没有符合条件的云端过滤器",
                                                "No cloud filters match"
                                            )
                                        }),
                                )
                            })
                            .children(self.cloud_items.iter().map(|item| {
                                let selected = self.cloud_selected.contains(&item.id);
                                let select_id = item.id.clone();
                                let detail_id = item.id.clone();
                                let detail_focus = self.cloud_row_focus.get(&item.id).cloned();
                                let like_id = item.id.clone();
                                let action_id = item.id.clone();
                                let status =
                                    cloud_filter_local_status(local_filters, server_url, item);
                                let action_label = match status {
                                    CloudFilterLocalStatus::NotDownloaded => {
                                        crate::tr!("下载到本地", "Download locally")
                                    }
                                    CloudFilterLocalStatus::RemoteUpdated => {
                                        crate::tr!("更新本地", "Update local")
                                    }
                                    CloudFilterLocalStatus::AutoMerge => {
                                        crate::tr!("合并更新", "Merge update")
                                    }
                                    CloudFilterLocalStatus::Synced => {
                                        crate::tr!("已同步", "Synchronized")
                                    }
                                    CloudFilterLocalStatus::LocalModified => {
                                        crate::tr!("本地有修改", "Locally modified")
                                    }
                                    CloudFilterLocalStatus::Conflict if self.cloud_offline => {
                                        crate::tr!("稍后解决", "Resolve later")
                                    }
                                    CloudFilterLocalStatus::Conflict => {
                                        crate::tr!("解决冲突", "Resolve conflict")
                                    }
                                    CloudFilterLocalStatus::RemoteDeleted => {
                                        crate::tr!("远端已删除", "Deleted remotely")
                                    }
                                    CloudFilterLocalStatus::ProtocolUnsupported => {
                                        crate::tr!("需升级服务器", "Server upgrade required")
                                    }
                                };
                                let action_disabled = matches!(
                                    status,
                                    CloudFilterLocalStatus::Synced
                                        | CloudFilterLocalStatus::LocalModified
                                        | CloudFilterLocalStatus::RemoteDeleted
                                        | CloudFilterLocalStatus::ProtocolUnsupported
                                );
                                let owner = if item.owner_name.is_empty() {
                                    &item.owner_id
                                } else {
                                    &item.owner_name
                                };
                                let detail_accessibility_label = if item.note.is_empty() {
                                    crate::tr_args!(
                                        "打开云端过滤器详情：{} · 分享者：{owner}",
                                        "Open cloud filter details: {} · owner: {owner}",
                                        item.name
                                    )
                                } else {
                                    crate::tr_args!(
                                        "打开云端过滤器详情：{} · 分享者：{}\n{}",
                                        "Open cloud filter details: {} · owner: {}\n{}",
                                        item.name,
                                        owner,
                                        item.note
                                    )
                                };
                                let like_accessibility_label = if item.liked {
                                    crate::tr_args!(
                                        "取消点赞云端过滤器：{}，当前 {} 个赞",
                                        "Unlike cloud filter: {}; {} likes",
                                        item.name,
                                        item.like_count
                                    )
                                } else {
                                    crate::tr_args!(
                                        "点赞云端过滤器：{}，当前 {} 个赞",
                                        "Like cloud filter: {}; {} likes",
                                        item.name,
                                        item.like_count
                                    )
                                };

                                h_flex()
                                    .id(format!("cloud-filter-row-{}", item.id))
                                    .w_full()
                                    .h(rems(FILTER_ROW_HEIGHT_REMS))
                                    .flex_none()
                                    .border_b_1()
                                    .border_color(cx.theme().border)
                                    .when(selected, |row| row.bg(cx.theme().list_active))
                                    .when(!selected, |row| {
                                        row.hover(|row| row.bg(cx.theme().tokens.list_hover))
                                    })
                                    .child(
                                        div()
                                            .w(rems(FILTER_INDEX_WIDTH_REMS))
                                            .flex_none()
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .child(
                                                Checkbox::new(format!(
                                                    "cloud-filter-select-{}",
                                                    item.id
                                                ))
                                                .checked(selected)
                                                .aria_label(crate::tr_args!(
                                                    "选择云端过滤器：{}",
                                                    "Select cloud filter: {}",
                                                    item.name
                                                ))
                                                .on_click(cx.listener(
                                                    move |this, checked: &bool, _, cx| {
                                                        if *checked {
                                                            this.cloud_selected
                                                                .insert(select_id.clone());
                                                        } else {
                                                            this.cloud_selected.remove(&select_id);
                                                        }
                                                        cx.notify();
                                                    },
                                                )),
                                            ),
                                    )
                                    .child(
                                        BaseButton::new(format!("cloud-filter-detail-{detail_id}"))
                                            .accessibility_label(detail_accessibility_label)
                                            .h_full()
                                            .flex_1()
                                            .min_w_0()
                                            .flex()
                                            .items_center()
                                            .justify_start()
                                            .disabled(busy)
                                            .when(!busy, |button| {
                                                button.hover(|button| {
                                                    button.bg(cx.theme().tokens.list_hover)
                                                })
                                            })
                                            .when_some(detail_focus, |button, focus| {
                                                button.track_focus(&focus).focus_visible(|style| {
                                                    style.border_1().border_color(cx.theme().ring)
                                                })
                                            })
                                            .child(
                                                h_flex()
                                                    .size_full()
                                                    .min_w_0()
                                                    .child(
                                                        div()
                                                            .w(rems(CLOUD_NAME_WIDTH_REMS))
                                                            .flex_none()
                                                            .px_3()
                                                            .truncate()
                                                            .text_sm()
                                                            .font_semibold()
                                                            .child(item.name.clone()),
                                                    )
                                                    .child(
                                                        div()
                                                            .flex_1()
                                                            .min_w_0()
                                                            .px_3()
                                                            .truncate()
                                                            .text_xs()
                                                            .font_family(
                                                                cx.theme().mono_font_family.clone(),
                                                            )
                                                            .text_color(cx.theme().muted_foreground)
                                                            .child(item.value.clone()),
                                                    )
                                                    .child(
                                                        div()
                                                            .w(rems(FILTER_TYPE_WIDTH_REMS))
                                                            .flex_none()
                                                            .flex()
                                                            .items_center()
                                                            .justify_center()
                                                            .child(self.render_type_badge(
                                                                item.use_regex,
                                                                cx,
                                                            )),
                                                    ),
                                            )
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.open_cloud_detail(
                                                    detail_id.clone(),
                                                    window,
                                                    cx,
                                                )
                                            })),
                                    )
                                    .child(
                                        h_flex()
                                            .w(rems(CLOUD_ACTIONS_WIDTH_REMS))
                                            .flex_none()
                                            .justify_end()
                                            .gap_1()
                                            .pr_2()
                                            .child(
                                                BaseButton::new(format!(
                                                    "cloud-filter-like-{like_id}"
                                                ))
                                                .accessibility_label(like_accessibility_label)
                                                .h_6()
                                                .px_2()
                                                .flex_none()
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .gap_1()
                                                .rounded(cx.theme().radius)
                                                .text_xs()
                                                .text_color(if item.liked {
                                                    cx.theme().primary
                                                } else {
                                                    cx.theme().muted_foreground
                                                })
                                                .disabled(busy || self.cloud_offline)
                                                .when(!busy && !self.cloud_offline, |button| {
                                                    button
                                                        .hover(|button| {
                                                            button
                                                                .bg(cx.theme().tokens.button_hover)
                                                        })
                                                        .active(|button| {
                                                            button
                                                                .bg(cx.theme().tokens.button_active)
                                                        })
                                                })
                                                .focus_visible(|style| {
                                                    style.border_1().border_color(cx.theme().ring)
                                                })
                                                .styles(|styles| {
                                                    styles.disabled(|style| style.opacity(0.5))
                                                })
                                                .child(Icon::new(IconName::Heart).small())
                                                .child(item.like_count.to_string())
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    this.toggle_cloud_like(like_id.clone(), cx)
                                                })),
                                            )
                                            .child(
                                                h_flex()
                                                    .gap_1()
                                                    .px_1()
                                                    .text_xs()
                                                    .text_color(cx.theme().muted_foreground)
                                                    .child(Icon::new(IconName::ArrowDown).xsmall())
                                                    .child(item.download_count.to_string()),
                                            )
                                            .child(
                                                Button::new(format!(
                                                    "cloud-filter-action-{action_id}"
                                                ))
                                                .small()
                                                .when(
                                                    matches!(
                                                        status,
                                                        CloudFilterLocalStatus::NotDownloaded
                                                            | CloudFilterLocalStatus::RemoteUpdated
                                                            | CloudFilterLocalStatus::AutoMerge
                                                    ),
                                                    |button| button.outline(),
                                                )
                                                .when(
                                                    status == CloudFilterLocalStatus::Conflict,
                                                    |button| button.danger(),
                                                )
                                                .when(action_disabled, |button| button.ghost())
                                                .icon(if status == CloudFilterLocalStatus::Synced {
                                                    IconName::Check
                                                } else if status == CloudFilterLocalStatus::Conflict
                                                {
                                                    IconName::Info
                                                } else {
                                                    IconName::ArrowDown
                                                })
                                                .label(action_label)
                                                .disabled(busy || action_disabled)
                                                .on_click(cx.listener(
                                                    move |this, _, window, cx| {
                                                        this.download_cloud_filters(
                                                            vec![action_id.clone()],
                                                            window,
                                                            cx,
                                                        );
                                                    },
                                                )),
                                            ),
                                    )
                                    .into_any_element()
                            })),
                    )
                    .child(filter_table_scrollbar(
                        "cloud-filter-table-scrollbar",
                        &self.cloud_scroll,
                    )),
            )
    }

    fn render_cloud_detail_page(
        &self,
        detail: &CloudFilterItem,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let busy = self.cloud_task.is_some();
        let online = self.cloud_connection.is_some() && !self.cloud_offline;
        let protocol_supported = self
            .cloud_connection
            .as_ref()
            .is_some_and(CloudConnectionProfile::supports_uuid_filter_branches);
        let editable = false;
        let local_filters = self.draft_filters(cx);
        let server_url = self
            .cloud_connection
            .as_ref()
            .map(|connection| connection.server_url.as_str())
            .unwrap_or_default();
        let local_status = cloud_filter_local_status(&local_filters, server_url, detail);
        let can_replace_local = matches!(
            local_status,
            CloudFilterLocalStatus::NotDownloaded
                | CloudFilterLocalStatus::RemoteUpdated
                | CloudFilterLocalStatus::AutoMerge
                | CloudFilterLocalStatus::Conflict
        );
        let revision_total_pages = self
            .cloud_revision_total
            .div_ceil(u64::from(self.cloud_revision_page_size.max(1)))
            .max(1);
        let revision_preview = self.cloud_revision.clone();
        let owner = if detail.owner_name.is_empty() {
            detail.owner_id.clone()
        } else {
            detail.owner_name.clone()
        };
        let regex_id = detail.id.clone();
        let collaborative_id = detail.id.clone();
        let copy_cloud_id = detail.id.clone();

        v_flex()
            .id(format!("cloud-filter-detail-page-{}", detail.id))
            .size_full()
            .min_h_0()
            .overflow_hidden()
            .bg(cx.theme().background)
            .child(
                TabBar::new("cloud-filter-detail-tabs")
                    .w_full()
                    .flex_none()
                    .large()
                    .underline()
                    .selected_index(match self.cloud_detail_tab {
                        FilterDetailTab::Details => 0,
                        FilterDetailTab::Revisions => 1,
                    })
                    .px_6()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(Tab::new().label(crate::tr!("详情", "Details")))
                    .child(Tab::new().label(crate::tr!("编辑记录", "Revision history")))
                    .on_click(cx.listener(|this, index: &usize, _, cx| {
                        this.cloud_detail_tab = if *index == 0 {
                            FilterDetailTab::Details
                        } else {
                            FilterDetailTab::Revisions
                        };
                        cx.notify();
                    })),
            )
            .child(
                v_flex()
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .when(self.cloud_detail_tab == FilterDetailTab::Details, |body| {
                        body.child(
                            h_flex()
                                .w_full()
                                .items_start()
                                .justify_center()
                                .px_8()
                                .py_6()
                                .child(
                                    v_flex()
                                        .w_full()
                                        .max_w(rems(68.))
                                        .gap_6()
                                        .when_some(
                                            self.cloud_message.clone(),
                                            |content, message| {
                                                content.child(
                                                    div()
                                                        .w_full()
                                                        .p_3()
                                                        .rounded(cx.theme().radius)
                                                        .bg(cx.theme().muted.opacity(0.35))
                                                        .text_sm()
                                                        .child(message),
                                                )
                                            },
                                        )
                                        .child(
                                            v_flex()
                                                .gap_2()
                                                .child(
                                                    h_flex()
                                                        .justify_between()
                                                        .gap_4()
                                                        .child(
                                                            div().font_semibold().child(crate::tr!("名称", "Name")),
                                                        )
                                                        .child(
                                                            h_flex()
                                                                .gap_2()
                                                                .text_sm()
                                                                .text_color(
                                                                    cx.theme().muted_foreground,
                                                                )
                                                                .child(
                                                                    Switch::new(format!(
                                                                        "cloud-detail-collaborative-{collaborative_id}"
                                                                    ))
                                                                    .small()
                                                                    .checked(
                                                                        self.cloud_detail_collaborative,
                                                                    )
                                                                    .disabled(!editable || !detail.can_delete)
                                                                    .tooltip(if detail.can_delete {
                                                                        crate::tr!(
                                                                            "允许其他用户共同编辑",
                                                                            "Allow other users to edit collaboratively"
                                                                        )
                                                                    } else {
                                                                        crate::tr!(
                                                                            "只有分享者可以修改共创权限",
                                                                            "Only the owner can change collaboration access"
                                                                        )
                                                                    })
                                                                    .on_click(cx.listener(
                                                                        |this,
                                                                         checked: &bool,
                                                                         _,
                                                                         cx| {
                                                                            this.cloud_detail_collaborative = *checked;
                                                                            cx.notify();
                                                                        },
                                                                    )),
                                                                )
                                                                .child(crate::tr!("允许共创", "Allow collaboration")),
                                                        ),
                                                )
                                                .child(
                                                    Input::new(&self.cloud_detail_name)
                                                        .large()
                                                        .readonly(!editable),
                                                ),
                                        )
                                        .child(
                                            v_flex()
                                                .gap_2()
                                                .child(
                                                    h_flex()
                                                        .justify_between()
                                                        .gap_4()
                                                        .child(
                                                            div()
                                                                .font_semibold()
                                                                .child(crate::tr!("匹配的值", "Match value")),
                                                        )
                                                        .child(
                                                            h_flex()
                                                                .gap_2()
                                                                .text_sm()
                                                                .text_color(
                                                                    cx.theme().muted_foreground,
                                                                )
                                                                .child(
                                                                    Switch::new(format!(
                                                                        "cloud-detail-regex-{regex_id}"
                                                                    ))
                                                                    .small()
                                                                    .checked(
                                                                        self.cloud_detail_use_regex,
                                                                    )
                                                                    .disabled(!editable)
                                                                    .tooltip(crate::tr!("使用正则表达式", "Use regular expressions"))
                                                                    .on_click(cx.listener(
                                                                        |this,
                                                                         checked: &bool,
                                                                         _,
                                                                         cx| {
                                                                            this.cloud_detail_use_regex = *checked;
                                                                            cx.notify();
                                                                        },
                                                                    )),
                                                                )
                                                                .child(crate::tr!("正则", "Regex")),
                                                        ),
                                                )
                                                .child(
                                                    Input::new(&self.cloud_detail_value)
                                                        .large()
                                                        .readonly(!editable),
                                                ),
                                        )
                                        .child(
                                            v_flex()
                                                .gap_2()
                                                .child(div().font_semibold().child(crate::tr!("备注", "Note")))
                                                .child(
                                                    Input::new(&self.cloud_detail_note)
                                                        .large()
                                                        .readonly(!editable),
                                                ),
                                        )
                                        .child(
                                            v_flex()
                                                .gap_2()
                                                .child(div().font_semibold().child("UUID"))
                                                .child(
                                                    h_flex()
                                                        .gap_2()
                                                        .child(
                                                            div()
                                                                .flex_1()
                                                                .min_w_0()
                                                                .truncate()
                                                                .font_family(
                                                                    cx.theme()
                                                                        .mono_font_family
                                                                        .clone(),
                                                                )
                                                                .child(copy_cloud_id.clone()),
                                                        )
                                                        .child(
                                                            Button::new(format!(
                                                                "cloud-filter-copy-uuid-{}",
                                                                detail.id
                                                            ))
                                                            .small()
                                                            .outline()
                                                            .label(crate::tr!("复制", "Duplicate"))
                                                            .on_click(cx.listener({
                                                                let copy_cloud_id =
                                                                    copy_cloud_id.clone();
                                                                move |_, _, _, cx| {
                                                                    cx.write_to_clipboard(
                                                                        ClipboardItem::new_string(
                                                                            copy_cloud_id.clone(),
                                                                        ),
                                                                    )
                                                                }
                                                            })),
                                                        ),
                                                ),
                                        )
                                        .child(
                                            v_flex()
                                                .w_full()
                                                .gap_5()
                                                .p_4()
                                                .rounded(cx.theme().radius_lg)
                                                .border_1()
                                                .border_color(cx.theme().border)
                                                .bg(cx.theme().group_box)
                                                .child(
                                                    h_flex()
                                                        .w_full()
                                                        .gap_6()
                                                        .child(
                                                            v_flex()
                                                                .flex_1()
                                                                .gap_1()
                                                                .child(
                                                                    div()
                                                                        .text_xs()
                                                                        .text_color(
                                                                            cx.theme()
                                                                                .muted_foreground,
                                                                        )
                                                                        .child(crate::tr!("分享者", "Owner")),
                                                                )
                                                                .child(
                                                                    div().text_sm().child(owner),
                                                                ),
                                                        )
                                                        .child(
                                                            v_flex()
                                                                .flex_1()
                                                                .gap_1()
                                                                .child(
                                                                    div()
                                                                        .text_xs()
                                                                        .text_color(
                                                                            cx.theme()
                                                                                .muted_foreground,
                                                                        )
                                                                        .child(crate::tr!("修订", "Revision")),
                                                                )
                                                                .child(
                                                                    div()
                                                                        .text_sm()
                                                                        .child(
                                                                            detail
                                                                                .revision
                                                                                .to_string(),
                                                                        ),
                                                                ),
                                                        ),
                                                )
                                                .child(
                                                    h_flex()
                                                        .w_full()
                                                        .gap_6()
                                                        .child(
                                                            v_flex()
                                                                .flex_1()
                                                                .gap_1()
                                                                .child(
                                                                    div()
                                                                        .text_xs()
                                                                        .text_color(
                                                                            cx.theme()
                                                                                .muted_foreground,
                                                                        )
                                                                        .child(crate::tr!("点赞 / 下载", "Likes / downloads")),
                                                                )
                                                                .child(
                                                                    div().text_sm().child(format!(
                                                                        "{} / {}",
                                                                        detail.like_count,
                                                                        detail.download_count
                                                                    )),
                                                                ),
                                                        )
                                                        .child(
                                                            v_flex()
                                                                .flex_1()
                                                                .gap_1()
                                                                .child(
                                                                    div()
                                                                        .text_xs()
                                                                        .text_color(
                                                                            cx.theme()
                                                                                .muted_foreground,
                                                                        )
                                                                        .child(crate::tr!("更新时间", "Updated")),
                                                                )
                                                                .child(
                                                                    div().text_sm().child(
                                                                        format_cloud_timestamp(
                                                                            detail.updated_at,
                                                                        ),
                                                                    ),
                                                                ),
                                                        ),
                                                ),
                                        ),
                                ),
                        )
                    })
                    .when(
                        self.cloud_detail_tab == FilterDetailTab::Revisions,
                        |body| {
                            body.child(
                                h_flex()
                                    .w_full()
                                    .items_start()
                                    .justify_center()
                                    .px_8()
                                    .py_6()
                                    .child(
                                        v_flex()
                                            .w_full()
                                            .max_w(rems(68.))
                                            .gap_4()
                                            .when(self.cloud_offline, |content| {
                                                content.child(
                                                    div()
                                                        .p_3()
                                                        .rounded(cx.theme().radius)
                                                        .bg(cx.theme().muted.opacity(0.35))
                                                        .text_sm()
                                                        .child(
                                                            crate::tr!("离线状态下无法读取云端编辑记录", "Cloud revision history is unavailable offline"),
                                                        ),
                                                )
                                            })
                                            .when(
                                                self.cloud_revisions.is_empty()
                                                    && !self.cloud_offline,
                                                |content| {
                                                    content.child(
                                                        div()
                                                            .py_8()
                                                            .text_center()
                                                            .text_color(
                                                                cx.theme().muted_foreground,
                                                            )
                                                            .child(if busy {
                                                                crate::tr!("正在读取编辑记录…", "Reading revision history…")
                                                            } else {
                                                                crate::tr!("暂无编辑记录", "No revision history")
                                                            }),
                                                    )
                                                },
                                            )
                                            .children(self.cloud_revisions.iter().map(
                                                |revision| {
                                                    let revision_number = revision.revision;
                                                    let editor = if revision.editor_name.is_empty()
                                                    {
                                                        revision.editor_id.as_str()
                                                    } else {
                                                        revision.editor_name.as_str()
                                                    };
                                                    Button::new((
                                                        "cloud-detail-revision",
                                                        revision_number as usize,
                                                    ))
                                                    .large()
                                                    .outline()
                                                    .selected(
                                                        revision_preview.as_ref().is_some_and(
                                                            |selected| {
                                                                selected.revision
                                                                    == revision.revision
                                                            },
                                                        ),
                                                    )
                                                    .label(crate::tr_args!(
                                                        "修订 {} · {} · {}{}",
                                                        "Revision {} · {} · {}{}",
                                                        revision.revision,
                                                        editor,
                                                        format_cloud_timestamp(
                                                            revision.created_at
                                                        ),
                                                        if revision.current {
                                                            crate::tr!(" · 当前", " · current")
                                                        } else {
                                                            ""
                                                        }
                                                    ))
                                                    .disabled(busy || self.cloud_offline)
                                                    .on_click(cx.listener(
                                                        move |this, _, window, cx| {
                                                            this.load_cloud_revision(
                                                                revision_number,
                                                                window,
                                                                cx,
                                                            )
                                                        },
                                                    ))
                                                },
                                            ))
                                            .when_some(
                                                revision_preview,
                                                |content, revision| {
                                                    content.child(
                                                        v_flex()
                                                            .gap_3()
                                                            .p_4()
                                                            .rounded(cx.theme().radius_lg)
                                                            .border_1()
                                                            .border_color(cx.theme().border)
                                                            .bg(cx.theme().group_box)
                                                            .child(
                                                                h_flex()
                                                                    .justify_between()
                                                                    .gap_3()
                                                                    .child(
                                                                        div()
                                                                            .font_semibold()
                                                                            .child(crate::tr_args!(
                                                                                "修订 {} · {}",
                                                                                "Revision {} · {}",
                                                                                revision.revision,
                                                                                if revision
                                                                                    .editor_name
                                                                                    .is_empty()
                                                                                {
                                                                                    &revision
                                                                                        .editor_id
                                                                                } else {
                                                                                    &revision
                                                                                        .editor_name
                                                                                }
                                                                            )),
                                                                    )
                                                                    .when(
                                                                        !revision.current,
                                                                        |row| {
                                                                            row.child(
                                                                                Button::new(
                                                                                    "cloud-detail-restore-revision",
                                                                                )
                                                                                .small()
                                                                                .outline()
                                                                                .label(crate::tr!(
                                                                                    "恢复到本地",
                                                                                    "Restore locally"
                                                                                ))
                                                                                .disabled(busy)
                                                                                .on_click(
                                                                                    cx.listener(
                                                                                        |this, _, window, cx| {
                                                                                            this.confirm_restore_cloud_revision(window, cx)
                                                                                        },
                                                                                    ),
                                                                                ),
                                                                            )
                                                                        },
                                                                    ),
                                                            )
                                                            .child(
                                                                div()
                                                                    .text_sm()
                                                                    .font_semibold()
                                                                    .child(revision.name),
                                                            )
                                                            .child(
                                                                div()
                                                                    .text_sm()
                                                                    .font_family(
                                                                        cx.theme()
                                                                            .mono_font_family
                                                                            .clone(),
                                                                    )
                                                                    .child(if revision.use_regex {
                                                                        crate::tr_args!(
                                                                            "正则：{}",
                                                                            "Regex: {}",
                                                                            revision.value
                                                                        )
                                                                    } else {
                                                                        crate::tr_args!(
                                                                            "文本：{}",
                                                                            "Text: {}",
                                                                            revision.value
                                                                        )
                                                                    }),
                                                            )
                                                            .when(
                                                                !revision.note.is_empty(),
                                                                |preview| {
                                                                    preview.child(
                                                                        div()
                                                                            .text_sm()
                                                                            .text_color(
                                                                                cx.theme()
                                                                                    .muted_foreground,
                                                                            )
                                                                            .child(revision.note),
                                                                    )
                                                                },
                                                            ),
                                                    )
                                                },
                                            )
                                            .child(
                                                h_flex()
                                                    .w_full()
                                                    .justify_between()
                                                    .gap_3()
                                                    .child(
                                                        div()
                                                            .text_xs()
                                                            .text_color(
                                                                cx.theme().muted_foreground,
                                                            )
                                                            .child(crate::tr_args!(
                                                                "第 {} / {} 页 · 共 {} 条",
                                                                "Page {} of {} · {} total",
                                                                self.cloud_revision_page,
                                                                revision_total_pages,
                                                                self.cloud_revision_total,
                                                            )),
                                                    )
                                                    .child(
                                                        h_flex()
                                                            .gap_2()
                                                            .child(
                                                                Button::new(
                                                                    "cloud-revision-previous-page",
                                                                )
                                                                .small()
                                                                .outline()
                                                                .label(crate::tr!("上一页", "Previous"))
                                                                .disabled(
                                                                    busy
                                                                        || self.cloud_offline
                                                                        || self.cloud_revision_page
                                                                            <= 1,
                                                                )
                                                                .on_click(cx.listener(
                                                                    |this, _, window, cx| {
                                                                        this.load_cloud_revision_page(
                                                                            this.cloud_revision_page
                                                                                .saturating_sub(1),
                                                                            window,
                                                                            cx,
                                                                        )
                                                                    },
                                                                )),
                                                            )
                                                            .child(
                                                                Button::new(
                                                                    "cloud-revision-next-page",
                                                                )
                                                                .small()
                                                                .outline()
                                                                .label(crate::tr!("下一页", "Next"))
                                                                .disabled(
                                                                    busy
                                                                        || self.cloud_offline
                                                                        || u64::from(
                                                                            self.cloud_revision_page,
                                                                        ) >= revision_total_pages,
                                                                )
                                                                .on_click(cx.listener(
                                                                    |this, _, window, cx| {
                                                                        this.load_cloud_revision_page(
                                                                            this.cloud_revision_page
                                                                                .saturating_add(1),
                                                                            window,
                                                                            cx,
                                                                        )
                                                                    },
                                                                )),
                                                            ),
                                                    ),
                                            ),
                                    ),
                            )
                        },
                    ),
            )
            .child(
                h_flex()
                        .w_full()
                        .flex_none()
                        .justify_between()
                        .gap_3()
                        .px_6()
                        .py_3()
                        .border_t_1()
                        .border_color(cx.theme().border)
                        .child(
                            h_flex()
                                .gap_2()
                                .child(
                                    Button::new("cloud-detail-fork-local")
                                        .small()
                                        .outline()
                                        .label(crate::tr!("另存为新过滤器", "Save as new filter"))
                                        .disabled(busy)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.fork_cloud_detail_to_local(window, cx)
                                        })),
                                )
                                .when(detail.can_delete && online, |actions| {
                                    actions.child(
                                        BaseButton::new("cloud-detail-delete")
                                            .track_focus(&self.cloud_detail_delete_focus)
                                            .accessibility_label(crate::tr!("删除云端分享", "Delete cloud share"))
                                            .h_8()
                                            .px_3()
                                            .flex_none()
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .rounded(cx.theme().radius)
                                            .bg(cx.theme().tokens.button_danger)
                                            .text_sm()
                                            .text_color(cx.theme().button_danger_foreground)
                                            .disabled(busy)
                                            .when(!busy, |button| {
                                                button
                                                    .hover(|button| {
                                                        button.bg(
                                                            cx.theme().tokens.button_danger_hover,
                                                        )
                                                    })
                                                    .active(|button| {
                                                        button.bg(
                                                            cx.theme().tokens.button_danger_active,
                                                        )
                                                    })
                                            })
                                            .focus_visible(|style| {
                                                style
                                                    .border_1()
                                                    .border_color(cx.theme().ring)
                                            })
                                            .styles(|styles| {
                                                styles.disabled(|style| style.opacity(0.5))
                                            })
                                            .child(crate::tr!("删除分享", "Delete share"))
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.confirm_delete_cloud_filter(window, cx)
                                            })),
                                    )
                                })
                                .when(can_replace_local && online, |actions| {
                                    actions.child(
                                        Button::new("cloud-detail-use-remote")
                                            .small()
                                            .outline()
                                            .label(match local_status {
                                                CloudFilterLocalStatus::NotDownloaded => {
                                                    crate::tr!("下载到本地", "Download locally")
                                                }
                                                CloudFilterLocalStatus::Conflict => crate::tr!("解决冲突", "Resolve conflict"),
                                                CloudFilterLocalStatus::AutoMerge => crate::tr!("合并更新", "Merge update"),
                                                _ => crate::tr!("更新本地", "Update local"),
                                            })
                                            .disabled(busy)
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.replace_local_with_cloud_detail(window, cx)
                                            })),
                                    )
                                })
                                .when(detail.can_edit && online, |actions| {
                                    actions.child(
                                        Button::new("cloud-detail-use-local")
                                            .small()
                                            .outline()
                                            .label(crate::tr!("提交本地修改", "Submit local changes"))
                                            .disabled(
                                                busy
                                                    || !protocol_supported
                                                    || local_status
                                                        != CloudFilterLocalStatus::LocalModified,
                                            )
                                            .tooltip(if !protocol_supported {
                                                crate::tr!(
                                                    "当前服务器需要升级后才能提交本地修改",
                                                    "Upgrade the current server before submitting local changes"
                                                )
                                            } else if local_status
                                                != CloudFilterLocalStatus::LocalModified
                                            {
                                                crate::tr!(
                                                    "本地没有待提交的修改",
                                                    "There are no local changes to submit"
                                                )
                                            } else {
                                                crate::tr!(
                                                    "将本地修改提交为新的云端修订",
                                                    "Submit local changes as a new cloud revision"
                                                )
                                            })
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.push_local_filter_to_cloud(window, cx)
                                            })),
                                    )
                                }),
                        )
                        .child(
                            Button::new("cloud-detail-finish")
                                .large()
                                .outline()
                                .w_24()
                                .label(crate::tr!("关闭", "Close"))
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(Box::new(Cancel), cx)
                                }),
                        ),
            )
    }

    fn render_conflict_page(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(conflict) = self.merge_conflicts.first() else {
            return div().size_full().into_any_element();
        };
        let Some(resolution) = self.conflict_resolution.as_ref() else {
            return div().size_full().into_any_element();
        };
        let branch_id = conflict.id;
        let remaining = self.merge_conflicts.len();
        v_flex()
            .id(format!("filter-conflict-page-{branch_id}"))
            .size_full()
            .min_h_0()
            .overflow_hidden()
            .bg(cx.theme().background)
            .child(
                v_flex()
                    .gap_1()
                    .px_6()
                    .py_4()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(div().font_semibold().child(crate::tr_args!(
                        "逐字段选择要保留的内容 · 还剩 {remaining} 项",
                        "Choose which value to keep for each field · {remaining} remaining"
                    )))
                    .child(
                        div()
                            .text_xs()
                            .font_family(cx.theme().mono_font_family.clone())
                            .text_color(cx.theme().muted_foreground)
                            .child(branch_id.to_string()),
                    ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .gap_4()
                    .px_6()
                    .py_4()
                    .overflow_y_scrollbar()
                    .children(conflict.fields.iter().copied().map(|field| {
                        let local_value = filter_field_value(&conflict.local, field);
                        let remote_value = filter_field_value(&conflict.incoming, field);
                        let resolved_value = filter_field_value(resolution, field);
                        let local_selected = resolved_value == local_value;
                        let remote_selected = resolved_value == remote_value;
                        v_flex()
                            .id(format!(
                                "filter-conflict-field-{branch_id}-{}",
                                filter_field_label(field)
                            ))
                            .gap_2()
                            .p_3()
                            .rounded(cx.theme().radius_lg)
                            .border_1()
                            .border_color(cx.theme().border)
                            .bg(cx.theme().group_box)
                            .child(div().font_semibold().child(filter_field_label(field)))
                            .child(
                                h_flex()
                                    .gap_3()
                                    .items_start()
                                    .child(
                                        v_flex()
                                            .flex_1()
                                            .min_w_0()
                                            .gap_2()
                                            .child(
                                                Button::new(format!(
                                                    "filter-conflict-local-{branch_id}-{}",
                                                    filter_field_label(field)
                                                ))
                                                .small()
                                                .outline()
                                                .selected(local_selected)
                                                .label(crate::tr!("保留本地", "Keep local"))
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    this.choose_conflict_field(field, false, cx)
                                                })),
                                            )
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .text_color(cx.theme().muted_foreground)
                                                    .child(local_value),
                                            ),
                                    )
                                    .child(
                                        v_flex()
                                            .flex_1()
                                            .min_w_0()
                                            .gap_2()
                                            .child(
                                                Button::new(format!(
                                                    "filter-conflict-remote-{branch_id}-{}",
                                                    filter_field_label(field)
                                                ))
                                                .small()
                                                .outline()
                                                .selected(remote_selected)
                                                .label(crate::tr!("采用远程", "Use remote"))
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    this.choose_conflict_field(field, true, cx)
                                                })),
                                            )
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .text_color(cx.theme().muted_foreground)
                                                    .child(remote_value),
                                            ),
                                    ),
                            )
                    })),
            )
            .child(
                h_flex()
                    .w_full()
                    .flex_none()
                    .justify_between()
                    .gap_3()
                    .px_6()
                    .py_3()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("filter-conflict-all-local")
                                    .small()
                                    .outline()
                                    .label(crate::tr!("全部保留本地", "Keep all local"))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.choose_all_conflict_fields(false, cx)
                                    })),
                            )
                            .child(
                                Button::new("filter-conflict-all-remote")
                                    .small()
                                    .outline()
                                    .label(crate::tr!("全部采用远程", "Use all remote"))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.choose_all_conflict_fields(true, cx)
                                    })),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("filter-conflict-later")
                                    .large()
                                    .outline()
                                    .w_24()
                                    .label(crate::tr!("稍后处理", "Resolve later"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.finish_current_conflict(false, window, cx)
                                    })),
                            )
                            .child(
                                Button::new("filter-conflict-apply")
                                    .large()
                                    .primary()
                                    .w_24()
                                    .label(crate::tr!("完成合并", "Complete merge"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.finish_current_conflict(true, window, cx)
                                    })),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn render_cloud_share_page(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let busy = self.cloud_task.is_some();
        let online = self.cloud_connection.is_some() && !self.cloud_offline;
        let server_url = self
            .cloud_connection
            .as_ref()
            .map(|connection| connection.server_url.as_str())
            .unwrap_or_default();
        let shareable_filters = self
            .draft_filters(cx)
            .into_iter()
            .filter(|filter| Self::local_filter_has_publish_changes(filter, server_url))
            .collect::<Vec<_>>();
        let share_ids = self
            .cloud_share_selected
            .iter()
            .cloned()
            .collect::<Vec<_>>();

        v_flex()
            .id("cloud-filter-share-page")
            .size_full()
            .min_h_0()
            .overflow_hidden()
            .bg(cx.theme().background)
            .child(
                div()
                    .flex_none()
                    .px_6()
                    .pt_2()
                    .pb_3()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(crate::tr!("选择首次发布到云端的本地过滤器；后续变化请在详情中提交", "Select local filters to publish for the first time. Submit later changes from Details.")),
            )
            .child(
                v_flex()
                    .id("cloud-share-local-scroll")
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .gap_1()
                    .px_6()
                    .py_3()
                    .border_t_1()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().group_box)
                    .overflow_y_scrollbar()
                    .when(shareable_filters.is_empty(), |list| {
                        list.items_center()
                            .justify_center()
                            .text_color(cx.theme().muted_foreground)
                            .child(crate::tr!("没有尚未发布的变化", "No unpublished changes"))
                    })
                    .children(shareable_filters.iter().map(|filter| {
                        let filter_id = filter.id;
                        Checkbox::new(format!("cloud-share-local-{}", filter.id))
                            .checked(self.cloud_share_selected.contains(&filter.id))
                            .disabled(busy)
                            .label(format!("{} · {}", filter.name, filter.value))
                            .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                                if *checked {
                                    this.cloud_share_selected.insert(filter_id);
                                } else {
                                    this.cloud_share_selected.remove(&filter_id);
                                }
                                cx.notify();
                            }))
                    })),
            )
            .when_some(self.cloud_message.clone(), |page, message| {
                page.child(
                    div()
                        .flex_none()
                        .mx_6()
                        .mt_3()
                        .p_2()
                        .rounded(cx.theme().radius)
                        .bg(cx.theme().muted.opacity(0.35))
                        .text_sm()
                        .child(message),
                )
            })
            .child(
                h_flex()
                    .w_full()
                    .flex_none()
                    .justify_end()
                    .gap_2()
                    .px_6()
                    .py_3()
                    .child(
                        Button::new("cloud-share-cancel")
                            .large()
                            .outline()
                            .w_24()
                            .label(crate::tr!("取消", "Cancel"))
                            .on_click(|_, window, cx| window.dispatch_action(Box::new(Cancel), cx)),
                    )
                    .child(
                        Button::new("cloud-share-confirm")
                            .large()
                            .primary()
                            .w_24()
                            .label(crate::tr_args!("分享所选 ({})", "Share selected ({})", share_ids.len()))
                            .loading(busy)
                            .disabled(busy || !online || share_ids.is_empty())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.share_local_filters(share_ids.clone(), window, cx)
                            })),
                    ),
            )
    }

    fn render_cloud_panel(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let busy = self.cloud_task.is_some();
        let connected = self.cloud_connection.is_some();
        let local_filters = self.draft_filters(cx);
        let selected_ids = self.cloud_selected.iter().cloned().collect::<Vec<_>>();
        let total_pages = self
            .cloud_total
            .div_ceil(u64::from(self.cloud_page_size.max(1)))
            .max(1);
        v_flex()
            .id("predefined-filter-cloud-panel")
            .size_full()
            .min_h_0()
            .overflow_hidden()
            .when(!connected, |panel| {
                panel
                    .items_center()
                    .justify_center()
                    .p_6()
                    .child(self.render_cloud_connection_panel(cx))
            })
            .when(connected, |panel| {
                panel
                    .child(self.render_cloud_toolbar(selected_ids, cx))
                    .when(self.cloud_offline, |panel| {
                        panel.child(
                            h_flex()
                                .id("cloud-filter-offline-status")
                                .justify_between()
                                .gap_3()
                                .p_2()
                                .mx_3()
                                .mt_3()
                                .rounded(cx.theme().radius)
                                .bg(cx.theme().warning.opacity(0.12))
                                .child(div().text_sm().child(crate::tr_args!(
                                    "只读离线目录 · {} · 可搜索、排序、翻页并导入本地",
                                    "Read-only offline directory · {} · search, sort, browse pages, and import locally",
                                    self.cloud_cache_age()
                                )))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(crate::tr!("点赞、发布、编辑和修订操作需重新连接", "Reconnect to like, publish, edit, or view revisions")),
                                ),
                        )
                    })
                    .child(
                        v_flex()
                            .w_full()
                            .flex_1()
                            .min_h_0()
                            .px_4()
                            .pt_4()
                            .overflow_hidden()
                            .child(self.render_cloud_directory_table(&local_filters, window, cx)),
                    )
                    .child(
                        h_flex()
                            .w_full()
                            .flex_none()
                            .justify_center()
                            .gap_2()
                            .px_3()
                            .py_2()
                            .child(
                                Button::new("cloud-filter-previous-page")
                                    .small()
                                    .outline()
                                    .w(rems(4.8))
                                    .h(rems(1.87))
                                    .label(crate::tr!("上一页", "Previous"))
                                    .disabled(busy || self.cloud_page <= 1)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.load_cloud_page(
                                            this.cloud_page.saturating_sub(1),
                                            window,
                                            cx,
                                        )
                                    })),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(crate::tr_args!(
                                        "第 {} 页 · 共 {} 条",
                                        "Page {} · {} total",
                                        self.cloud_page, self.cloud_total
                                    )),
                            )
                            .child(
                                Button::new("cloud-filter-next-page")
                                    .small()
                                    .outline()
                                    .w(rems(4.8))
                                    .h(rems(1.87))
                                    .label(crate::tr!("下一页", "Next"))
                                    .disabled(busy || u64::from(self.cloud_page) >= total_pages)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.load_cloud_page(
                                            this.cloud_page.saturating_add(1),
                                            window,
                                            cx,
                                        )
                                    })),
                            ),
                    )
            })
            .when_some(self.cloud_message.clone(), |panel, message| {
                panel.child(
                    div()
                        .flex_none()
                        .mx_3()
                        .mb_2()
                        .p_2()
                        .rounded(cx.theme().radius)
                        .bg(cx.theme().muted.opacity(0.35))
                        .text_sm()
                        .child(message),
                )
            })
    }

    fn render_footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let drafts_valid = self.filters(cx).is_ok();
        let busy = self.io_task.is_some();
        div()
            .id("predefined-filter-footer")
            .w_full()
            .flex_none()
            .border_t_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .child(
                DialogFooter::new()
                    .w_full()
                    .px_4()
                    .py_4()
                    .when(self.active_tab == DialogTab::Local, |footer| {
                        footer
                            .child(
                                Button::new("predefined-filter-dialog-cancel")
                                    .large()
                                    .outline()
                                    .w_24()
                                    .label(crate::tr!("取消", "Cancel"))
                                    .on_click(|_, window, cx| {
                                        window.dispatch_action(Box::new(Cancel), cx)
                                    }),
                            )
                            .child(
                                Button::new("predefined-filter-dialog-apply")
                                    .large()
                                    .outline()
                                    .w_24()
                                    .label(crate::tr!("保存", "Save"))
                                    .disabled(busy || !drafts_valid)
                                    .tooltip(if drafts_valid {
                                        crate::tr!(
                                            "保存更改但保持窗口打开",
                                            "Save changes and keep the window open"
                                        )
                                    } else {
                                        crate::tr!(
                                            "请先完成并修正本地过滤器草稿",
                                            "Complete and fix local filter drafts first"
                                        )
                                    })
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.save_filters(window, cx)
                                    })),
                            )
                            .child(
                                Button::new("predefined-filter-dialog-confirm")
                                    .large()
                                    .primary()
                                    .w_24()
                                    .label(crate::tr!("确定", "OK"))
                                    .disabled(busy || !drafts_valid)
                                    .on_click(|_, window, cx| {
                                        window.dispatch_action(
                                            Box::new(Confirm { secondary: false }),
                                            cx,
                                        )
                                    }),
                            )
                    })
                    .when(self.active_tab == DialogTab::Cloud, |footer| {
                        footer.child(
                            Button::new("predefined-filter-dialog-close")
                                .large()
                                .outline()
                                .w_24()
                                .label(crate::tr!("关闭", "Close"))
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(Box::new(Cancel), cx)
                                }),
                        )
                    }),
            )
    }

    fn render_secondary_dialog(&self, cx: &mut Context<Self>) -> AnyElement {
        match self.secondary_route.as_ref() {
            Some(FilterSecondaryRoute::Share) => {
                return self.render_cloud_share_page(cx).into_any_element();
            }
            Some(FilterSecondaryRoute::LocalDetail(detail_id)) => {
                if let Some((index, row)) = self
                    .rows
                    .iter()
                    .enumerate()
                    .find(|(_, row)| row.filter.id == *detail_id)
                {
                    return self
                        .render_local_detail_page(row, index, cx)
                        .into_any_element();
                }
            }
            Some(FilterSecondaryRoute::CloudDetail(detail_id)) => {
                if let Some(detail) = self
                    .cloud_detail
                    .as_ref()
                    .filter(|detail| &detail.id == detail_id)
                {
                    return self.render_cloud_detail_page(detail, cx).into_any_element();
                }
            }
            Some(FilterSecondaryRoute::Conflict) => return self.render_conflict_page(cx),
            None => {}
        }
        div().size_full().into_any_element()
    }
}

impl Render for PredefinedFilterSecondarySurface {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _performance_scope =
            crate::ui_performance::scope("PredefinedFilterSecondarySurface::render");
        let dialog = self.dialog.clone();
        dialog
            .update(cx, |dialog, cx| dialog.render_secondary_dialog(cx))
            .unwrap_or_else(|_| div().into_any_element())
    }
}

impl Render for PredefinedFiltersDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _performance_scope = crate::ui_performance::scope("PredefinedFiltersDialog::render");
        v_flex()
            .id("predefined-filters-dialog-content")
            .size_full()
            .min_h_0()
            .rounded(cx.theme().radius_lg)
            .overflow_hidden()
            .bg(cx.theme().muted.opacity(0.2))
            .child(self.render_tabs(cx))
            .child(
                v_flex()
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .when(self.active_tab == DialogTab::Local, |body| {
                        body.child(self.render_local_panel(window, cx))
                    })
                    .when(self.active_tab == DialogTab::Cloud, |body| {
                        body.child(self.render_cloud_panel(window, cx))
                    }),
            )
            .child(self.render_footer(cx))
    }
}
