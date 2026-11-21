use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    rc::Rc,
};

use chrono::{DateTime, Local, Utc};
use gpui_kit::base::Link;
use gpui_kit::component::{
    ActiveTheme as _, Colorize as _, Disableable as _, IconName, IndexPath, Selectable as _,
    Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    color_picker::{ColorPicker, ColorPickerEvent, ColorPickerState},
    description_list::DescriptionList,
    dialog::DialogFooter,
    h_flex,
    input::{Input, InputContentType, InputEvent, InputState, NumberInput},
    radio::{Radio, RadioGroup},
    scroll::ScrollableElement as _,
    select::{SearchableVec, Select, SelectEvent, SelectItem, SelectState},
    sidebar::{Sidebar, SidebarMenu, SidebarMenuItem},
    slider::{Slider, SliderEvent, SliderState},
    switch::Switch,
    theme::{ThemeMode, try_parse_color},
    v_flex,
};
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, EventEmitter, Focusable as _,
    InteractiveElement as _, IntoElement, Keystroke, ObjectFit, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, StyledImage as _, Subscription,
    Task, UniformListScrollHandle, Window, div, img, prelude::FluentBuilder as _, uniform_list,
};

use crate::{
    app_icon::AppIcon,
    app_log::{self, AppLogLevel},
    cloud_filters::{CloudClient, CloudConnectionProfile},
    history_dialog::{
        HISTORY_ROW_HEIGHT_REMS, HistoryCategory, HistoryDialog, persistent_list_scrollbar,
    },
    i18n::Language,
    notifications::NotificationWindowExt as _,
    state_store::{
        AppSettings, CloudSettings, LogFontFamily, MAX_WORD_BOUNDARY_CHARACTERS,
        SEARCH_TOOLBAR_FONT_SIZE_RANGE, SEARCH_TOOLBAR_HEIGHT_RANGE, ShortcutSettings,
        ThemePreference, normalize_search_history,
    },
    system_fonts::SystemFonts,
};
use vclogg_data::IndexCacheInfo;

const GITHUB_REPOSITORY_URL: &str = "https://github.com/zhaiyanqi/vclogg2";
const SHORTCUT_INPUT_CONTEXT: &str = "VCLogg2ShortcutInput";

impl SelectItem for LogFontFamily {
    type Value = Self;

    fn title(&self) -> gpui_kit::SharedString {
        match self {
            Self::CascadiaMono => "Cascadia Mono".into(),
            Self::JetBrainsMono => "JetBrains Mono".into(),
            Self::Consolas => "Consolas".into(),
            Self::SystemMonospace => crate::tr!("系统等宽字体", "System monospace").into(),
            Self::Named(name) => name.clone().into(),
        }
    }

    fn value(&self) -> &Self::Value {
        self
    }
}

impl SelectItem for AppLogLevel {
    type Value = Self;

    fn title(&self) -> SharedString {
        match self {
            Self::Off => crate::tr!("关闭", "Off"),
            Self::Error => "Error",
            Self::Warn => "Warn",
            Self::Info => "Info",
            Self::Debug => "Debug",
            Self::Trace => "Trace",
        }
        .into()
    }

    fn value(&self) -> &Self::Value {
        self
    }
}

fn theme_preference_label(preference: ThemePreference) -> &'static str {
    match preference {
        ThemePreference::System => crate::tr!("跟随系统", "System"),
        ThemePreference::Light => crate::tr!("晨雾浅色", "Morning mist"),
        ThemePreference::Dark => crate::tr!("深海夜色", "Deep sea"),
    }
}

fn theme_preference_description(preference: ThemePreference) -> &'static str {
    match preference {
        ThemePreference::System => crate::tr!(
            "随操作系统自动切换浅色或深色主题",
            "Switch between light and dark with the operating system",
        ),
        ThemePreference::Light => crate::tr!(
            "默认的柔和雾面材质与明亮背景",
            "Soft matte surfaces with a bright background",
        ),
        ThemePreference::Dark => crate::tr!(
            "低眩光深色材质，适合暗光环境",
            "Low-glare dark surfaces for dim environments",
        ),
    }
}

pub(crate) mod layout;
pub(crate) mod preview;
mod reading;
use layout::{section, slider};
pub(crate) mod search;
use search::SearchOption;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum SettingsCategory {
    #[default]
    General,
    Network,
    Reading,
    Highlight,
    Search,
    History,
    Shortcuts,
    Diagnostics,
    About,
}

impl SettingsCategory {
    const ALL: [Self; 9] = [
        Self::General,
        Self::Reading,
        Self::Highlight,
        Self::Search,
        Self::Shortcuts,
        Self::History,
        Self::Network,
        Self::Diagnostics,
        Self::About,
    ];

    pub(crate) fn from_storage_value(value: &str) -> Option<Self> {
        match value {
            "general" | "windows" => Some(Self::General),
            "reading" | "appearance" | "scrolling" => Some(Self::Reading),
            "highlight" => Some(Self::Highlight),
            "search" => Some(Self::Search),
            "shortcuts" => Some(Self::Shortcuts),
            "history" | "storage" => Some(Self::History),
            "network" => Some(Self::Network),
            "diagnostics" | "advanced" => Some(Self::Diagnostics),
            "about" => Some(Self::About),
            _ => None,
        }
    }

    pub(crate) fn storage_value(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Reading => "reading",
            Self::Highlight => "highlight",
            Self::Search => "search",
            Self::Shortcuts => "shortcuts",
            Self::History => "history",
            Self::Network => "network",
            Self::Diagnostics => "diagnostics",
            Self::About => "about",
        }
    }

    pub(crate) fn is_available(self) -> bool {
        true
    }

    pub(crate) fn shows_log_preview(self) -> bool {
        matches!(self, Self::Reading | Self::Highlight)
    }

    fn icon(self) -> gpui_kit::assets::IconName {
        use gpui_kit::assets::IconName as Icon;
        match self {
            Self::General => Icon::Settings2,
            Self::Reading => Icon::BookOpen,
            Self::Highlight => Icon::Highlighter,
            Self::Search => Icon::Search,
            Self::Shortcuts => Icon::Keyboard,
            Self::History => Icon::Database,
            Self::Network => Icon::Cloud,
            Self::Diagnostics => Icon::Activity,
            Self::About => Icon::Info,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::General => crate::tr!("通用", "General"),
            Self::Reading => crate::tr!("日志阅读", "Log reading"),
            Self::Highlight => crate::tr!("高亮与标注", "Highlights & annotations"),
            Self::Search => crate::tr!("搜索", "Search"),
            Self::Shortcuts => crate::tr!("快捷键", "Shortcuts"),
            Self::History => crate::tr!("历史与存储", "History & storage"),
            Self::Network => crate::tr!("云端连接", "Cloud connection"),
            Self::Diagnostics => crate::tr!("诊断与性能", "Diagnostics & performance"),
            Self::About => crate::tr!("关于", "About"),
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NetworkStatusKind {
    Neutral,
    Success,
    Error,
}

fn system_display_name() -> String {
    std::env::var_os("USERNAME")
        .or_else(|| std::env::var_os("USER"))
        .map(|value| value.to_string_lossy().trim().chars().take(64).collect())
        .unwrap_or_default()
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
enum ShortcutAction {
    OpenFile,
    FocusSearch,
    QuickFind,
    CloseTab,
    OpenSettings,
    ToggleCase,
    JumpBottom,
    CycleColorLabel,
    ToggleWordWrap,
    AddTextMark,
    EnterEditMode,
}

impl ShortcutAction {
    fn groups() -> [(&'static str, &'static [Self]); 3] {
        [
            (
                crate::tr!("文件与窗口", "Files and windows"),
                &[Self::OpenFile, Self::CloseTab, Self::OpenSettings],
            ),
            (
                crate::tr!("搜索与查找", "Search and find"),
                &[Self::FocusSearch, Self::QuickFind, Self::ToggleCase],
            ),
            (
                crate::tr!("阅读与标注", "Reading and annotation"),
                &[
                    Self::JumpBottom,
                    Self::ToggleWordWrap,
                    Self::CycleColorLabel,
                    Self::AddTextMark,
                    Self::EnterEditMode,
                ],
            ),
        ]
    }

    fn all() -> impl Iterator<Item = Self> {
        Self::groups()
            .into_iter()
            .flat_map(|(_, actions)| actions.iter().copied())
    }

    fn id(self) -> &'static str {
        match self {
            Self::OpenFile => "open-file",
            Self::FocusSearch => "focus-search",
            Self::QuickFind => "quick-find",
            Self::CloseTab => "close-tab",
            Self::OpenSettings => "open-settings",
            Self::ToggleCase => "toggle-case",
            Self::JumpBottom => "jump-bottom",
            Self::CycleColorLabel => "cycle-color-label",
            Self::ToggleWordWrap => "toggle-word-wrap",
            Self::AddTextMark => "add-text-mark",
            Self::EnterEditMode => "enter-edit-mode",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::OpenFile => crate::tr!("打开文件", "Open file"),
            Self::FocusSearch => crate::tr!("聚焦搜索框", "Focus search"),
            Self::QuickFind => crate::tr!("快速查找", "Quick find"),
            Self::CloseTab => crate::tr!("关闭当前标签", "Close current tab"),
            Self::OpenSettings => crate::tr!("打开设置", "Open settings"),
            Self::ToggleCase => crate::tr!("切换区分大小写", "Toggle case sensitivity"),
            Self::JumpBottom => crate::tr!("跳到日志底部", "Jump to end"),
            Self::CycleColorLabel => crate::tr!("轮换颜色标签", "Cycle color label"),
            Self::ToggleWordWrap => crate::tr!("切换自动换行", "Toggle word wrap"),
            Self::AddTextMark => crate::tr!("新增文字标记", "New text mark"),
            Self::EnterEditMode => crate::tr!("进入编辑模式", "Enter edit mode"),
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::OpenFile => crate::tr!("选择并打开一个日志文件", "Choose and open a log file"),
            Self::FocusSearch => crate::tr!(
                "将键盘焦点移入主搜索框",
                "Move keyboard focus to the main search field"
            ),
            Self::QuickFind => crate::tr!(
                "在当前日志或结果中查找文本",
                "Find text in the current log or results"
            ),
            Self::CloseTab => crate::tr!("关闭当前活动标签页", "Close the active tab"),
            Self::OpenSettings => crate::tr!("打开应用设置对话框", "Open application settings"),
            Self::ToggleCase => crate::tr!(
                "切换主搜索的大小写匹配方式",
                "Toggle case sensitivity for the main search"
            ),
            Self::JumpBottom => {
                crate::tr!("滚动到当前日志末尾", "Scroll to the end of the current log")
            }
            Self::CycleColorLabel => crate::tr!(
                "为选中日志行添加、轮换或移除颜色标签",
                "Add, cycle, or remove a color label on selected log lines"
            ),
            Self::AddTextMark => crate::tr!(
                "为选中的一行打开新增文字标记弹窗",
                "Open the new text mark dialog for the selected line"
            ),
            Self::ToggleWordWrap => crate::tr!(
                "切换正文、当前结果和全局结果的长行换行显示",
                "Toggle wrapping in the log, current results, and global results"
            ),
            Self::EnterEditMode => crate::tr!(
                "从选中的日志行开始编辑当前文件",
                "Edit the current file at the selected log line"
            ),
        }
    }

    fn value(self, settings: &ShortcutSettings) -> &str {
        match self {
            Self::OpenFile => &settings.open_file,
            Self::FocusSearch => &settings.focus_search,
            Self::QuickFind => &settings.quick_find,
            Self::CloseTab => &settings.close_tab,
            Self::OpenSettings => &settings.open_settings,
            Self::ToggleCase => &settings.toggle_case_sensitive,
            Self::JumpBottom => &settings.jump_to_bottom,
            Self::CycleColorLabel => &settings.cycle_color_label,
            Self::ToggleWordWrap => &settings.toggle_word_wrap,
            Self::AddTextMark => &settings.add_text_mark,
            Self::EnterEditMode => &settings.enter_edit_mode,
        }
    }

    fn set(self, settings: &mut ShortcutSettings, value: String) {
        match self {
            Self::OpenFile => settings.open_file = value,
            Self::FocusSearch => settings.focus_search = value,
            Self::QuickFind => settings.quick_find = value,
            Self::CloseTab => settings.close_tab = value,
            Self::OpenSettings => settings.open_settings = value,
            Self::ToggleCase => settings.toggle_case_sensitive = value,
            Self::JumpBottom => settings.jump_to_bottom = value,
            Self::CycleColorLabel => settings.cycle_color_label = value,
            Self::ToggleWordWrap => settings.toggle_word_wrap = value,
            Self::AddTextMark => settings.add_text_mark = value,
            Self::EnterEditMode => settings.enter_edit_mode = value,
        }
    }
}

#[derive(Clone, Copy)]
enum LogColorTheme {
    Light,
    Dark,
}

struct LogColorDraft {
    text: Entity<ColorPickerState>,
    background: Entity<ColorPickerState>,
    text_custom: bool,
    background_custom: bool,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum HistoryTab {
    #[default]
    Files,
    TemporaryResults,
    Search,
    Cache,
}

impl HistoryTab {
    fn file_category(self) -> Option<HistoryCategory> {
        match self {
            Self::Files => Some(HistoryCategory::Files),
            Self::TemporaryResults => Some(HistoryCategory::TemporaryResults),
            Self::Search | Self::Cache => None,
        }
    }
}

pub struct SettingsDialog {
    #[cfg(windows)]
    windows_context_menu: Entity<crate::windows_context_menu::ContextMenuSettings>,
    draft: AppSettings,
    preview_selections: crate::selectable_log_text::TextSelectionCache<usize>,
    highlight_editor: Option<Entity<crate::color_labels_dialog::ColorLabelsDialog>>,
    highlight_baseline: Option<crate::color_labels_dialog::LogColoringConfig>,
    active_category: SettingsCategory,
    settings_search: Entity<InputState>,
    search_hits: Vec<SearchOption>,
    location: search::OptionLocation,
    detail_scroll: gpui_kit::ScrollHandle,
    network_server_url: Entity<InputState>,
    network_display_name: Entity<InputState>,
    cloud_client: Option<CloudClient>,
    cloud_connection: Option<CloudConnectionProfile>,
    cloud_client_error: Option<String>,
    network_status: SharedString,
    network_status_kind: NetworkStatusKind,
    network_task: Option<Task<()>>,
    file_history: Option<Entity<HistoryDialog>>,
    file_history_error: Option<SharedString>,
    history_tab: HistoryTab,
    search_history: Vec<String>,
    search_history_filter: Entity<InputState>,
    search_history_scroll: UniformListScrollHandle,
    font_family: Entity<SelectState<SearchableVec<LogFontFamily>>>,
    font_loading_task: Option<Task<()>>,
    app_log_level: Entity<SelectState<Vec<AppLogLevel>>>,
    font_size: Entity<SliderState>,
    search_toolbar_height: Entity<SliderState>,
    search_toolbar_font_size: Entity<SliderState>,
    search_input_font_size: Entity<SliderState>,
    line_spacing: Entity<SliderState>,
    line_number_width: Entity<SliderState>,
    line_number_text_color: Entity<ColorPickerState>,
    line_number_background_color: Entity<ColorPickerState>,
    line_number_text_color_custom: bool,
    line_number_background_color_custom: bool,
    light_log_colors: LogColorDraft,
    dark_log_colors: LogColorDraft,
    scroll_percent: Entity<SliderState>,
    scroll_lines: Entity<SliderState>,
    max_search_results: Entity<InputState>,
    word_boundary_characters: Entity<InputState>,
    open_directory_command: Entity<InputState>,
    shortcut_inputs: BTreeMap<ShortcutAction, Entity<InputState>>,
    cache_dir: Option<PathBuf>,
    cache_info: Option<IndexCacheInfo>,
    cache_status: Option<SharedString>,
    cache_busy: bool,
    cache_task: Option<Task<()>>,
    trace_recorder: Entity<crate::ui_performance::trace::Recorder>,
    log_status: Option<SharedString>,
    log_export_task: Option<Task<()>>,
    log_clear_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

#[derive(Clone, Debug)]
pub(crate) enum SettingsDialogEvent {
    DraftChanged,
    CategoryChanged(SettingsCategory),
    CloudConnection(Option<CloudConnectionProfile>),
}

impl EventEmitter<SettingsDialogEvent> for SettingsDialog {}

pub struct SettingsNetworkSnapshot {
    pub settings: CloudSettings,
    pub client: Option<CloudClient>,
    pub connection: Option<CloudConnectionProfile>,
    pub client_error: Option<String>,
}

impl SettingsDialog {
    pub(crate) fn select_category(
        &mut self,
        category: SettingsCategory,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if category.is_available() {
            self.clear_search_location(cx);
            self.settings_search
                .update(cx, |search, cx| search.set_value("", window, cx));
            self.active_category = category;
            if category.shows_log_preview() {
                self.ensure_highlight_editor(window, cx);
            }
            cx.emit(SettingsDialogEvent::CategoryChanged(category));
            cx.notify();
        }
    }

    pub(crate) fn sync_scrollbar_from_menu(
        &mut self,
        previous: bool,
        current: bool,
        cx: &mut Context<Self>,
    ) {
        if self.draft.show_horizontal_scrollbar == previous {
            self.draft.show_horizontal_scrollbar = current;
            cx.notify();
        }
    }

    fn locate_option(&mut self, hit: SearchOption, window: &mut Window, cx: &mut Context<Self>) {
        if hit.category.shows_log_preview() {
            self.ensure_highlight_editor(window, cx);
        }
        if hit.category == SettingsCategory::History {
            self.history_tab = match hit.key {
                "files" => HistoryTab::Files,
                "temp" => HistoryTab::TemporaryResults,
                "cache" => HistoryTab::Cache,
                _ => HistoryTab::Search,
            };
            if let Some(category) = self.history_tab.file_category()
                && let Some(history) = &self.file_history
            {
                history.update(cx, |history, cx| history.set_category(category, cx));
            }
        }
        if let Some(editor) = &self.highlight_editor
            && (hit.category == SettingsCategory::Highlight
                || self.active_category == SettingsCategory::Highlight)
        {
            let key = (hit.category == SettingsCategory::Highlight).then_some(hit.key);
            editor.update(cx, |editor, cx| editor.locate_option(key, cx));
        }
        if self.active_category != hit.category {
            self.active_category = hit.category;
            cx.emit(SettingsDialogEvent::CategoryChanged(hit.category));
        }
        self.location.select(Some(hit.key), &self.detail_scroll);
        cx.notify();
    }

    fn clear_search_location(&mut self, cx: &mut Context<Self>) {
        self.location.select(None, &self.detail_scroll);
        if self.active_category == SettingsCategory::Highlight
            && let Some(editor) = &self.highlight_editor
        {
            editor.update(cx, |editor, cx| editor.locate_option(None, cx));
        }
    }

    fn draft_changed(cx: &mut Context<Self>) {
        cx.emit(SettingsDialogEvent::DraftChanged);
        cx.notify();
    }

    fn search_size_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut settings = self.draft.clone();
        settings.search_toolbar_height =
            self.search_toolbar_height.read(cx).value().start().round() as u16;
        settings.search_toolbar_font_size = self
            .search_toolbar_font_size
            .read(cx)
            .value()
            .start()
            .round() as u16;
        settings.search_input_font_size =
            self.search_input_font_size.read(cx).value().start().round() as u16;
        let height = settings.search_toolbar_control_height();
        if height != settings.search_toolbar_height {
            self.search_toolbar_height.update(cx, |slider, cx| {
                slider.set_value(f32::from(height), window, cx);
            });
        }
        Self::draft_changed(cx);
    }

    fn render_search_size_settings(&self, cx: &gpui_kit::App) -> impl IntoElement {
        section(
            crate::tr!("搜索栏外观", "Search bar appearance"),
            Some(
                crate::tr!(
                    "字号较大时，控件会自动增高。",
                    "Controls grow to fit larger text."
                )
                .to_owned(),
            ),
            [
                self.setting_row(
                    "height",
                    crate::tr!("共用控件高度", "Shared control height"),
                    None,
                    slider(&self.search_toolbar_height, "px", cx),
                    cx,
                ),
                self.setting_row(
                    "toolbar-font",
                    crate::tr!("工具栏字号", "Toolbar text size"),
                    None,
                    slider(&self.search_toolbar_font_size, "px", cx),
                    cx,
                ),
                self.setting_row(
                    "input-font",
                    crate::tr!("输入框字号", "Search field text size"),
                    None,
                    slider(&self.search_input_font_size, "px", cx)
                        .id("settings-search-input-section"),
                    cx,
                ),
            ],
            cx,
        )
        .id("settings-search-toolbar-section")
    }

    fn current_log_color_theme(cx: &gpui_kit::App) -> LogColorTheme {
        if crate::ui_theme::is_dark(cx) {
            LogColorTheme::Dark
        } else {
            LogColorTheme::Light
        }
    }

    fn log_colors(&self, theme: LogColorTheme) -> &LogColorDraft {
        match theme {
            LogColorTheme::Light => &self.light_log_colors,
            LogColorTheme::Dark => &self.dark_log_colors,
        }
    }

    fn log_colors_mut(&mut self, theme: LogColorTheme) -> &mut LogColorDraft {
        match theme {
            LogColorTheme::Light => &mut self.light_log_colors,
            LogColorTheme::Dark => &mut self.dark_log_colors,
        }
    }

    pub fn new(
        settings: AppSettings,
        search_history: Vec<String>,
        network: SettingsNetworkSnapshot,
        active_category: SettingsCategory,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let SettingsNetworkSnapshot {
            settings: cloud_settings,
            client: cloud_client,
            connection: cloud_connection,
            client_error: cloud_client_error,
        } = network;
        let mut shortcut_inputs = BTreeMap::new();
        let mut subscriptions = Vec::new();
        let detail_scroll = gpui_kit::ScrollHandle::new();
        let settings_search = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(crate::tr!("搜索设置选项…", "Search settings…"))
                .default_value("")
        });
        let default_server_url = cloud_client
            .as_ref()
            .map(CloudClient::default_server_url)
            .unwrap_or_default();
        let server_url = if cloud_settings.server_url.trim().is_empty() {
            cloud_connection
                .as_ref()
                .map(|connection| connection.server_url.as_str())
                .filter(|value| !value.trim().is_empty())
                .unwrap_or(default_server_url)
                .to_string()
        } else {
            cloud_settings.server_url.clone()
        };
        let display_name = if cloud_settings.display_name.trim().is_empty() {
            cloud_connection
                .as_ref()
                .map(|connection| connection.display_name.as_str())
                .filter(|value| !value.trim().is_empty())
                .map(str::to_string)
                .unwrap_or_else(system_display_name)
        } else {
            cloud_settings
                .display_name
                .trim()
                .chars()
                .take(64)
                .collect()
        };
        let network_server_url = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("https://filters.example.com")
                .default_value(server_url.clone())
        });
        let network_display_name = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(crate::tr!("工号或昵称", "Employee ID or nickname"))
                .default_value(display_name.clone())
        });
        let (network_status, network_status_kind) = if let Some(error) = &cloud_client_error {
            (
                crate::tr_args!(
                    "网络客户端不可用：{error}",
                    "Network client unavailable: {error}",
                )
                .into(),
                NetworkStatusKind::Error,
            )
        } else if let Some(connection) = cloud_connection.as_ref().filter(|value| value.connected) {
            (
                crate::tr_args!(
                    "服务器配置已就绪；已为 {} 保存 Cookie。",
                    "Server configuration is ready; a cookie was saved for {}.",
                    connection.display_name,
                )
                .into(),
                NetworkStatusKind::Success,
            )
        } else if server_url.trim().is_empty() || display_name.trim().is_empty() {
            (
                crate::tr!(
                    "请填写服务器地址和用户名。",
                    "Enter the server address and user name.",
                )
                .into(),
                NetworkStatusKind::Neutral,
            )
        } else {
            (
                crate::tr!(
                    "服务器配置已就绪；云端功能会在使用时通过 Cookie 访问。",
                    "Server configuration is ready. Cloud features will use the cookie when needed.",
                )
                .into(),
                NetworkStatusKind::Neutral,
            )
        };
        let search_history_filter = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(crate::tr!("筛选搜索历史…", "Filter search history…"))
                .default_value("")
        });
        // Keep the saved choice usable while the OS font catalog is enumerated off-thread.
        let selected_family = LogFontFamily::from_family_name(
            settings
                .log_font_family
                .family_name()
                .unwrap_or(cx.theme().mono_font_family.as_ref()),
        );
        let font_family = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(vec![selected_family]),
                Some(IndexPath::new(0)),
                window,
                cx,
            )
            .searchable(true)
        });
        let app_log_level = cx.new(|cx| {
            SelectState::new(
                AppLogLevel::ALL.to_vec(),
                Some(IndexPath::new(settings.app_log_level.select_index())),
                window,
                cx,
            )
        });
        let font_size = cx.new(|_| {
            SliderState::new()
                .min(8.)
                .max(32.)
                .step(1.)
                .default_value(settings.log_font_size as f32)
        });
        let search_toolbar_height = cx.new(|_| {
            SliderState::new()
                .min(f32::from(*SEARCH_TOOLBAR_HEIGHT_RANGE.start()))
                .max(f32::from(*SEARCH_TOOLBAR_HEIGHT_RANGE.end()))
                .step(1.)
                .default_value(f32::from(settings.search_toolbar_control_height()))
        });
        let search_toolbar_font_size = cx.new(|_| {
            SliderState::new()
                .min(f32::from(*SEARCH_TOOLBAR_FONT_SIZE_RANGE.start()))
                .max(f32::from(*SEARCH_TOOLBAR_FONT_SIZE_RANGE.end()))
                .step(1.)
                .default_value(f32::from(settings.search_toolbar_font_size))
        });
        let search_input_font_size = cx.new(|_| {
            SliderState::new()
                .min(f32::from(*SEARCH_TOOLBAR_FONT_SIZE_RANGE.start()))
                .max(f32::from(*SEARCH_TOOLBAR_FONT_SIZE_RANGE.end()))
                .step(1.)
                .default_value(f32::from(settings.search_input_font_size))
        });
        let line_spacing = cx.new(|_| {
            SliderState::new()
                .min(1.)
                .max(40.)
                .step(1.)
                .default_value(settings.log_line_spacing as f32)
        });
        let line_number_width = cx.new(|_| {
            SliderState::new()
                .min(40.)
                .max(160.)
                .step(1.)
                .default_value(settings.line_number_width.clamp(40, 160) as f32)
        });
        let line_number_text_color_custom = settings.line_number_text_color.is_some();
        let line_number_text_color_value = settings
            .line_number_text_color
            .as_deref()
            .and_then(|value| try_parse_color(value).ok())
            .unwrap_or(cx.theme().muted_foreground);
        let line_number_background_color_custom = settings.line_number_background_color.is_some();
        let line_number_background_color_value = settings
            .line_number_background_color
            .as_deref()
            .and_then(|value| try_parse_color(value).ok())
            .unwrap_or_else(|| cx.theme().muted.opacity(0.45));
        let line_number_text_color = cx.new(|cx| {
            ColorPickerState::new(window, cx).default_value(line_number_text_color_value)
        });
        let line_number_background_color = cx.new(|cx| {
            ColorPickerState::new(window, cx).default_value(line_number_background_color_value)
        });
        let light_palette = crate::ui_theme::palette_for_mode(ThemeMode::Light);
        let light_log_text_custom = settings.light_log_text_color.is_some();
        let light_log_text_value = settings
            .light_log_text_color
            .as_deref()
            .and_then(|value| try_parse_color(value).ok())
            .unwrap_or(light_palette.log_text);
        let light_log_background_custom = settings.light_log_background_color.is_some();
        let light_log_background_value = settings
            .light_log_background_color
            .as_deref()
            .and_then(|value| try_parse_color(value).ok())
            .unwrap_or(light_palette.log_background);
        let light_log_text =
            cx.new(|cx| ColorPickerState::new(window, cx).default_value(light_log_text_value));
        let light_log_background = cx
            .new(|cx| ColorPickerState::new(window, cx).default_value(light_log_background_value));
        let dark_palette = crate::ui_theme::palette_for_mode(ThemeMode::Dark);
        let dark_log_text_custom = settings.dark_log_text_color.is_some();
        let dark_log_text_value = settings
            .dark_log_text_color
            .as_deref()
            .and_then(|value| try_parse_color(value).ok())
            .unwrap_or(dark_palette.log_text);
        let dark_log_background_custom = settings.dark_log_background_color.is_some();
        let dark_log_background_value = settings
            .dark_log_background_color
            .as_deref()
            .and_then(|value| try_parse_color(value).ok())
            .unwrap_or(dark_palette.log_background);
        let dark_log_text =
            cx.new(|cx| ColorPickerState::new(window, cx).default_value(dark_log_text_value));
        let dark_log_background =
            cx.new(|cx| ColorPickerState::new(window, cx).default_value(dark_log_background_value));
        let scroll_percent = cx.new(|_| {
            SliderState::new()
                .min(1.)
                .max(400.)
                .step(1.)
                .default_value(settings.mouse_wheel_scroll_percent as f32)
        });
        let scroll_lines = cx.new(|_| {
            SliderState::new()
                .min(1.)
                .max(100.)
                .step(1.)
                .default_value(settings.mouse_wheel_scroll_lines as f32)
        });
        let max_search_results = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("0")
                .default_value(settings.max_search_results.to_string())
                .step(1_000.)
                .min(0.)
                .max(u32::MAX as f64)
        });
        let word_boundary_characters = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(crate::tr!(
                    "留空时仅按空白分词",
                    "Leave empty to split on whitespace only",
                ))
                .default_value(settings.word_boundary_characters.clone())
        });
        let open_directory_command = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(crate::tr!(
                    "例如：explorer.exe \"{directory}\"",
                    "Example: explorer.exe \"{directory}\"",
                ))
                .default_value(settings.open_directory_command.clone())
        });
        subscriptions.push(cx.subscribe(&font_size, |_, _, _: &SliderEvent, cx| {
            SettingsDialog::draft_changed(cx)
        }));
        for slider in [
            &search_toolbar_height,
            &search_toolbar_font_size,
            &search_input_font_size,
        ] {
            subscriptions.push(cx.subscribe_in(
                slider,
                window,
                |this, _, _: &SliderEvent, window, cx| {
                    this.search_size_changed(window, cx);
                },
            ));
        }
        subscriptions.push(cx.subscribe_in(
            &settings_search,
            window,
            |this, search, event: &InputEvent, window, cx| {
                match event {
                    InputEvent::Change => {
                        this.search_hits = search::results(search.read(cx).value().as_ref());
                        if let Some(first) = this.search_hits.first().copied() {
                            this.locate_option(first, window, cx);
                        } else {
                            this.clear_search_location(cx);
                            cx.notify();
                        }
                    }
                    InputEvent::PressEnter { .. } => {
                        if let Some(hit) = this
                            .search_hits
                            .iter()
                            .find(|hit| this.location.is(hit.key))
                            .copied()
                        {
                            this.locate_option(hit, window, cx);
                        }
                    }
                    _ => {}
                }
            },
        ));
        subscriptions.push(
            cx.subscribe(&network_server_url, |_, _, event: &InputEvent, cx| {
                if !matches!(event, InputEvent::Focus) {
                    Self::draft_changed(cx);
                }
            }),
        );
        let bounded_display_name = network_display_name.clone();
        subscriptions.push(cx.subscribe_in(
            &network_display_name,
            window,
            move |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    let value = bounded_display_name.read(cx).value().to_string();
                    let bounded = value.chars().take(64).collect::<String>();
                    if bounded != value {
                        bounded_display_name
                            .update(cx, move |state, cx| state.set_value(bounded, window, cx));
                    }
                    Self::draft_changed(cx);
                }
                InputEvent::PressEnter { .. } => this.connect_network(window, cx),
                InputEvent::Focus | InputEvent::Blur => cx.notify(),
            },
        ));
        subscriptions.push(
            cx.subscribe(&search_history_filter, |_, _, _: &InputEvent, cx| {
                cx.notify()
            }),
        );
        subscriptions.push(cx.subscribe(
            &font_family,
            |_, _, _: &SelectEvent<SearchableVec<LogFontFamily>>, cx| {
                SettingsDialog::draft_changed(cx)
            },
        ));
        subscriptions.push(cx.subscribe(
            &app_log_level,
            |_, _, _: &SelectEvent<Vec<AppLogLevel>>, cx| SettingsDialog::draft_changed(cx),
        ));
        subscriptions.push(cx.subscribe(&line_spacing, |_, _, _: &SliderEvent, cx| {
            SettingsDialog::draft_changed(cx)
        }));
        subscriptions.push(
            cx.subscribe(&line_number_width, |_, _, _: &SliderEvent, cx| {
                SettingsDialog::draft_changed(cx)
            }),
        );
        subscriptions.push(cx.subscribe_in(
            &line_number_text_color,
            window,
            |this, picker, _: &ColorPickerEvent, window, cx| {
                this.line_number_text_color_custom = true;
                crate::dialog_focus::restore_color_picker_trigger(picker, window, cx);
                SettingsDialog::draft_changed(cx);
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &line_number_background_color,
            window,
            |this, picker, _: &ColorPickerEvent, window, cx| {
                this.line_number_background_color_custom = true;
                crate::dialog_focus::restore_color_picker_trigger(picker, window, cx);
                SettingsDialog::draft_changed(cx);
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &light_log_text,
            window,
            |this, picker, _: &ColorPickerEvent, window, cx| {
                this.light_log_colors.text_custom = true;
                crate::dialog_focus::restore_color_picker_trigger(picker, window, cx);
                SettingsDialog::draft_changed(cx);
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &light_log_background,
            window,
            |this, picker, _: &ColorPickerEvent, window, cx| {
                this.light_log_colors.background_custom = true;
                crate::dialog_focus::restore_color_picker_trigger(picker, window, cx);
                SettingsDialog::draft_changed(cx);
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &dark_log_text,
            window,
            |this, picker, _: &ColorPickerEvent, window, cx| {
                this.dark_log_colors.text_custom = true;
                crate::dialog_focus::restore_color_picker_trigger(picker, window, cx);
                SettingsDialog::draft_changed(cx);
            },
        ));
        subscriptions.push(cx.subscribe_in(
            &dark_log_background,
            window,
            |this, picker, _: &ColorPickerEvent, window, cx| {
                this.dark_log_colors.background_custom = true;
                crate::dialog_focus::restore_color_picker_trigger(picker, window, cx);
                SettingsDialog::draft_changed(cx);
            },
        ));
        subscriptions.push(cx.subscribe(&scroll_percent, |_, _, _: &SliderEvent, cx| {
            SettingsDialog::draft_changed(cx)
        }));
        subscriptions.push(cx.subscribe(&scroll_lines, |_, _, _: &SliderEvent, cx| {
            SettingsDialog::draft_changed(cx)
        }));
        subscriptions.push(
            cx.subscribe(&max_search_results, |_, _, _: &InputEvent, cx| {
                SettingsDialog::draft_changed(cx)
            }),
        );
        subscriptions.push(
            cx.subscribe(&word_boundary_characters, |_, _, _: &InputEvent, cx| {
                SettingsDialog::draft_changed(cx)
            }),
        );
        subscriptions.push(
            cx.subscribe(&open_directory_command, |_, _, _: &InputEvent, cx| {
                SettingsDialog::draft_changed(cx)
            }),
        );

        for action in ShortcutAction::all() {
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(crate::tr!("未设置", "Not set"))
                    .default_value(action.value(&settings.shortcuts))
            });
            subscriptions.push(cx.subscribe(&input, |_, _, _: &InputEvent, cx| {
                SettingsDialog::draft_changed(cx)
            }));
            shortcut_inputs.insert(action, input);
        }

        // Action bindings run before both capture and bubble key listeners. Intercept
        // recording here so Input commands cannot consume an assigned combination.
        let inputs_for_capture = shortcut_inputs.clone();
        subscriptions.push(cx.intercept_keystrokes(move |event, window, cx| {
            if !event
                .context_stack
                .iter()
                .any(|context| context.contains(SHORTCUT_INPUT_CONTEXT))
            {
                return;
            }
            let Some(input) = inputs_for_capture
                .values()
                .find(|input| input.read(cx).focus_handle(cx).is_focused(window))
            else {
                return;
            };
            let value = match event.keystroke.key.as_str() {
                "escape" | "tab" => return,
                "backspace" | "delete" => String::new(),
                "control" | "ctrl" | "alt" | "shift" | "meta" | "cmd" | "command" => return,
                _ => shortcut_from_keystroke(&event.keystroke),
            };
            cx.stop_propagation();
            input.update(cx, |state, cx| state.set_value(value, window, cx));
        }));

        let cache_dir = crate::app_paths::index_cache_dir();
        let mut dialog = Self {
            #[cfg(windows)]
            windows_context_menu: crate::windows_context_menu::settings(cx),
            draft: settings,
            preview_selections: Default::default(),
            highlight_editor: None,
            highlight_baseline: None,
            active_category: if active_category.is_available() {
                active_category
            } else {
                SettingsCategory::default()
            },
            settings_search,
            search_hits: Vec::new(),
            location: Default::default(),
            detail_scroll,
            network_server_url,
            network_display_name,
            cloud_client,
            cloud_connection,
            cloud_client_error,
            network_status,
            network_status_kind,
            network_task: None,
            file_history: None,
            file_history_error: None,
            history_tab: HistoryTab::default(),
            search_history: normalize_search_history(search_history),
            search_history_filter,
            search_history_scroll: UniformListScrollHandle::new(),
            font_family,
            font_loading_task: None,
            app_log_level,
            font_size,
            search_toolbar_height,
            search_toolbar_font_size,
            search_input_font_size,
            line_spacing,
            line_number_width,
            line_number_text_color,
            line_number_background_color,
            line_number_text_color_custom,
            line_number_background_color_custom,
            light_log_colors: LogColorDraft {
                text: light_log_text,
                background: light_log_background,
                text_custom: light_log_text_custom,
                background_custom: light_log_background_custom,
            },
            dark_log_colors: LogColorDraft {
                text: dark_log_text,
                background: dark_log_background,
                text_custom: dark_log_text_custom,
                background_custom: dark_log_background_custom,
            },
            scroll_percent,
            scroll_lines,
            max_search_results,
            word_boundary_characters,
            open_directory_command,
            shortcut_inputs,
            cache_dir,
            cache_info: None,
            cache_status: None,
            cache_busy: false,
            cache_task: None,
            trace_recorder: crate::ui_performance::trace::recorder(cx),
            log_status: None,
            log_export_task: None,
            log_clear_task: None,
            _subscriptions: subscriptions,
        };
        dialog.load_font_families(window, cx);
        dialog.refresh_cache_info(cx);
        dialog
    }

    fn load_font_families(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let fonts = cx.global::<SystemFonts>().clone();
        self.font_loading_task = Some(cx.spawn_in(window, async move |this, cx| {
            let families = cx
                .background_spawn(async move {
                    fonts
                        .available_families()
                        .into_iter()
                        .map(|name| LogFontFamily::from_family_name(&name))
                        .collect::<Vec<_>>()
                })
                .await;
            _ = this.update_in(cx, |this, window, cx| {
                this.font_family.update(cx, |select, cx| {
                    let selected = select.selected_value().cloned();
                    let mut families = families;
                    // Missing fonts remain selected so unrelated edits never replace them.
                    if let Some(selected) = &selected
                        && !families.contains(selected)
                    {
                        families.push(selected.clone());
                    }
                    select.set_items(SearchableVec::new(families), window, cx);
                    if let Some(selected) = selected {
                        select.set_selected_value(&selected, window, cx);
                    }
                    cx.notify();
                });
                this.font_loading_task = None;
                cx.notify();
            });
        }));
    }

    pub(crate) fn with_highlight_baseline(
        mut self,
        baseline: crate::color_labels_dialog::LogColoringConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        self.highlight_baseline = Some(baseline);
        if self.active_category.shows_log_preview() {
            self.ensure_highlight_editor(window, cx);
        }
        self
    }

    pub(crate) fn highlight_editor(
        &self,
    ) -> Option<Entity<crate::color_labels_dialog::ColorLabelsDialog>> {
        self.highlight_editor.clone()
    }

    pub(crate) fn ensure_highlight_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.highlight_editor.is_some() {
            return;
        }
        let Some(baseline) = self.highlight_baseline.take() else {
            return;
        };
        let editor = cx.new(|cx| {
            crate::color_labels_dialog::ColorLabelsDialog::new(
                self.draft.highlight_log_levels,
                baseline.log_coloring,
                baseline.labels,
                window,
                cx,
            )
            .with_selection_styles(baseline.selection_styles, window, cx)
            .with_keyword_match_styles(baseline.keyword_match_styles, window, cx)
        });
        self._subscriptions
            .push(cx.observe(&editor, |_, _, cx| Self::draft_changed(cx)));
        self.highlight_editor = Some(editor);
        cx.notify();
    }

    fn render_performance_settings(&self, cx: &mut Context<Self>) -> impl IntoElement {
        section(
            crate::tr!("绘制性能", "Rendering performance"),
            None,
            [self.setting_row(
                "overlay",
                crate::tr!("性能叠加层", "Performance overlay"),
                Some(crate::tr!(
                    "在窗口右上角显示绘制耗时、P99、P90、最大耗时和累计帧数，帮助观察卡顿。",
                    "Show draw time, P99, P90, maximum draw time, and frame count in the top-right corner to help spot stutters.",
                )),
                Switch::new("settings-performance-overlay")
                    .small()
                    .checked(self.draft.show_performance_overlay)
                    .accessibility_label(crate::tr!("性能叠加层", "Performance overlay"))
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.draft.show_performance_overlay = *checked;
                        Self::draft_changed(cx);
                    })),
                cx,
            )],
            cx,
        )
        .id("settings-performance-section")
    }

    pub fn settings(&self, cx: &gpui_kit::App) -> Result<AppSettings, String> {
        let (settings, error) = self.settings_for_autosave(&self.draft, cx);
        match error {
            Some(error) => Err(error),
            None => Ok(settings),
        }
    }

    /// Keep each invalid field at its last valid value while other controls remain usable.
    pub(crate) fn settings_for_autosave(
        &self,
        previous: &AppSettings,
        cx: &gpui_kit::App,
    ) -> (AppSettings, Option<String>) {
        let mut error = None;
        let mut settings = self.draft.clone();
        if let Some(editor) = &self.highlight_editor {
            settings.highlight_log_levels = editor.read(cx).is_log_coloring_enabled();
        }
        settings.app_log_level = self
            .app_log_level
            .read(cx)
            .selected_value()
            .copied()
            .unwrap_or_default();
        if let Some(family) = self.font_family.read(cx).selected_value() {
            let saved_name = settings
                .log_font_family
                .family_name()
                .unwrap_or(cx.theme().mono_font_family.as_ref());
            // Preserve a legacy system-font choice until the user selects a
            // different family. A missing saved font is likewise not overwritten
            // merely by changing another setting.
            if family.family_name() != Some(saved_name) {
                settings.log_font_family = family.clone();
            }
        }
        settings.log_font_size = self.font_size.read(cx).value().start().round() as u16;
        settings.search_toolbar_height =
            self.search_toolbar_height.read(cx).value().start().round() as u16;
        settings.search_toolbar_font_size = self
            .search_toolbar_font_size
            .read(cx)
            .value()
            .start()
            .round() as u16;
        settings.search_input_font_size =
            self.search_input_font_size.read(cx).value().start().round() as u16;
        settings.search_toolbar_height = settings.search_toolbar_control_height();
        settings.log_line_spacing = self.line_spacing.read(cx).value().start().round() as u16;
        settings.line_number_width = self.line_number_width.read(cx).value().start().round() as u16;
        settings.line_number_text_color = self
            .line_number_text_color_custom
            .then(|| {
                self.line_number_text_color
                    .read(cx)
                    .value()
                    .map(|color| color.alpha(1.).to_hex())
            })
            .flatten();
        settings.line_number_background_color = self
            .line_number_background_color_custom
            .then(|| {
                self.line_number_background_color
                    .read(cx)
                    .value()
                    .map(|color| color.alpha(1.).to_hex())
            })
            .flatten();
        settings.light_log_text_color = self
            .light_log_colors
            .text_custom
            .then(|| {
                self.light_log_colors
                    .text
                    .read(cx)
                    .value()
                    .map(|color| color.alpha(1.).to_hex())
            })
            .flatten();
        settings.light_log_background_color = self
            .light_log_colors
            .background_custom
            .then(|| {
                self.light_log_colors
                    .background
                    .read(cx)
                    .value()
                    .map(|color| color.alpha(1.).to_hex())
            })
            .flatten();
        settings.dark_log_text_color = self
            .dark_log_colors
            .text_custom
            .then(|| {
                self.dark_log_colors
                    .text
                    .read(cx)
                    .value()
                    .map(|color| color.alpha(1.).to_hex())
            })
            .flatten();
        settings.dark_log_background_color = self
            .dark_log_colors
            .background_custom
            .then(|| {
                self.dark_log_colors
                    .background
                    .read(cx)
                    .value()
                    .map(|color| color.alpha(1.).to_hex())
            })
            .flatten();
        settings.mouse_wheel_scroll_percent =
            self.scroll_percent.read(cx).value().start().round() as u16;
        settings.mouse_wheel_scroll_lines =
            self.scroll_lines.read(cx).value().start().round() as u16;
        let max_search_results = self.max_search_results.read(cx).value();
        settings.max_search_results = match max_search_results.trim().parse::<u32>() {
            Ok(value) => value,
            Err(_) => {
                error = Some(
                    crate::tr!(
                        "最大搜索结果数必须是 0 到 4,294,967,295 之间的整数",
                        "Maximum search results must be an integer from 0 to 4,294,967,295",
                    )
                    .to_owned(),
                );
                previous.max_search_results
            }
        };
        settings.word_boundary_characters =
            self.word_boundary_characters.read(cx).value().to_string();
        settings.open_directory_command = self
            .open_directory_command
            .read(cx)
            .value()
            .chars()
            .take(2048)
            .collect();
        if settings.word_boundary_characters.chars().count() > MAX_WORD_BOUNDARY_CHARACTERS {
            error.get_or_insert_with(|| crate::tr_args!(
                "分词边界字符最多允许 {MAX_WORD_BOUNDARY_CHARACTERS} 个 Unicode 字符",
                "Word-boundary characters may contain at most {MAX_WORD_BOUNDARY_CHARACTERS} Unicode characters",
            ));
            settings.word_boundary_characters = previous.word_boundary_characters.clone();
        }
        let mut shortcut_error = None;
        for action in ShortcutAction::all() {
            let value = self.shortcut_value(action, cx);
            if !value.is_empty() && crate::actions::shortcut_to_key_binding(&value).is_none() {
                shortcut_error.get_or_insert_with(|| {
                    crate::tr_args!(
                        "“{}”的快捷键无效，请重新录入",
                        "The shortcut for “{}” is invalid. Enter it again.",
                        action.label(),
                    )
                });
            }
            action.set(&mut settings.shortcuts, value);
        }
        if !self.conflicts(cx).is_empty() {
            shortcut_error.get_or_insert_with(|| {
                crate::tr!(
                    "存在重复快捷键，请先解决标红的冲突",
                    "Some shortcuts are duplicated. Resolve the highlighted conflicts first.",
                )
                .to_owned()
            });
        }
        if let Some(shortcut_error) = shortcut_error {
            error.get_or_insert(shortcut_error);
            settings.shortcuts = previous.shortcuts.clone();
        }
        (settings, error)
    }

    pub fn search_history(&self) -> Vec<String> {
        normalize_search_history(self.search_history.clone())
    }

    pub fn network_settings(&self, cx: &gpui_kit::App) -> CloudSettings {
        CloudSettings {
            server_url: self.network_server_url.read(cx).value().trim().to_string(),
            display_name: self
                .network_display_name
                .read(cx)
                .value()
                .trim()
                .to_string(),
        }
    }

    fn connect_network(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.network_task.is_some() {
            return;
        }

        let settings = self.network_settings(cx);
        cx.emit(SettingsDialogEvent::DraftChanged);
        if settings.server_url.is_empty() || settings.display_name.is_empty() {
            self.network_status = crate::tr!(
                "请填写服务器地址和用户名。",
                "Enter the server address and user name.",
            )
            .into();
            self.network_status_kind = NetworkStatusKind::Error;
            cx.notify();
            return;
        }
        if settings.display_name.chars().count() > 64 {
            self.network_status = crate::tr!(
                "用户名最多允许 64 个字符。",
                "The user name may contain at most 64 characters.",
            )
            .into();
            self.network_status_kind = NetworkStatusKind::Error;
            cx.notify();
            return;
        }
        let Some(client) = self.cloud_client.clone() else {
            self.network_status = self
                .cloud_client_error
                .as_deref()
                .map(|error| {
                    crate::tr_args!(
                        "网络客户端不可用：{error}",
                        "Network client unavailable: {error}"
                    )
                })
                .unwrap_or_else(|| {
                    crate::tr!("网络客户端不可用。", "Network client unavailable.").to_string()
                })
                .into();
            self.network_status_kind = NetworkStatusKind::Error;
            cx.notify();
            return;
        };

        self.network_status = crate::tr!(
            "正在测试服务器并获取 Cookie…",
            "Testing the server and obtaining a cookie…",
        )
        .into();
        self.network_status_kind = NetworkStatusKind::Neutral;
        self.network_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn({
                    let settings = settings.clone();
                    async move { client.connect(&settings.server_url, &settings.display_name) }
                })
                .await;
            _ = this.update_in(cx, |this, window, cx| {
                this.network_task = None;
                match result {
                    Ok(connection) => {
                        this.network_server_url.update(cx, |state, cx| {
                            state.set_value(connection.server_url.clone(), window, cx)
                        });
                        this.network_display_name.update(cx, |state, cx| {
                            state.set_value(connection.display_name.clone(), window, cx)
                        });
                        this.cloud_connection = Some(connection.clone());
                        this.network_status = crate::tr_args!(
                            "测试成功，已为 {} 获取 Cookie。",
                            "Test succeeded. A cookie was obtained for {}.",
                            connection.display_name,
                        )
                        .into();
                        this.network_status_kind = NetworkStatusKind::Success;
                        cx.emit(SettingsDialogEvent::DraftChanged);
                        cx.emit(SettingsDialogEvent::CloudConnection(Some(connection)));
                    }
                    Err(error) => {
                        this.network_status =
                            crate::tr_args!("测试失败：{error}", "Test failed: {error}",).into();
                        this.network_status_kind = NetworkStatusKind::Error;
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn remove_search_history(&mut self, query: &str, cx: &mut Context<Self>) {
        self.search_history.retain(|entry| entry != query);
        Self::draft_changed(cx);
    }

    fn confirm_clear_search_history(&self, window: &mut Window, cx: &mut Context<Self>) {
        let settings = cx.entity();
        window.open_alert_dialog(cx, move |alert, _, cx| {
            let settings = settings.clone();
            alert
                .title(crate::tr!("清除搜索历史？", "Clear search history?"))
                .description(crate::tr!("清除后无法恢复。", "This cannot be undone."))
                .footer(
                    DialogFooter::new()
                        .child(crate::dialog_focus::dialog_cancel_action(
                            "settings-history-cancel-action",
                            Button::new("settings-history-cancel")
                                .label(crate::tr!("取消", "Cancel")),
                            cx,
                        ))
                        .child(crate::dialog_focus::dialog_confirm_action(
                            "settings-history-clear-action",
                            Button::new("settings-history-clear-confirm")
                                .danger()
                                .label(crate::tr!("清除历史", "Clear history")),
                            cx,
                        )),
                )
                .on_ok(move |_, _, cx| {
                    settings.update(cx, |settings, cx| settings.clear_search_history(cx));
                    true
                })
        });
    }

    fn clear_search_history(&mut self, cx: &mut Context<Self>) {
        if self.search_history.is_empty() {
            return;
        }
        self.search_history.clear();
        Self::draft_changed(cx);
    }

    fn reset_line_number_text_color(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.line_number_text_color_custom = false;
        let fallback = cx.theme().muted_foreground;
        self.line_number_text_color.update(cx, |state, cx| {
            state.set_value(fallback, window, cx);
        });
        Self::draft_changed(cx);
    }

    fn reset_line_number_background_color(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.line_number_background_color_custom = false;
        let fallback = cx.theme().muted.opacity(0.45);
        self.line_number_background_color.update(cx, |state, cx| {
            state.set_value(fallback, window, cx);
        });
        Self::draft_changed(cx);
    }

    fn reset_log_text_color(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let theme = Self::current_log_color_theme(cx);
        let mode = match theme {
            LogColorTheme::Light => ThemeMode::Light,
            LogColorTheme::Dark => ThemeMode::Dark,
        };
        let fallback = crate::ui_theme::palette_for_mode(mode).log_text;
        let picker = {
            let colors = self.log_colors_mut(theme);
            colors.text_custom = false;
            colors.text.clone()
        };
        picker.update(cx, |state, cx| {
            state.set_value(fallback, window, cx);
        });
        Self::draft_changed(cx);
    }

    fn reset_log_background_color(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let theme = Self::current_log_color_theme(cx);
        let mode = match theme {
            LogColorTheme::Light => ThemeMode::Light,
            LogColorTheme::Dark => ThemeMode::Dark,
        };
        let fallback = crate::ui_theme::palette_for_mode(mode).log_background;
        let picker = {
            let colors = self.log_colors_mut(theme);
            colors.background_custom = false;
            colors.background.clone()
        };
        picker.update(cx, |state, cx| {
            state.set_value(fallback, window, cx);
        });
        Self::draft_changed(cx);
    }

    fn shortcut_value(&self, action: ShortcutAction, cx: &gpui_kit::App) -> String {
        self.shortcut_inputs[&action]
            .read(cx)
            .value()
            .trim()
            .to_string()
    }

    fn conflicts(&self, cx: &gpui_kit::App) -> HashMap<ShortcutAction, String> {
        let mut by_value: HashMap<String, Vec<ShortcutAction>> = HashMap::new();
        for action in ShortcutAction::all() {
            let value = self.shortcut_value(action, cx);
            if !value.is_empty() {
                by_value
                    .entry(value.to_ascii_lowercase())
                    .or_default()
                    .push(action);
            }
        }

        let mut conflicts = HashMap::new();
        for actions in by_value.into_values().filter(|actions| actions.len() > 1) {
            for action in &actions {
                let others = actions
                    .iter()
                    .filter(|other| *other != action)
                    .map(|other| other.label())
                    .collect::<Vec<_>>()
                    .join(crate::tr!("、", ", "));
                conflicts.insert(
                    *action,
                    crate::tr_args!("与“{others}”冲突", "Conflicts with “{others}”"),
                );
            }
        }
        conflicts
    }

    fn refresh_cache_info(&mut self, cx: &mut Context<Self>) {
        if self.cache_busy {
            return;
        }
        let Some(directory) = self.cache_dir.clone() else {
            self.cache_status = Some(
                crate::tr!(
                    "无法确定索引缓存目录。",
                    "Couldn’t determine the index-cache directory."
                )
                .into(),
            );
            return;
        };
        self.cache_busy = true;
        self.cache_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { vclogg_data::index_cache_info(directory) })
                .await;
            _ = this.update(cx, |this, cx| {
                this.cache_busy = false;
                this.cache_task = None;
                match result {
                    Ok(info) => this.cache_info = Some(info),
                    Err(error) => {
                        this.cache_status = Some(
                            crate::tr_args!(
                                "读取缓存信息失败：{error}",
                                "Couldn’t read cache information: {error}",
                            )
                            .into(),
                        )
                    }
                }
                cx.notify();
            });
        }));
    }

    fn clear_cache(&mut self, cx: &mut Context<Self>) {
        if self.cache_busy
            || self
                .cache_info
                .as_ref()
                .is_none_or(|info| info.file_count == 0)
        {
            return;
        }
        let Some(directory) = self.cache_dir.clone() else {
            return;
        };
        self.cache_busy = true;
        self.cache_status = Some(crate::tr!("正在清理缓存…", "Cleaning cache…").into());
        self.cache_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { vclogg_data::clear_index_cache(directory) })
                .await;
            _ = this.update(cx, |this, cx| {
                this.cache_busy = false;
                this.cache_task = None;
                match result {
                    Ok(result) => {
                        this.cache_info = Some(result.info);
                        this.cache_status = Some(
                            if result.removed_file_count == 0 {
                                crate::tr!(
                                    "当前没有可清理的缓存，正在使用或已变化的文件已保留。",
                                    "There is no cache to clean. Files in use or changed were kept.",
                                )
                                .to_string()
                            } else {
                                crate::tr_args!(
                                    "已清理 {} 个缓存文件，释放 {}。",
                                    "Cleaned {} cache files and freed {}.",
                                    result.removed_file_count,
                                    format_byte_size(result.removed_byte_size),
                                )
                            }
                            .into(),
                        );
                    }
                    Err(error) => {
                        this.cache_status = Some(
                            crate::tr_args!(
                                "清理缓存失败：{error}",
                                "Couldn’t clean the cache: {error}",
                            )
                            .into(),
                        )
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn open_cache_directory(&mut self, cx: &mut Context<Self>) {
        if self.cache_busy {
            return;
        }
        let Some(directory) = self.cache_dir.clone() else {
            self.cache_status = Some(
                crate::tr!(
                    "无法确定索引缓存目录。",
                    "Couldn’t determine the index-cache directory."
                )
                .into(),
            );
            return;
        };
        self.cache_busy = true;
        self.cache_task = Some(cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn({
                    let directory = directory.clone();
                    async move {
                        std::fs::create_dir_all(&directory)?;
                        Ok::<_, std::io::Error>(directory)
                    }
                })
                .await;
            _ = this.update(cx, |this, cx| {
                this.cache_busy = false;
                this.cache_task = None;
                match result {
                    Ok(directory) => {
                        cx.reveal_path(&directory);
                        this.cache_status =
                            Some(crate::tr!("已打开缓存文件夹。", "Cache folder opened.").into());
                    }
                    Err(error) => {
                        this.cache_status = Some(
                            crate::tr_args!(
                                "打开缓存文件夹失败：{error}",
                                "Couldn’t open the cache folder: {error}",
                            )
                            .into(),
                        )
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn clear_application_log(&mut self, cx: &mut Context<Self>) {
        if self.log_export_task.is_some() || self.log_clear_task.is_some() {
            return;
        }
        self.log_status = None;
        self.log_clear_task = Some(cx.spawn(async move |this, cx| {
            let result = cx.background_spawn(async { app_log::clear() }).await;
            _ = this.update(cx, |this, cx| {
                this.log_clear_task = None;
                this.log_status = Some(match result {
                    Ok(()) => crate::tr!(
                        "应用日志已清理；新日志将继续按当前等级记录。",
                        "Application logs cleared. New logs will continue at the current level."
                    )
                    .into(),
                    Err(error) => crate::tr_args!(
                        "应用日志未完全清理：{error}",
                        "Couldn’t clear all application logs: {error}"
                    )
                    .into(),
                });
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn export_application_log(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.log_export_task.is_some() || self.log_clear_task.is_some() {
            return;
        }
        let directory = dirs::document_dir().unwrap_or_else(|| PathBuf::from("."));
        let suggested_name = format!("vclogg2-{}.log", Local::now().format("%Y%m%d-%H%M%S"));
        let prompt = cx.prompt_for_new_path(&directory, Some(&suggested_name));
        self.log_export_task = Some(cx.spawn_in(window, async move |this, cx| {
            let selected_path = prompt.await;
            let result = match selected_path {
                Ok(Ok(Some(path))) => Some(
                    cx.background_spawn(async move {
                        let entry_count = app_log::export(&path)?;
                        Ok::<_, anyhow::Error>((path, entry_count))
                    })
                    .await,
                ),
                Ok(Ok(None)) => None,
                Ok(Err(error)) => Some(Err(error)),
                Err(error) => Some(Err(anyhow::anyhow!(error))),
            };
            _ = this.update_in(cx, |this, window, cx| {
                this.log_export_task = None;
                match result {
                    Some(Ok((path, entry_count))) => {
                        this.log_status = Some(
                            crate::tr_args!(
                                "已导出 {entry_count} 条应用日志到 {}",
                                "Exported {entry_count} application log entries to {}",
                                path.display(),
                            )
                            .into(),
                        );
                        window.notify_message(
                            crate::tr!("应用日志已导出", "Application log exported"),
                            cx,
                        );
                    }
                    Some(Err(error)) => {
                        this.log_status = Some(
                            crate::tr_args!(
                                "应用日志导出失败：{error}",
                                "Couldn’t export the application log: {error}",
                            )
                            .into(),
                        );
                    }
                    None => {}
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn render_shortcut_row(
        &self,
        action: ShortcutAction,
        conflict: Option<&str>,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let input = self.shortcut_inputs[&action].clone();
        let default_value = action.value(&ShortcutSettings::default()).to_string();
        let input_for_reset = input.clone();

        h_flex()
            .id(format!("settings-shortcut-row-{}", action.id()))
            .items_start()
            .justify_between()
            .gap_4()
            .px_3()
            .py_2()
            .child(
                v_flex()
                    .min_w_0()
                    .flex_1()
                    .gap_0p5()
                    .child(div().text_sm().child(action.label()))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(action.description()),
                    ),
            )
            .child(
                v_flex()
                    .w_56()
                    .gap_1()
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                div()
                                    .id(format!("settings-shortcut-input-{}", action.id()))
                                    .key_context(SHORTCUT_INPUT_CONTEXT)
                                    .flex_1()
                                    .child(Input::new(&input).small().readonly(true)),
                            )
                            .child(
                                Button::new(format!("settings-shortcut-reset-{}", action.id()))
                                    .small()
                                    .ghost()
                                    .icon(IconName::Undo2)
                                    .tooltip(crate::tr!(
                                        "恢复默认快捷键",
                                        "Restore default shortcut",
                                    ))
                                    .on_click(move |_, window, cx| {
                                        input_for_reset.update(cx, |state, cx| {
                                            state.set_value(default_value.clone(), window, cx)
                                        });
                                    }),
                            ),
                    )
                    .when_some(conflict, |this, conflict| {
                        this.child(
                            div()
                                .id(format!("settings-shortcut-conflict-{}", action.id()))
                                .text_xs()
                                .text_color(cx.theme().danger)
                                .child(conflict.to_string()),
                        )
                    }),
            )
            .on_mouse_down(gpui_kit::MouseButton::Left, move |_, window, cx| {
                input.focus_handle(cx).focus(window, cx);
            })
            .into_any_element()
    }

    pub(crate) fn set_file_history(
        &mut self,
        result: Result<Entity<HistoryDialog>, SharedString>,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(history) => {
                if let Some(category) = self.history_tab.file_category() {
                    history.update(cx, |this, cx| this.set_category(category, cx));
                }
                self.file_history = Some(history);
                self.file_history_error = None;
            }
            Err(error) => self.file_history_error = Some(error),
        }
        cx.notify();
    }

    fn render_history(&self, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .id("settings-history-section")
            .flex_1()
            .min_h_0()
            .gap_3()
            .child(
                h_flex()
                    .flex_none()
                    .flex_wrap()
                    .gap_2()
                    .p_1()
                    .rounded(cx.theme().radius)
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().group_box)
                    .children(
                        [
                            (
                                HistoryTab::Files,
                                "settings-file-history-tab",
                                crate::tr!("文件记录", "File entries"),
                            ),
                            (
                                HistoryTab::TemporaryResults,
                                "settings-temporary-history-tab",
                                crate::tr!("临时搜索结果", "Temporary search results"),
                            ),
                            (
                                HistoryTab::Search,
                                "settings-search-history-tab",
                                crate::tr!("搜索历史", "Search history"),
                            ),
                            (
                                HistoryTab::Cache,
                                "settings-cache-tab",
                                crate::tr!("索引缓存", "Index cache"),
                            ),
                        ]
                        .into_iter()
                        .map(|(tab, id, label)| {
                            let key = match tab {
                                HistoryTab::Files => "files",
                                HistoryTab::TemporaryResults => "temp",
                                HistoryTab::Cache => "cache",
                                HistoryTab::Search => "history",
                            };
                            let button = Button::new(id)
                                .small()
                                .ghost()
                                .label(label)
                                .selected(self.history_tab == tab)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.history_tab = tab;
                                    if let Some(category) = tab.file_category()
                                        && let Some(history) = &this.file_history
                                    {
                                        history
                                            .update(cx, |this, cx| this.set_category(category, cx));
                                    }
                                    cx.notify();
                                }));
                            self.location.mark(key, button, cx)
                        }),
                    ),
            )
            .child(if self.history_tab == HistoryTab::Cache {
                self.render_cache(cx)
            } else if self.history_tab == HistoryTab::Search {
                self.render_search_history_management(cx)
            } else if let Some(history) = &self.file_history {
                div()
                    .flex_1()
                    .min_h_0()
                    .child(history.clone())
                    .into_any_element()
            } else {
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        self.file_history_error.clone().unwrap_or_else(|| {
                            crate::tr!("正在读取历史…", "Loading history…").into()
                        }),
                    )
                    .into_any_element()
            })
            .into_any_element()
    }

    fn render_search_history_management(&self, cx: &mut Context<Self>) -> AnyElement {
        let filter = self
            .search_history_filter
            .read(cx)
            .value()
            .trim()
            .to_lowercase();
        let entries = self
            .search_history
            .iter()
            .filter(|query| filter.is_empty() || query.to_lowercase().contains(&filter))
            .cloned()
            .collect::<Vec<_>>();
        let visible_count = entries.len();
        let total_count = self.search_history.len();
        let entries = Rc::new(entries);
        let settings = cx.entity();
        let history_list = if entries.is_empty() {
            div()
                .h_full()
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(if total_count == 0 {
                    crate::tr!("暂无搜索历史", "No search history")
                } else {
                    crate::tr!(
                        "没有符合筛选条件的搜索历史",
                        "No search history matches the filter"
                    )
                })
                .into_any_element()
        } else {
            uniform_list(
                "settings-search-history-list",
                entries.len(),
                move |visible_range, _, cx| {
                    visible_range
                        .map(|ix| {
                            let query = entries[ix].clone();
                            let query_for_delete = query.clone();
                            let settings = settings.clone();
                            h_flex()
                                .id(format!("settings-search-history-row:{query}"))
                                .h(gpui_kit::rems(HISTORY_ROW_HEIGHT_REMS))
                                .w_full()
                                .min_w_0()
                                .flex_none()
                                .gap_3()
                                .px_5()
                                .hover(|row| row.bg(cx.theme().tokens.list_hover))
                                .child(
                                    div()
                                        .w_4()
                                        .flex_none()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .child(
                                            gpui_kit::component::Icon::new(IconName::Search)
                                                .size_4()
                                                .text_color(cx.theme().primary),
                                        ),
                                )
                                .child(
                                    div()
                                        .min_w_0()
                                        .flex_1()
                                        .truncate()
                                        .text_sm()
                                        .child(query.clone()),
                                )
                                .child(
                                    Button::new(format!("settings-search-history-delete:{query}"))
                                        .xsmall()
                                        .ghost()
                                        .danger()
                                        .label(crate::tr!("删除记录", "Delete entry"))
                                        .tooltip(query)
                                        .on_click(move |_, _, cx| {
                                            settings.update(cx, |this, cx| {
                                                this.remove_search_history(&query_for_delete, cx);
                                            });
                                        }),
                                )
                                .into_any_element()
                        })
                        .collect()
                },
            )
            .h_full()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .track_scroll(&self.search_history_scroll)
            .into_any_element()
        };

        v_flex()
            .id("settings-search-history-section")
            .flex_1()
            .min_h_0()
            .overflow_hidden()
            .gap_3()
            .child(
                div()
                    .w_full()
                    .flex_none()
                    .child(Input::new(&self.search_history_filter)),
            )
            .child(
                h_flex()
                    .flex_none()
                    .justify_between()
                    .gap_3()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(div().min_w_0().truncate().child(crate::tr_args!(
                        "显示 {visible_count} / {total_count} 条记录",
                        "Showing {visible_count} of {total_count} entries",
                    )))
                    .child(
                        h_flex()
                            .flex_none()
                            .gap_2()
                            .child(crate::tr!(
                                "删除后自动保存",
                                "Deletions are saved automatically"
                            ))
                            .child(
                                Button::new("settings-search-history-clear")
                                    .small()
                                    .ghost()
                                    .label(crate::tr!("清除历史", "Clear history"))
                                    .disabled(total_count == 0)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.confirm_clear_search_history(window, cx);
                                    })),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .id("settings-search-history-list-container")
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .rounded(cx.theme().radius)
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().group_box)
                    .overflow_hidden()
                    .child(history_list)
                    .child(persistent_list_scrollbar(
                        "settings-search-history-list-scrollbar",
                        &self.search_history_scroll,
                    )),
            )
            .into_any_element()
    }

    fn render_icon_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let icons = RadioGroup::horizontal("settings-application-icon")
            .selected_index(Some(self.draft.app_icon.index()))
            .children(AppIcon::ALL.into_iter().map(|icon| {
                Radio::new((
                    gpui_kit::ElementId::from("application-icon"),
                    icon.storage_value(),
                ))
                .label(icon.label())
                .small()
                .flex_1()
                .min_w_0()
                .p_2()
                .border_1()
                .border_color(if self.draft.app_icon == icon {
                    cx.theme().primary
                } else {
                    cx.theme().border
                })
                .child(img(icon.image()).size_12().object_fit(ObjectFit::Contain))
            }))
            .on_click(cx.listener(|this, ix, _, cx| {
                if let Some(icon) = AppIcon::ALL.get(*ix) {
                    this.draft.app_icon = *icon;
                    Self::draft_changed(cx);
                }
            }));

        section(
            crate::tr!("应用图标", "Application icon"),
            Some(crate::tr!(
                "选择后立即生效，下次启动继续使用。",
                "Your choice applies immediately and is kept for the next launch.",
            ).to_owned()),
            [div()
                .px_3()
                .py_2()
                .child(
                    v_flex().gap_2().child(icons).child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(crate::tr!(
                                "更改运行中的应用图标；安装包、文件管理器和已固定快捷方式仍使用默认图标。Linux 桌面需运行安装器，显示效果取决于桌面环境。",
                                "Changes the running app icon. Packages, file managers, and pinned shortcuts keep the default. Linux desktop icons require the installer and depend on the desktop environment.",
                            )),
                    ),
                )
                .into_any_element()],
            cx,
        )
        .id("settings-application-icon-section")
        .into_any_element()
    }

    fn render_about(&self, cx: &mut Context<Self>) -> AnyElement {
        let build_time = local_build_time();
        v_flex()
            .id("settings-about-page")
            .gap_6()
            .pb_4()
            .child(
                h_flex()
                    .items_center()
                    .gap_4()
                    .child(
                        div()
                            .size_16()
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(cx.theme().radius)
                            .border_1()
                            .border_color(cx.theme().border)
                            .bg(cx.theme().muted)
                            .child(
                                img(self.draft.app_icon.image())
                                    .size_12()
                                    .object_fit(ObjectFit::Contain),
                            ),
                    )
                    .child(
                        v_flex()
                            .min_w_0()
                            .gap_1()
                            .child(
                                div()
                                    .text_lg()
                                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                    .child("VCLogg2"),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(crate::tr!(
                                        "高性能原生日志查看、检索与分析工具",
                                        "High-performance native log viewing, search, and analysis",
                                    )),
                            ),
                    ),
            )
            .child(self.location.mark("build", about_section(
                "settings-about-build",
                crate::tr!("版本与构建", "Version & build"),
                crate::tr!(
                    "这些信息来自当前可执行文件，可用于问题定位与版本核对。",
                    "This information comes from the current executable and can help diagnose issues and verify versions.",
                ),
                DescriptionList::new()
                    .small()
                    .columns(1)
                    .item(crate::tr!("应用版本", "Application version"), crate::build_info::DISPLAY_VERSION, 1)
                    .item(crate::tr!("编译 Commit", "Build commit"), env!("VCLOGG2_BUILD_COMMIT"), 1)
                    .item(crate::tr!("编译时间", "Build time"), build_time, 1)
                    .item(crate::tr!("构建目标", "Build target"), env!("VCLOGG2_BUILD_TARGET"), 1)
                    .item(crate::tr!("构建配置", "Build profile"), env!("VCLOGG2_BUILD_PROFILE"), 1),
                cx,
            ), cx))
            .child(self.location.mark("stack", about_section(
                "settings-about-stack",
                crate::tr!("技术栈", "Technology"),
                crate::tr!(
                    "采用 Rust 原生桌面架构，界面、日志索引与搜索核心均在本地运行。",
                    "A native Rust desktop architecture runs the interface, log indexing, and search locally.",
                ),
                DescriptionList::new()
                    .small()
                    .columns(1)
                    .item(crate::tr!("语言与工具链", "Language & toolchain"), "Rust 2024 Edition · Cargo · native toolchain", 1)
                    .item(crate::tr!("界面", "Interface"), "GPUI Kit", 1)
                    .item(crate::tr!("数据与检索", "Data & search"), crate::tr!("SQLite/WAL · 内存映射 · 正则与多模式匹配", "SQLite/WAL · memory mapping · regex and multi-pattern matching"), 1)
                    .item(crate::tr!("网络", "Networking"), "HTTP/HTTPS · rustls", 1)
                    .item(crate::tr!("平台集成", "Platform integration"), crate::tr!("系统回收站、凭据库、文件打开关联、单实例与原生窗口", "System trash, credential storage, file-opening associations, single-instance routing, and native windows"), 1),
                cx,
            ), cx))
            .child(self.location.mark("libs", about_section(
                "settings-about-libraries",
                crate::tr!("主要开源库", "Key open-source libraries"),
                crate::tr!(
                    "以下为应用直接使用的核心开源组件；各组件继续遵循其自身许可证。",
                    "These core open-source components are used directly by the application and remain subject to their own licenses.",
                ),
                DescriptionList::new()
                    .small()
                    .columns(1)
                    .item(
                        "GPUI",
                        crate::tr!("GPU 加速的原生桌面 UI 框架，由 Zed Industries 开源", "GPU-accelerated native desktop UI framework open-sourced by Zed Industries"),
                        1,
                    )
                    .item(
                        "gpui-kit",
                        crate::tr!("主题、桌面控件、虚拟列表、表格、弹层与交互基础", "Themes, desktop controls, virtual lists, tables, overlays, and interaction foundations"),
                        1,
                    )
                    .item("rusqlite / SQLite", crate::tr!("设置、历史与会话状态的本地持久化", "Local persistence for settings, history, and session state"), 1)
                    .item(
                        "reqwest / rustls",
                        crate::tr!("云端过滤器所需的 HTTPS 通信", "HTTPS communication for cloud filters"),
                        1,
                    )
                    .item("serde / serde_json", crate::tr!("配置、会话与交换格式的序列化", "Serialization for configuration, sessions, and exchange formats"), 1)
                    .item(
                        "memmap2 / encoding_rs / chardetng",
                        crate::tr!("大文件内存映射、字符编码检测与按需解码", "Memory mapping, encoding detection, and on-demand decoding for large files"),
                        1,
                    )
                    .item(
                        "regex / aho-corasick / roaring",
                        crate::tr!("正则搜索、多关键词匹配与压缩结果集合", "Regex search, multi-keyword matching, and compressed result sets"),
                        1,
                    )
                    .item(
                        "sha2",
                        crate::tr!("安全摘要", "Secure digests"),
                        1,
                    ),
                cx,
            ), cx))
            .child(self.location.mark("repo", about_section(
                "settings-about-repository",
                crate::tr!("GitHub 仓库", "GitHub repository"),
                crate::tr!(
                    "访问项目源代码、问题追踪与发布版本。",
                    "Visit the source code, issue tracker, and published releases.",
                ),
                DescriptionList::new().small().columns(1).item(
                    crate::tr!("仓库地址", "Repository URL"),
                    layout::external_link(
                        "settings-about-github-link",
                        GITHUB_REPOSITORY_URL,
                        GITHUB_REPOSITORY_URL,
                        cx,
                    )
                        .accessibility_label(crate::tr!(
                            "在浏览器中打开 VCLogg2 GitHub 仓库",
                            "Open the VCLogg2 GitHub repository in a browser",
                        ))
                        .into_any_element(),
                    1,
                ),
                cx,
            ), cx))
            .child(self.location.mark("license", about_section(
                "settings-about-credits",
                crate::tr!("作者与许可", "Author & licenses"),
                crate::tr!("感谢所有开源项目维护者与贡献者。", "Thanks to all open-source maintainers and contributors."),
                DescriptionList::new()
                    .small()
                    .columns(1)
                    .item(crate::tr!("项目作者", "Author"), "zhaiyanqi", 1)
                    .item(
                        crate::tr!("项目许可", "Project license"),
                        "Apache License 2.0",
                        1,
                    )
                    .item("Copyright", "Copyright © 2026 zhaiyanqi", 1)
                    .item(
                        crate::tr!("第三方声明", "Third-party notices"),
                        crate::tr!("第三方库、字体与图标的版权及许可归各自权利人所有", "Third-party libraries, fonts, and icons remain subject to their respective copyrights and licenses"),
                        1,
                    ),
                cx,
            ), cx))
            .into_any_element()
    }
}

fn about_section(
    id: &'static str,
    title: &'static str,
    description: &'static str,
    content: impl IntoElement,
    cx: &mut Context<SettingsDialog>,
) -> AnyElement {
    section(
        title,
        Some(description.to_owned()),
        [div().px_3().py_2().child(content).into_any_element()],
        cx,
    )
    .id(id)
    .into_any_element()
}

impl SettingsDialog {
    fn render_general(&self, cx: &mut Context<Self>) -> AnyElement {
        let languages = RadioGroup::horizontal("settings-language")
            .selected_index(Some(self.draft.language.select_index()))
            .children(Language::ALL.into_iter().map(|language| {
                Radio::new(format!("settings-language-{}", language.database_value()))
                    .small()
                    .label(language.native_name())
            }))
            .on_click(cx.listener(|this, selected_index: &usize, _, cx| {
                if let Some(language) = Language::ALL.get(*selected_index).copied() {
                    this.draft.language = language;
                    SettingsDialog::draft_changed(cx);
                }
            }));
        let themes = RadioGroup::vertical("settings-theme-preference")
            .selected_index(Some(self.draft.theme_preference.select_index()))
            .children(ThemePreference::ALL.into_iter().map(|preference| {
                Radio::new(format!("settings-theme-{}", preference.database_value()))
                    .small()
                    .label(theme_preference_label(preference))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(theme_preference_description(preference)),
                    )
            }))
            .on_click(cx.listener(|this, selected_index: &usize, _, cx| {
                if let Some(preference) = ThemePreference::ALL.get(*selected_index).copied() {
                    this.draft.theme_preference = preference;
                    SettingsDialog::draft_changed(cx);
                }
            }));
        let motion = Switch::new("settings-reduce-motion")
            .small()
            .checked(self.draft.reduce_motion)
            .tooltip(crate::tr!("减少动态效果", "Reduce motion"))
            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                this.draft.reduce_motion = *checked;
                SettingsDialog::draft_changed(cx);
            }));
        let full_path = Switch::new("settings-show-full-path")
            .small()
            .checked(self.draft.show_full_path)
            .tooltip(crate::tr!("显示完整路径", "Show full path"))
            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                this.draft.show_full_path = *checked;
                SettingsDialog::draft_changed(cx);
            }));
        let close_tab = Switch::new("settings-confirm-close-tab")
            .small()
            .checked(self.draft.confirm_close_tab)
            .tooltip(crate::tr!(
                "关闭日志标签前确认",
                "Confirm before closing log tabs"
            ))
            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                this.draft.confirm_close_tab = *checked;
                SettingsDialog::draft_changed(cx);
            }));
        let paste = Switch::new("settings-confirm-clipboard-paste")
            .small()
            .checked(self.draft.confirm_clipboard_paste)
            .tooltip(crate::tr!("粘贴时确认", "Confirm clipboard paste"))
            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                this.draft.confirm_clipboard_paste = *checked;
                SettingsDialog::draft_changed(cx);
            }));

        let content = v_flex()
            .gap_6()
            .pb_4()
            .child(
                section(
                    crate::tr!("语言", "Language"),
                    Some(
                        (if cfg!(target_os = "macos") {
                            crate::tr!(
                                "应用界面立即切换，系统窗口菜单在下次启动时切换。",
                                "The app language changes immediately. Native window menus change on the next launch.",
                            )
                        } else {
                            crate::tr!(
                                "选择应用界面使用的语言。",
                                "Choose the language used by the application interface.",
                            )
                        })
                        .to_string(),
                    ),
                    [div()
                        .px_3()
                        .py_2()
                        .child(self.location.mark("language", languages, cx))
                        .into_any_element()],
                    cx,
                )
                .id("settings-language-section"),
            )
            .child(
                section(
                    crate::tr!("界面主题", "Interface theme"),
                    None,
                    [
                        div()
                            .px_3()
                            .py_2()
                            .child(self.location.mark("theme", themes, cx))
                            .into_any_element(),
                        self.setting_row("motion", crate::tr!("减少动态效果", "Reduce motion"), None, motion, cx),
                    ],
                    cx,
                )
                .id("settings-theme-section"),
            )
            .child(
                self.location
                    .mark("icon", self.render_icon_settings(cx), cx),
            )
            .child(
                section(
                    crate::tr!("文件与标签", "Files & tabs"),
                    None,
                    [
                        self.setting_row("full-path", crate::tr!("在文件工具栏显示完整路径", "Show full path in the file toolbar"), None, full_path, cx),
                        self.setting_row("close-tab", crate::tr!("关闭日志标签前确认", "Confirm before closing log tabs"), None, close_tab, cx),
                        self.setting_row("paste", crate::tr!("粘贴时确认", "Confirm clipboard paste"), None, paste, cx),
                        self.setting_row(
                            "directory",
                            crate::tr!("打开目录命令", "Open-folder command"),
                            Some(crate::tr!(
                                "留空使用系统默认程序；支持 {directory} 与 {path} 占位符，未使用时自动追加目录。",
                                "Leave empty to use the system default. Supports {directory} and {path}; the folder is appended when neither placeholder is used.",
                            )),
                            div()
                                .id("settings-open-directory-command")
                                .w_48()
                                .child(Input::new(&self.open_directory_command).w_full()),
                            cx,
                        ),
                    ],
                    cx,
                )
                .id("settings-files-section"),
            );
        #[cfg(windows)]
        let content = content.child(self.location.mark(
            "explorer",
            self.windows_context_menu.clone(),
            cx,
        ));
        content.into_any_element()
    }

    fn render_search_options(&self, cx: &mut Context<Self>) -> AnyElement {
        let case = Switch::new("settings-default-case-sensitive")
            .small()
            .checked(self.draft.default_case_sensitive)
            .tooltip(crate::tr!("区分大小写", "Case-sensitive"))
            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                this.draft.default_case_sensitive = *checked;
                SettingsDialog::draft_changed(cx);
            }));
        let regex = Switch::new("settings-default-use-regex")
            .small()
            .checked(self.draft.default_use_regex)
            .tooltip(crate::tr!("使用正则表达式", "Use regular expressions"))
            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                this.draft.default_use_regex = *checked;
                SettingsDialog::draft_changed(cx);
            }));
        let highlights = Switch::new("settings-highlight-matches")
            .small()
            .checked(self.draft.highlight_matches)
            .tooltip(crate::tr!(
                "高亮已提交搜索的匹配文字",
                "Highlight matches from submitted searches",
            ))
            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                this.draft.highlight_matches = *checked;
                SettingsDialog::draft_changed(cx);
            }));
        let styles = Button::new("settings-open-match-styles")
            .small()
            .ghost()
            .label(crate::tr!("匹配样式…", "Match styles…"))
            .on_click(cx.listener(|this, _, window, cx| {
                this.select_category(SettingsCategory::Highlight, window, cx);
                if let Some(editor) = &this.highlight_editor {
                    editor.update(cx, |editor, cx| editor.show_match_styles(cx));
                }
            }));

        v_flex()
            .gap_6()
            .pb_4()
            .child(
                section(
                    crate::tr!("查询匹配", "Query matching"),
                    None,
                    [
                        self.setting_row("case", crate::tr!("区分大小写", "Case-sensitive"), None, case, cx),
                        self.setting_row("regex", crate::tr!("使用正则表达式", "Use regular expressions"), None, regex, cx),
                    ],
                    cx,
                )
                .id("settings-search-section"),
            )
            .child(section(
                crate::tr!("结果与高亮", "Results and highlighting"),
                None,
                [
                    self.setting_row(
                        "limit",
                        crate::tr!("每个文件的结果上限", "Result limit per file"),
                        Some(crate::tr!(
                            "设为 0 表示不限制；每个参与文件独立应用上限。",
                            "Set to 0 for no limit. The limit is applied separately to each participating file.",
                        )),
                        NumberInput::new(&self.max_search_results).small().w_48(),
                        cx,
                    ),
                    self.setting_row("hits", crate::tr!(
                        "高亮已提交搜索的匹配文字",
                        "Highlight matches from submitted searches",
                    ), None, highlights, cx),
                    div()
                        .px_3()
                        .py_2()
                        .child(h_flex().justify_end().child(styles))
                        .into_any_element(),
                ],
                cx,
            ))
            .child(self.render_search_size_settings(cx))
            .into_any_element()
    }

    fn render_network(&self, cx: &mut Context<Self>) -> AnyElement {
        let network_busy = self.network_task.is_some();
        let network_status_color = match self.network_status_kind {
            NetworkStatusKind::Neutral => cx.theme().muted_foreground,
            NetworkStatusKind::Success => cx.theme().success,
            NetworkStatusKind::Error => cx.theme().danger,
        };
        let address = Input::new(&self.network_server_url)
            .id("settings-network-server")
            .w_48()
            .content_type(InputContentType::Url)
            .accessibility_id("A391")
            .aria_label(crate::tr!("云端服务器地址", "Cloud server address"))
            .disabled(network_busy);
        let name = Input::new(&self.network_display_name)
            .w_48()
            .content_type(InputContentType::Username)
            .accessibility_id("A392")
            .aria_label(crate::tr!("云端用户名", "Cloud user name"))
            .disabled(network_busy);
        let test_connection = Button::new("settings-network-connect")
            .small()
            .primary()
            .label(crate::tr!("测试连接", "Test connection"))
            .loading(network_busy)
            .disabled(network_busy)
            .accessibility_id("A393")
            .on_click(cx.listener(|this, _, window, cx| this.connect_network(window, cx)));

        section(
            crate::tr!("远程服务", "Remote service"),
            Some(crate::tr!(
                "地址和用户名自动保存。测试连接会联系服务器并获取会话 Cookie。",
                "The address and user name are saved automatically. Testing contacts the server and obtains a session cookie.",
            ).to_owned()),
            [
                self.setting_row("server", crate::tr!("服务器地址", "Server address"), Some(crate::tr!(
                    "支持 HTTP 或 HTTPS；建议远程服务使用 HTTPS。",
                    "Supports HTTP or HTTPS. HTTPS is recommended for remote services.",
                )), address, cx),
                self.setting_row("name", crate::tr!("用户名", "User name"), Some(crate::tr!(
                    "可填写工号或昵称，不校验身份真实性，最多 64 个字符。",
                    "Enter an employee ID or nickname, up to 64 characters. Identity is not verified.",
                )), name, cx),
                div()
                    .px_3()
                    .py_2()
                    .child(
                        h_flex()
                            .gap_3()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .id("settings-network-status")
                                    .text_sm()
                                    .text_color(network_status_color)
                                    .child(self.network_status.clone()),
                            )
                            .child(self.location.mark("connection", test_connection, cx)),
                    )
                    .into_any_element(),
            ],
            cx,
        )
        .id("settings-network-section")
        .into_any_element()
    }

    fn render_cache(&self, cx: &mut Context<Self>) -> AnyElement {
        let cache_available = self.cache_dir.is_some();
        let cache_summary = self.cache_info.as_ref().map_or_else(
            || {
                if cache_available {
                    crate::tr!("正在读取缓存大小…", "Reading cache size…").to_string()
                } else {
                    crate::tr!("缓存不可用", "Cache unavailable").to_string()
                }
            },
            |info| {
                crate::tr_args!(
                    "{} · {} 个文件",
                    "{} · {} files",
                    format_byte_size(info.byte_size),
                    info.file_count,
                )
            },
        );
        let cache_empty = self
            .cache_info
            .as_ref()
            .is_none_or(|info| info.file_count == 0);
        let cache_path = self.cache_dir.as_deref().map_or_else(
            || crate::tr!("不可用", "Unavailable").to_string(),
            |path| path.display().to_string(),
        );
        let open_folder = Button::new("settings-open-index-cache")
            .small()
            .ghost()
            .label(crate::tr!("打开文件夹", "Open folder"))
            .disabled(self.cache_busy || !cache_available)
            .on_click(cx.listener(|this, _, _, cx| this.open_cache_directory(cx)));
        let clean_cache = Button::new("settings-clear-index-cache")
            .small()
            .ghost()
            .label(crate::tr!("清理缓存", "Clean cache"))
            .loading(self.cache_busy)
            .disabled(self.cache_busy || cache_empty || !cache_available)
            .on_click(cx.listener(|this, _, _, cx| this.clear_cache(cx)));

        section(
            crate::tr!("索引缓存", "Index cache"),
            Some(crate::tr!(
                "用于加快再次打开同一代文件；清理后会在需要时安全重建。",
                "Speeds up reopening the same file generation and is rebuilt safely when needed after cleanup.",
            ).to_owned()),
            [div()
                .px_3()
                .py_2()
                .child(
                    v_flex()
                        .gap_2()
                        .child(
                            h_flex()
                                .justify_between()
                                .gap_4()
                                .child(
                                    v_flex().min_w_0().gap_0p5().child(cache_summary).child(
                                        div()
                                            .truncate()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(cache_path),
                                    ),
                                )
                                .child(h_flex().gap_2().child(open_folder).child(clean_cache)),
                        )
                        .when_some(self.cache_status.clone(), |this, status| {
                            this.child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(status),
                            )
                        }),
                )
                .into_any_element()],
            cx,
        )
        .id("settings-index-cache-section")
        .into_any_element()
    }

    fn render_diagnostics(&self, cx: &mut Context<Self>) -> AnyElement {
        let log_entries = app_log::entry_count();
        let clear = Button::new("settings-clear-application-log")
            .small()
            .outline()
            .label(crate::tr!("清理日志", "Clear logs"))
            .tooltip(crate::tr!(
                "清理应用诊断缓冲和磁盘记录；不删除已导出的文件。",
                "Clear buffered and stored application diagnostics. Exported files are kept.",
            ))
            .loading(self.log_clear_task.is_some())
            .disabled(self.log_export_task.is_some() || self.log_clear_task.is_some())
            .on_click(cx.listener(|this, _, _, cx| this.clear_application_log(cx)));
        let export = Button::new("settings-export-application-log")
            .small()
            .outline()
            .label(crate::tr!("导出日志…", "Export log…"))
            .loading(self.log_export_task.is_some())
            .disabled(self.log_export_task.is_some() || self.log_clear_task.is_some())
            .on_click(cx.listener(|this, _, window, cx| this.export_application_log(window, cx)));

        let level = self.setting_row(
            "app-log",
            crate::tr!("最低记录等级", "Minimum log level"),
            Some(crate::tr!(
                "按所选最低等级打印并保留最近的诊断记录；关闭后不打印也不收集新日志。",
                "Print and retain recent diagnostics at or above the selected level. Off stops both printing and collection.",
            )),
            div().w_48().child(Select::new(&self.app_log_level).small()), cx,
        );
        let count = crate::tr_args!(
            "当前缓冲区有 {log_entries} 条日志",
            "{log_entries} log entries are currently buffered",
        );
        let actions = v_flex()
            .px_3()
            .py_2()
            .gap_1()
            .child(
                h_flex()
                    .gap_3()
                    .justify_between()
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(count),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .flex_none()
                            .child(self.location.mark("log-clear", clear, cx))
                            .child(self.location.mark("log-export", export, cx)),
                    ),
            )
            .when_some(self.log_status.clone(), |row, status| {
                row.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(status),
                )
            });
        v_flex()
            .gap_6()
            .pb_4()
            .child(
                section(
                    crate::tr!("应用日志", "Application log"),
                    None,
                    [level, actions.into_any_element()],
                    cx,
                )
                .id("settings-application-log-section"),
            )
            .child(self.render_performance_settings(cx))
            .child(self.location.mark("trace", self.trace_recorder.clone(), cx))
            .into_any_element()
    }

    fn render_shortcuts(&self, cx: &mut Context<Self>) -> AnyElement {
        let conflicts = self.conflicts(cx);
        v_flex()
            .id("settings-shortcut-section")
            .w_full()
            .flex_1()
            .min_h_0()
            .gap_2()
            .child(
                v_flex()
                    .gap_0p5()
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(crate::tr!("聚焦输入框后直接按下组合键；按 Backspace 或 Delete 可清除绑定。", "Focus a field and press the key combination. Press Backspace or Delete to clear a binding.")),
                    ),
            )
            .child(
                v_flex()
                    .id("settings-shortcut-list")
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .when(!self.search_hits.is_empty(), |list| list.track_scroll(&self.detail_scroll))
                    .gap_6().pb_4()
                    .children(ShortcutAction::groups().into_iter().map(|(title, actions)| {
                        let rows = actions.iter().map(|&action| {
                            self.location.mark(&format!("key-{}", action.id()), self.render_shortcut_row(action, conflicts.get(&action).map(String::as_str), cx), cx)
                        }).collect::<Vec<_>>();
                        section(title, None, rows, cx)
                    })),
            ).into_any_element()
    }
}
impl Render for SettingsDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _performance_scope = crate::ui_performance::scope("SettingsDialog::render");
        let active_category = self.active_category;
        let settings_entity = cx.entity();
        let settings_query = self.settings_search.read(cx).value().trim().to_lowercase();
        let searching = !settings_query.is_empty();
        let has_matches = !searching || !self.search_hits.is_empty();
        let category_menu = if searching {
            SidebarMenu::new().children(self.search_hits.iter().copied().map(|hit| {
                let settings = settings_entity.clone();
                SidebarMenuItem::new(hit.label())
                    .icon(hit.category.icon())
                    .accessibility_label(format!("{} · {}", hit.category.label(), hit.label()))
                    .active(self.location.is(hit.key))
                    .on_click(move |_, window, cx| {
                        settings.update(cx, |this, cx| this.locate_option(hit, window, cx))
                    })
            }))
        } else {
            SidebarMenu::new().children(
                SettingsCategory::ALL
                    .into_iter()
                    .filter(|category| category.is_available())
                    .map(|category| {
                        let settings = settings_entity.clone();
                        SidebarMenuItem::new(category.label())
                            .icon(category.icon())
                            .active(category == active_category)
                            .on_click(move |_, window, cx| {
                                settings.update(cx, |this, cx| {
                                    this.select_category(category, window, cx)
                                })
                            })
                    }),
            )
        };
        if !matches!(
            active_category,
            SettingsCategory::Highlight | SettingsCategory::History
        ) {
            self.location.reveal(window, cx);
        }

        let detail = if has_matches {
            match active_category {
                SettingsCategory::General => self.render_general(cx),
                SettingsCategory::Reading => self.render_reading(cx),
                SettingsCategory::Highlight => v_flex()
                    .flex_1()
                    .min_h_0()
                    .children(self.highlight_editor.clone())
                    .into_any_element(),
                SettingsCategory::Search => self.render_search_options(cx),
                SettingsCategory::Shortcuts => self.render_shortcuts(cx),
                SettingsCategory::History => self.render_history(cx),
                SettingsCategory::Network => self.render_network(cx),
                SettingsCategory::Diagnostics => self.render_diagnostics(cx),
                SettingsCategory::About => self.render_about(cx),
            }
        } else {
            div()
                .text_color(cx.theme().muted_foreground)
                .child(crate::tr!("没有匹配的设置选项", "No matching settings"))
                .into_any_element()
        };
        let owns_scroll = matches!(
            active_category,
            SettingsCategory::Shortcuts | SettingsCategory::Highlight | SettingsCategory::History
        );
        let details = v_flex()
            .id("settings-category-details")
            .flex_1()
            .min_h_0()
            .px_4()
            .pb_3()
            .child(detail);
        let details = if owns_scroll {
            details.into_any_element()
        } else if searching {
            details
                .overflow_y_scroll()
                .track_scroll(&self.detail_scroll)
                .vertical_scrollbar(&self.detail_scroll)
                .into_any_element()
        } else {
            details
                .overflow_y_scrollbar()
                .id(format!("settings-page-{}", active_category.storage_value()))
                .into_any_element()
        };
        h_flex()
            .id("settings-dialog-content")
            .w_full()
            .h_full()
            .min_h_0()
            .items_stretch()
            .overflow_hidden()
            .child(
                Sidebar::new("settings-category-sidebar")
                    .w_48()
                    .bg(cx.theme().background)
                    .collapsible(false)
                    .collapsed(false)
                    .header(
                        Input::new(&self.settings_search)
                            .small()
                            .prefix(IconName::Search),
                    )
                    .child(category_menu),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .bg(cx.theme().background)
                    .gap_3()
                    .when(has_matches, |pane| {
                        pane.child(
                            div()
                                .px_4()
                                .pt_3()
                                .text_lg()
                                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                .child(active_category.label()),
                        )
                    })
                    .when(has_matches && active_category.shows_log_preview(), |pane| {
                        pane.child(self.render_log_preview(window, cx))
                    })
                    .child(details)
                    .when(has_matches && active_category == SettingsCategory::Network, |pane| {
                        pane.child(
                            h_flex().flex_none().justify_end().px_4().child(layout::help_link(
                                "A830",
                                crate::tr!(
                                    "https://zhaiyanqi.github.io/vclogg2/wiki/zh-CN/Cloud-Filters.html#connect",
                                    "https://zhaiyanqi.github.io/vclogg2/wiki/en/Cloud-Filters.html#connect",
                                ),
                                crate::tr!("在浏览器中查看云端连接指南", "Open the cloud connection guide in a browser"),
                                cx,
                            )),
                        )
                    }),
            )
    }
}

fn local_build_time() -> String {
    env!("VCLOGG2_BUILD_UNIX_TIMESTAMP")
        .parse::<i64>()
        .ok()
        .and_then(|timestamp| DateTime::<Utc>::from_timestamp(timestamp, 0))
        .map(|timestamp| {
            timestamp
                .with_timezone(&Local)
                .format("%Y-%m-%d %H:%M:%S %:z")
                .to_string()
        })
        .unwrap_or_else(|| crate::tr!("未知", "Unknown").to_string())
}

fn format_byte_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024. && unit + 1 < UNITS.len() {
        value /= 1024.;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn shortcut_from_keystroke(keystroke: &Keystroke) -> String {
    let mut parts = Vec::with_capacity(5);
    if keystroke.modifiers.control {
        parts.push("Ctrl".to_string());
    }
    if keystroke.modifiers.alt {
        parts.push("Alt".to_string());
    }
    if keystroke.modifiers.platform {
        parts.push(
            if cfg!(target_os = "macos") {
                "Cmd"
            } else {
                "Meta"
            }
            .to_string(),
        );
    }
    if keystroke.modifiers.shift {
        parts.push("Shift".to_string());
    }

    let key = match keystroke.key.as_str() {
        "enter" => "Enter".to_string(),
        "space" => "Space".to_string(),
        "arrowup" | "up" => "Up".to_string(),
        "arrowdown" | "down" => "Down".to_string(),
        "arrowleft" | "left" => "Left".to_string(),
        "arrowright" | "right" => "Right".to_string(),
        key if key.len() == 1 => key.to_ascii_uppercase(),
        key => {
            let mut chars = key.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
                .unwrap_or_default()
        }
    };
    parts.push(key);
    parts.join("+")
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::TestAppContext;

    #[gpui_kit::test]
    fn opening_general_defers_highlight_and_preserves_font_while_loading(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::component::init(cx);
            cx.set_global(SystemFonts::new(std::sync::Arc::new(
                gpui_kit::NoopTextSystem,
            )));
        });
        let saved = AppSettings {
            log_font_family: LogFontFamily::Named("Missing saved font".into()),
            ..AppSettings::default()
        };
        let baseline = crate::color_labels_dialog::LogColoringConfig {
            highlight_log_levels: saved.highlight_log_levels,
            log_coloring: saved.log_coloring.clone(),
            labels: crate::color_labels::default_color_labels(),
            selection_styles: saved.selection_styles.clone(),
            keyword_match_styles: saved.keyword_match_styles.clone(),
        };
        let (dialog, cx) = cx.add_window_view(|window, cx| {
            SettingsDialog::new(
                saved.clone(),
                Vec::new(),
                SettingsNetworkSnapshot {
                    settings: CloudSettings::default(),
                    client: None,
                    connection: None,
                    client_error: None,
                },
                SettingsCategory::General,
                window,
                cx,
            )
            .with_highlight_baseline(baseline, window, cx)
        });
        dialog.read_with(cx, |dialog, cx| {
            assert!(dialog.highlight_editor().is_none());
            assert_eq!(
                dialog.settings(cx).unwrap().log_font_family,
                saved.log_font_family
            );
        });
        cx.update(|window, cx| {
            dialog.update(cx, |dialog, cx| {
                dialog.ensure_highlight_editor(window, cx);
                let editor = dialog.highlight_editor().unwrap();
                dialog.ensure_highlight_editor(window, cx);
                assert_eq!(
                    editor.entity_id(),
                    dialog.highlight_editor().unwrap().entity_id()
                );
                assert!(editor.read(cx).config(cx).is_ok());
            });
        });
        cx.run_until_parked();
        dialog.read_with(cx, |dialog, cx| {
            assert!(dialog.font_loading_task.is_none());
            assert_eq!(
                dialog.settings(cx).unwrap().log_font_family,
                saved.log_font_family
            );
        });
    }
    #[gpui_kit::test]
    fn invalid_fields_keep_the_last_valid_value_without_blocking_other_settings(
        cx: &mut TestAppContext,
    ) {
        cx.update(|cx| {
            gpui_kit::component::init(cx);
            cx.set_global(SystemFonts::new(std::sync::Arc::new(
                gpui_kit::NoopTextSystem,
            )));
        });
        let previous = AppSettings {
            max_search_results: 123,
            ..AppSettings::default()
        };
        let (dialog, cx) = cx.add_window_view(|window, cx| {
            SettingsDialog::new(
                previous.clone(),
                Vec::new(),
                SettingsNetworkSnapshot {
                    settings: CloudSettings::default(),
                    client: None,
                    connection: None,
                    client_error: None,
                },
                SettingsCategory::General,
                window,
                cx,
            )
        });
        cx.update(|window, cx| {
            dialog.update(cx, |dialog, cx| {
                dialog
                    .max_search_results
                    .update(cx, |input, cx| input.set_value("invalid", window, cx));
                dialog.draft.show_full_path = !previous.show_full_path;
                let (next, error) = dialog.settings_for_autosave(&previous, cx);
                assert!(error.is_some());
                assert_eq!(next.max_search_results, 123);
                assert_eq!(next.show_full_path, !previous.show_full_path);
                assert!(dialog.settings(cx).is_err());
                dialog
                    .max_search_results
                    .update(cx, |input, cx| input.set_value("456", window, cx));
                let (next, error) = dialog.settings_for_autosave(&next, cx);
                assert!(error.is_none());
                assert_eq!(next.max_search_results, 456);
            })
        });
    }
}
