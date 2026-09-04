use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    rc::Rc,
    sync::{Arc, OnceLock},
};

use chrono::{DateTime, Local, Utc};
use gpui::{
    AnyElement, AppContext as _, Context, Entity, EventEmitter, Focusable as _, Image, ImageFormat,
    InteractiveElement as _, IntoElement, KeyDownEvent, ObjectFit, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, StyledImage as _, Subscription,
    Task, UniformListScrollHandle, Window, div, img, prelude::FluentBuilder as _, uniform_list,
};
use gpui_base::Link;
use gpui_component::{
    ActiveTheme as _, Colorize as _, Disableable as _, IconName, IndexPath, Sizable as _,
    WindowExt as _,
    button::{Button, ButtonVariants as _},
    color_picker::{ColorPicker, ColorPickerEvent, ColorPickerState},
    description_list::DescriptionList,
    h_flex,
    input::{Input, InputContentType, InputEvent, InputState, NumberInput},
    radio::{Radio, RadioGroup},
    scroll::ScrollableElement as _,
    select::{Select, SelectEvent, SelectItem, SelectState},
    sidebar::{Sidebar, SidebarMenu, SidebarMenuItem},
    slider::{Slider, SliderEvent, SliderState},
    switch::Switch,
    theme::try_parse_color,
    v_flex,
};

use crate::{
    app_log::{self, AppLogLevel},
    cloud_filters::{CloudClient, CloudConnectionProfile},
    i18n::Language,
    state_store::{
        AppSettings, CloudSettings, LogFontFamily, MAX_WORD_BOUNDARY_CHARACTERS, ShortcutSettings,
        ThemePreference, normalize_search_history,
    },
};
use vclogg_data::IndexCacheInfo;

const GITHUB_REPOSITORY_URL: &str = "https://github.com/zhaiyanqi/vclogg2";

impl SelectItem for LogFontFamily {
    type Value = Self;

    fn title(&self) -> gpui::SharedString {
        match self {
            Self::CascadiaMono => "Cascadia Mono".into(),
            Self::JetBrainsMono => "JetBrains Mono".into(),
            Self::Consolas => "Consolas".into(),
            Self::SystemMonospace => crate::tr!("系统等宽字体", "System monospace").into(),
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

#[cfg(windows)]
fn application_icon() -> Arc<Image> {
    static ICON: OnceLock<Arc<Image>> = OnceLock::new();

    ICON.get_or_init(|| {
        Arc::new(Image::from_bytes(
            ImageFormat::Ico,
            include_bytes!("../resources/windows/vclogg2.ico").to_vec(),
        ))
    })
    .clone()
}

#[cfg(not(windows))]
fn application_icon() -> Arc<Image> {
    static ICON: OnceLock<Arc<Image>> = OnceLock::new();

    ICON.get_or_init(|| {
        Arc::new(Image::from_bytes(
            ImageFormat::Png,
            include_bytes!("../resources/windows/vclogg2.png").to_vec(),
        ))
    })
    .clone()
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum SettingsCategory {
    #[default]
    General,
    Network,
    Appearance,
    Search,
    Scrolling,
    Storage,
    Shortcuts,
    Advanced,
    About,
}

impl SettingsCategory {
    const ALL: [Self; 9] = [
        Self::General,
        Self::Network,
        Self::Appearance,
        Self::Search,
        Self::Scrolling,
        Self::Storage,
        Self::Shortcuts,
        Self::Advanced,
        Self::About,
    ];

    pub(crate) fn from_storage_value(value: &str) -> Option<Self> {
        match value {
            "general" => Some(Self::General),
            "network" => Some(Self::Network),
            "appearance" => Some(Self::Appearance),
            "search" => Some(Self::Search),
            "scrolling" => Some(Self::Scrolling),
            "storage" => Some(Self::Storage),
            "shortcuts" => Some(Self::Shortcuts),
            "advanced" | "windows" => Some(Self::Advanced),
            "about" => Some(Self::About),
            _ => None,
        }
    }

    pub(crate) fn storage_value(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Network => "network",
            Self::Appearance => "appearance",
            Self::Search => "search",
            Self::Scrolling => "scrolling",
            Self::Storage => "storage",
            Self::Shortcuts => "shortcuts",
            Self::Advanced => "advanced",
            Self::About => "about",
        }
    }

    pub(crate) fn is_available(self) -> bool {
        true
    }

    fn label(self) -> &'static str {
        match self {
            Self::General => crate::tr!("常规", "General"),
            Self::Network => crate::tr!("网络", "Network"),
            Self::Appearance => crate::tr!("外观", "Appearance"),
            Self::Search => crate::tr!("搜索", "Search"),
            Self::Scrolling => crate::tr!("滚动与交互", "Scrolling & interaction"),
            Self::Storage => crate::tr!("存储", "Storage"),
            Self::Shortcuts => crate::tr!("快捷键", "Shortcuts"),
            Self::Advanced => crate::tr!("高级", "Advanced"),
            Self::About => crate::tr!("关于", "About"),
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::General => crate::tr!(
                "文件显示、关闭确认与打开目录行为",
                "File display, close confirmation, and opening folders",
            ),
            Self::Network => crate::tr!(
                "云端服务器、用户身份与 Cookie 连接",
                "Cloud server, user identity, and cookie connection",
            ),
            Self::Appearance => crate::tr!(
                "主题、日志字体、行号与内容呈现",
                "Theme, log font, line numbers, and content presentation",
            ),
            Self::Search => crate::tr!(
                "默认匹配方式、结果数量、高亮与搜索历史",
                "Default matching, result limits, highlighting, and search history",
            ),
            Self::Scrolling => crate::tr!(
                "滚轮行为、双击选词与预读取范围",
                "Mouse wheel behavior, word selection, and read-ahead range",
            ),
            Self::Storage => crate::tr!(
                "索引缓存的占用情况与清理操作",
                "Index cache usage and cleanup",
            ),
            Self::Shortcuts => crate::tr!(
                "应用命令的键盘绑定与冲突检查",
                "Keyboard bindings and conflict checks for application commands",
            ),
            Self::Advanced => crate::tr!(
                "应用日志等级、诊断记录与导出",
                "Application log levels, diagnostic records, and export",
            ),
            Self::About => crate::tr!(
                "版本、构建信息、技术栈、开源组件与许可",
                "Version, build, technology, open-source components, and licenses",
            ),
        }
    }

    fn matches(self, query: &str) -> bool {
        if query.is_empty() {
            return true;
        }

        let keywords = match self {
            Self::General => crate::tr!(
                "常规 语言 中文 英文 文件与标签 在文件工具栏显示完整路径 关闭日志标签前确认 打开目录命令 路径 标签 关闭 确认 目录 命令 full path close tab open language Chinese English",
                "general language Chinese English files tabs full path close confirmation open folder command",
            ),
            Self::Network => {
                "网络 远程服务 云端服务器 服务器地址 用户名 工号 昵称 保存 测试 连接 Cookie HTTP HTTPS network remote server user connect"
            }
            Self::Appearance => {
                "外观 界面主题 深色 浅色 显示行号 显示行号行间分隔线 行号栏宽度 行号文字颜色 行号背景色 日志级别着色 日志分隔线 日志字体 日志字号 日志行距 theme font color"
            }
            Self::Search => {
                "搜索 区分大小写 使用正则表达式 最大搜索结果数 高亮已提交搜索的匹配文字 搜索历史 历史记录 管理 删除 清空 大小写 正则 结果 高亮 search regex case highlight history"
            }
            Self::Scrolling => {
                "滚动与交互 滚动与动态效果 按完整日志行滚动 每次滚动行数 自动换行时仍按完整日志行滚动 像素滚动距离 分词边界字符 相邻行预读取 减少动态效果 滚轮 像素 行数 自动换行 分词 双击 预读取 scroll motion word wrap"
            }
            Self::Storage => {
                "存储 索引缓存 缓存大小 打开缓存文件夹 清理缓存 文件夹 清理 storage index cache"
            }
            Self::Shortcuts => {
                "快捷键 打开文件 聚焦搜索框 快速查找 关闭当前标签 打开设置 切换区分大小写 跳到日志底部 轮换颜色标签 切换自动换行 按键 绑定 冲突 keyboard shortcut keymap"
            }
            Self::Advanced => {
                "高级 应用日志 日志等级 关闭 Error Warn Info Debug Trace 导出 诊断 advanced application log level export diagnostics"
            }
            Self::About => {
                "关于 VCLogg2 版本 编译时间 构建目标 commit 提交 技术栈 开源库 GitHub 仓库 repository 源代码 source code 作者 zhaiyanqi copyright 版权 Apache 2.0 license Rust GPUI SQLite"
            }
        };
        let query = query.to_lowercase();
        let haystack = format!("{} {} {keywords}", self.label(), self.description()).to_lowercase();
        query.split_whitespace().all(|term| haystack.contains(term))
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
}

impl ShortcutAction {
    const ALL: [Self; 9] = [
        Self::OpenFile,
        Self::FocusSearch,
        Self::QuickFind,
        Self::CloseTab,
        Self::OpenSettings,
        Self::ToggleCase,
        Self::JumpBottom,
        Self::CycleColorLabel,
        Self::ToggleWordWrap,
    ];

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
            Self::ToggleWordWrap => crate::tr!(
                "切换正文、当前结果和全局结果的长行换行显示",
                "Toggle wrapping in the log, current results, and global results"
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
        }
    }
}

pub struct SettingsDialog {
    draft: AppSettings,
    active_category: SettingsCategory,
    settings_search: Entity<InputState>,
    network_server_url: Entity<InputState>,
    network_display_name: Entity<InputState>,
    cloud_client: Option<CloudClient>,
    cloud_connection: Option<CloudConnectionProfile>,
    cloud_client_error: Option<String>,
    network_status: SharedString,
    network_status_kind: NetworkStatusKind,
    network_task: Option<Task<()>>,
    search_history: Vec<String>,
    search_history_filter: Entity<InputState>,
    search_history_scroll: UniformListScrollHandle,
    font_family: Entity<SelectState<Vec<LogFontFamily>>>,
    app_log_level: Entity<SelectState<Vec<AppLogLevel>>>,
    font_size: Entity<SliderState>,
    line_spacing: Entity<SliderState>,
    line_number_width: Entity<SliderState>,
    line_number_text_color: Entity<ColorPickerState>,
    line_number_background_color: Entity<ColorPickerState>,
    line_number_text_color_custom: bool,
    line_number_background_color_custom: bool,
    scroll_percent: Entity<SliderState>,
    scroll_lines: Entity<SliderState>,
    viewer_overscan: Entity<SliderState>,
    max_search_results: Entity<InputState>,
    word_boundary_characters: Entity<InputState>,
    open_directory_command: Entity<InputState>,
    shortcut_inputs: BTreeMap<ShortcutAction, Entity<InputState>>,
    cache_dir: Option<PathBuf>,
    cache_info: Option<IndexCacheInfo>,
    cache_status: Option<SharedString>,
    cache_busy: bool,
    cache_task: Option<Task<()>>,
    log_export_status: Option<SharedString>,
    log_export_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

#[derive(Clone, Debug)]
pub(crate) enum SettingsDialogEvent {
    DraftChanged,
    CategoryChanged(SettingsCategory),
    CloudSettings(CloudSettings),
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
    fn draft_changed(cx: &mut Context<Self>) {
        cx.emit(SettingsDialogEvent::DraftChanged);
        cx.notify();
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
        let settings_search = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(crate::tr!("搜索设置…", "Search settings…"))
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
        let font_family = cx.new(|cx| {
            SelectState::new(
                LogFontFamily::ALL.to_vec(),
                Some(IndexPath::new(settings.log_font_family.select_index())),
                window,
                cx,
            )
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
        let viewer_overscan = cx.new(|_| {
            SliderState::new()
                .min(4.)
                .max(40.)
                .step(1.)
                .default_value(settings.viewer_overscan.clamp(4, 40) as f32)
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
        subscriptions.push(
            cx.subscribe(&settings_search, |this, search, _: &InputEvent, cx| {
                let query = search.read(cx).value().trim().to_lowercase();
                if !this.active_category.matches(&query)
                    && let Some(category) = SettingsCategory::ALL
                        .into_iter()
                        .filter(|category| category.is_available())
                        .find(|category| category.matches(&query))
                {
                    this.active_category = category;
                    cx.emit(SettingsDialogEvent::CategoryChanged(category));
                }
                cx.notify();
            }),
        );
        subscriptions
            .push(cx.subscribe(&network_server_url, |_, _, _: &InputEvent, cx| cx.notify()));
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
                    cx.notify();
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
            |_, _, _: &SelectEvent<Vec<LogFontFamily>>, cx| SettingsDialog::draft_changed(cx),
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
        subscriptions.push(cx.subscribe(
            &line_number_text_color,
            |this, _, _: &ColorPickerEvent, cx| {
                this.line_number_text_color_custom = true;
                SettingsDialog::draft_changed(cx);
            },
        ));
        subscriptions.push(cx.subscribe(
            &line_number_background_color,
            |this, _, _: &ColorPickerEvent, cx| {
                this.line_number_background_color_custom = true;
                SettingsDialog::draft_changed(cx);
            },
        ));
        subscriptions.push(cx.subscribe(&scroll_percent, |_, _, _: &SliderEvent, cx| {
            SettingsDialog::draft_changed(cx)
        }));
        subscriptions.push(cx.subscribe(&scroll_lines, |_, _, _: &SliderEvent, cx| {
            SettingsDialog::draft_changed(cx)
        }));
        subscriptions.push(cx.subscribe(&viewer_overscan, |_, _, _: &SliderEvent, cx| {
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

        for action in ShortcutAction::ALL {
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

        let cache_dir = crate::app_paths::index_cache_dir();
        let mut dialog = Self {
            draft: settings,
            active_category: if active_category.is_available() {
                active_category
            } else {
                SettingsCategory::default()
            },
            settings_search,
            network_server_url,
            network_display_name,
            cloud_client,
            cloud_connection,
            cloud_client_error,
            network_status,
            network_status_kind,
            network_task: None,
            search_history: normalize_search_history(search_history),
            search_history_filter,
            search_history_scroll: UniformListScrollHandle::new(),
            font_family,
            app_log_level,
            font_size,
            line_spacing,
            line_number_width,
            line_number_text_color,
            line_number_background_color,
            line_number_text_color_custom,
            line_number_background_color_custom,
            scroll_percent,
            scroll_lines,
            viewer_overscan,
            max_search_results,
            word_boundary_characters,
            open_directory_command,
            shortcut_inputs,
            cache_dir,
            cache_info: None,
            cache_status: None,
            cache_busy: false,
            cache_task: None,
            log_export_status: None,
            log_export_task: None,
            _subscriptions: subscriptions,
        };
        dialog.refresh_cache_info(cx);
        dialog
    }

    pub fn settings(&self, cx: &gpui::App) -> Result<AppSettings, String> {
        let mut settings = self.draft.clone();
        settings.app_log_level = self
            .app_log_level
            .read(cx)
            .selected_value()
            .copied()
            .unwrap_or_default();
        settings.log_font_family = self
            .font_family
            .read(cx)
            .selected_value()
            .copied()
            .unwrap_or_default();
        settings.log_font_size = self.font_size.read(cx).value().start().round() as u16;
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
        settings.mouse_wheel_scroll_percent =
            self.scroll_percent.read(cx).value().start().round() as u16;
        settings.mouse_wheel_scroll_lines =
            self.scroll_lines.read(cx).value().start().round() as u16;
        settings.viewer_overscan = self.viewer_overscan.read(cx).value().start().round() as u16;
        let max_search_results = self.max_search_results.read(cx).value();
        settings.max_search_results = max_search_results.trim().parse::<u32>().map_err(|_| {
            crate::tr!(
                "最大搜索结果数必须是 0 到 4,294,967,295 之间的整数",
                "Maximum search results must be an integer from 0 to 4,294,967,295",
            )
            .to_string()
        })?;
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
            return Err(crate::tr_args!(
                "分词边界字符最多允许 {MAX_WORD_BOUNDARY_CHARACTERS} 个 Unicode 字符",
                "Word-boundary characters may contain at most {MAX_WORD_BOUNDARY_CHARACTERS} Unicode characters",
            ));
        }
        for action in ShortcutAction::ALL {
            let value = self.shortcut_value(action, cx);
            if !value.is_empty() && crate::actions::shortcut_to_key_binding(&value).is_none() {
                return Err(crate::tr_args!(
                    "“{}”的快捷键无效，请重新录入",
                    "The shortcut for “{}” is invalid. Enter it again.",
                    action.label(),
                ));
            }
            action.set(&mut settings.shortcuts, value);
        }

        if !self.conflicts(cx).is_empty() {
            return Err(crate::tr!(
                "存在重复快捷键，请先解决标红的冲突",
                "Some shortcuts are duplicated. Resolve the highlighted conflicts first.",
            )
            .to_string());
        }
        Ok(settings)
    }

    pub fn search_history(&self) -> Vec<String> {
        normalize_search_history(self.search_history.clone())
    }

    pub fn network_settings(&self, cx: &gpui::App) -> CloudSettings {
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
        cx.emit(SettingsDialogEvent::CloudSettings(settings.clone()));
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
                        let normalized_settings = CloudSettings {
                            server_url: connection.server_url.clone(),
                            display_name: connection.display_name.clone(),
                        };
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
                        cx.emit(SettingsDialogEvent::CloudSettings(normalized_settings));
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

    fn shortcut_value(&self, action: ShortcutAction, cx: &gpui::App) -> String {
        self.shortcut_inputs[&action]
            .read(cx)
            .value()
            .trim()
            .to_string()
    }

    fn conflicts(&self, cx: &gpui::App) -> HashMap<ShortcutAction, String> {
        let mut by_value: HashMap<String, Vec<ShortcutAction>> = HashMap::new();
        for action in ShortcutAction::ALL {
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

    fn export_application_log(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.log_export_task.is_some() {
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
                        this.log_export_status = Some(
                            crate::tr_args!(
                                "已导出 {entry_count} 条应用日志到 {}",
                                "Exported {entry_count} application log entries to {}",
                                path.display(),
                            )
                            .into(),
                        );
                        window.push_notification(
                            crate::tr!("应用日志已导出", "Application log exported"),
                            cx,
                        );
                    }
                    Some(Err(error)) => {
                        this.log_export_status = Some(
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
    ) -> gpui::AnyElement {
        let input = self.shortcut_inputs[&action].clone();
        let default_value = action.value(&ShortcutSettings::default()).to_string();
        let input_for_capture = input.clone();
        let input_for_reset = input.clone();

        h_flex()
            .id(format!("settings-shortcut-row-{}", action.id()))
            .items_start()
            .justify_between()
            .gap_4()
            .px_2()
            .py_2()
            .child(
                v_flex()
                    .min_w_0()
                    .flex_1()
                    .gap_0p5()
                    .child(
                        div()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(action.label()),
                    )
                    .child(
                        div()
                            .text_sm()
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
                                    .flex_1()
                                    .on_key_down(move |event: &KeyDownEvent, window, cx| {
                                        match event.keystroke.key.as_str() {
                                            "escape" | "tab" => return,
                                            "backspace" | "delete" => {
                                                input_for_capture.update(cx, |state, cx| {
                                                    state.set_value("", window, cx)
                                                });
                                            }
                                            "control" | "ctrl" | "alt" | "shift" | "meta"
                                            | "cmd" | "command" => return,
                                            _ => {
                                                let value = shortcut_from_event(event);
                                                input_for_capture.update(cx, |state, cx| {
                                                    state.set_value(value, window, cx)
                                                });
                                            }
                                        }
                                        cx.stop_propagation();
                                    })
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
            .on_mouse_down(gpui::MouseButton::Left, move |_, window, cx| {
                input.focus_handle(cx).focus(window, cx);
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
                .h_64()
                .flex()
                .items_center()
                .justify_center()
                .rounded(cx.theme().radius)
                .border_1()
                .border_color(cx.theme().border)
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(if total_count == 0 {
                    crate::tr!("暂无搜索历史", "No search history")
                } else {
                    crate::tr!(
                        "没有符合筛选条件的搜索历史",
                        "No search history matches the filter",
                    )
                })
                .into_any_element()
        } else {
            div()
                .relative()
                .h_64()
                .rounded(cx.theme().radius)
                .border_1()
                .border_color(cx.theme().border)
                .overflow_hidden()
                .child(
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
                                        .h(gpui::px(44.))
                                        .w_full()
                                        .min_w_0()
                                        .justify_between()
                                        .gap_3()
                                        .px_3()
                                        .border_b_1()
                                        .border_color(cx.theme().border.opacity(0.72))
                                        .child(
                                            div()
                                                .min_w_0()
                                                .flex_1()
                                                .truncate()
                                                .text_sm()
                                                .child(query.clone()),
                                        )
                                        .child(
                                            Button::new(format!(
                                                "settings-search-history-delete:{query}"
                                            ))
                                            .xsmall()
                                            .ghost()
                                            .icon(IconName::Delete)
                                            .tooltip(crate::tr!(
                                                "删除此条搜索历史",
                                                "Delete this search history entry",
                                            ))
                                            .on_click(move |_, _, cx| {
                                                settings.update(cx, |this, cx| {
                                                    this.remove_search_history(
                                                        &query_for_delete,
                                                        cx,
                                                    );
                                                });
                                            }),
                                        )
                                        .into_any_element()
                                })
                                .collect()
                        },
                    )
                    .size_full()
                    .track_scroll(&self.search_history_scroll),
                )
                .vertical_scrollbar(&self.search_history_scroll)
                .into_any_element()
        };

        v_flex()
            .id("settings-search-history-section")
            .gap_3()
            .p_3()
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .justify_between()
                    .gap_4()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                div()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child(crate::tr!("搜索历史", "Search history")),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(
                                        crate::tr!(
                                            "按原文去重并保留全部记录，最新搜索排在最前；删除会在保存设置后生效。",
                                            "Duplicate queries are removed and recent searches appear first. Deletions take effect when settings are saved.",
                                        ),
                                    ),
                            ),
                    )
                    .child(
                        Button::new("settings-search-history-clear")
                            .small()
                            .outline()
                            .icon(IconName::Delete)
                            .label(crate::tr!("清空全部", "Clear all"))
                            .disabled(total_count == 0)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.clear_search_history(cx);
                            })),
                    ),
            )
            .child(
                h_flex()
                    .gap_3()
                    .child(
                        Input::new(&self.search_history_filter)
                            .small()
                            .prefix(IconName::Search)
                            .flex_1(),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(crate::tr_args!(
                                "显示 {visible_count} / 共 {total_count} 条",
                                "Showing {visible_count} of {total_count}",
                            )),
                    ),
            )
            .child(history_list)
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
                                img(application_icon())
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
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
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
            .child(about_section(
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
            ))
            .child(about_section(
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
                    .item(crate::tr!("界面", "Interface"), "GPUI · gpui-component · gpui-base", 1)
                    .item(crate::tr!("数据与检索", "Data & search"), crate::tr!("SQLite/WAL · 内存映射 · 正则与多模式匹配", "SQLite/WAL · memory mapping · regex and multi-pattern matching"), 1)
                    .item(crate::tr!("网络", "Networking"), "HTTP/HTTPS · rustls", 1)
                    .item(crate::tr!("平台集成", "Platform integration"), crate::tr!("系统回收站、凭据库、文件打开关联、单实例与原生窗口", "System trash, credential storage, file-opening associations, single-instance routing, and native windows"), 1),
                cx,
            ))
            .child(about_section(
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
                        "gpui-component / gpui-base",
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
                        "keyring / sha2",
                        crate::tr!("系统凭据保护与安全摘要", "System credential protection and secure digests"),
                        1,
                    ),
                cx,
            ))
            .child(about_section(
                "settings-about-repository",
                crate::tr!("GitHub 仓库", "GitHub repository"),
                crate::tr!(
                    "访问项目源代码、问题追踪与发布版本。",
                    "Visit the source code, issue tracker, and published releases.",
                ),
                DescriptionList::new().small().columns(1).item(
                    crate::tr!("仓库地址", "Repository URL"),
                    Link::new("settings-about-github-link")
                        .href(GITHUB_REPOSITORY_URL)
                        .open_with(|href, _, _, cx| cx.open_url(href))
                        .accessibility_label(crate::tr!(
                            "在浏览器中打开 VCLogg2 GitHub 仓库",
                            "Open the VCLogg2 GitHub repository in a browser",
                        ))
                        .px_1()
                        .rounded(cx.theme().radius)
                        .border_1()
                        .border_color(cx.theme().ring.opacity(0.))
                        .text_color(cx.theme().link)
                        .text_decoration_1()
                        .text_decoration_color(cx.theme().link.opacity(0.5))
                        .hover(|link| {
                            link.text_color(cx.theme().link.opacity(0.8))
                                .text_decoration_color(cx.theme().link)
                        })
                        .active(|link| link.text_color(cx.theme().link.opacity(0.6)))
                        .focus_visible(|link| link.border_color(cx.theme().ring))
                        .cursor_pointer()
                        .child(GITHUB_REPOSITORY_URL)
                        .into_any_element(),
                    1,
                ),
                cx,
            ))
            .child(about_section(
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
            ))
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
    v_flex()
        .id(id)
        .gap_3()
        .child(
            v_flex()
                .gap_1()
                .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child(title))
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(description),
                ),
        )
        .child(content)
        .into_any_element()
}

impl Render for SettingsDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _performance_scope = crate::ui_performance::scope("SettingsDialog::render");
        let conflicts = self.conflicts(cx);
        let font_size = self.font_size.read(cx).value().start().round() as u16;
        let line_spacing = self.line_spacing.read(cx).value().start().round() as u16;
        let line_number_width = self.line_number_width.read(cx).value().start().round() as u16;
        let line_number_text_color = self
            .line_number_text_color
            .read(cx)
            .value()
            .map(|color| color.alpha(1.).to_hex());
        let line_number_background_color = self
            .line_number_background_color
            .read(cx)
            .value()
            .map(|color| color.alpha(1.).to_hex());
        let scroll_percent = self.scroll_percent.read(cx).value().start().round() as u16;
        let scroll_lines = self.scroll_lines.read(cx).value().start().round() as u16;
        let viewer_overscan = self.viewer_overscan.read(cx).value().start().round() as u16;
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
        let log_entries = app_log::entry_count();
        let network_busy = self.network_task.is_some();
        let network_status_color = match self.network_status_kind {
            NetworkStatusKind::Neutral => cx.theme().muted_foreground,
            NetworkStatusKind::Success => cx.theme().success,
            NetworkStatusKind::Error => cx.theme().danger,
        };
        let active_category = self.active_category;
        let settings_entity = cx.entity();
        let settings_query = self.settings_search.read(cx).value().trim().to_lowercase();
        let visible_categories = SettingsCategory::ALL
            .into_iter()
            .filter(|category| category.is_available())
            .filter(|category| category.matches(&settings_query))
            .collect::<Vec<_>>();
        let has_matches = !visible_categories.is_empty();
        let shortcuts_active = has_matches && active_category == SettingsCategory::Shortcuts;
        let category_menu =
            SidebarMenu::new().children(visible_categories.into_iter().map(|category| {
                let settings_entity = settings_entity.clone();
                SidebarMenuItem::new(category.label())
                    .active(category == active_category)
                    .on_click(move |_, _, cx| {
                        settings_entity.update(cx, |this, cx| {
                            this.active_category = category;
                            cx.emit(SettingsDialogEvent::CategoryChanged(category));
                            cx.notify();
                        });
                    })
            }));

        h_flex()
            .id("settings-dialog-content")
            .w_full()
            .h_full()
            .min_h_0()
            .items_stretch()
            .overflow_hidden()
            .child(
                Sidebar::new("settings-category-sidebar")
                    .w_64()
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
                    .child(
                        v_flex()
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scrollbar()
                            .child(
                                v_flex()
                                    .gap_5()
                                    .px_6()
                                    .py_5()
                                    .when(shortcuts_active, |content| {
                                        content.flex_1().min_h_0()
                                    })
                                    .when(!has_matches, |content| {
                                        content.child(
                                            v_flex()
                                                .items_center()
                                                .gap_2()
                                                .py_12()
                                                .text_color(cx.theme().muted_foreground)
                                                .child(
                                                    div()
                                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                                        .child(crate::tr!(
                                                            "未找到匹配的设置",
                                                            "No matching settings",
                                                        )),
                                                )
                                                .child(
                                                    div()
                                                        .text_sm()
                                                        .child(crate::tr!(
                                                            "请尝试更短或不同的关键词",
                                                            "Try a shorter or different search",
                                                        )),
                                                ),
                                        )
                                    })
                                    .when(
                                        has_matches
                                            && active_category == SettingsCategory::Appearance,
                                        |content| {
                                            content.child(
                                                v_flex()
                    .id("settings-theme-section")
                    .gap_3()
                    .p_3()
                    .rounded(cx.theme().radius)
                    .border_1()
                    .border_color(cx.theme().border)
                    .child(
                        v_flex()
                            .gap_1()
                            .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child(crate::tr!("界面主题", "Interface theme")))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(crate::tr!("保存后立即同步到所有窗口。", "Changes are synchronized to every window after saving.")),
                            ),
                    )
                    .child(
                        RadioGroup::vertical("settings-theme-preference")
                            .selected_index(Some(self.draft.theme_preference.select_index()))
                            .children(ThemePreference::ALL.into_iter().map(|preference| {
                                Radio::new(format!(
                                    "settings-theme-{}",
                                    preference.database_value()
                                ))
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
                                if let Some(preference) =
                                    ThemePreference::ALL.get(*selected_index).copied()
                                {
                                    this.draft.theme_preference = preference;
                                    SettingsDialog::draft_changed(cx);
                                }
                            })),
                    ),
                                            )
            .child(
                h_flex()
                    .justify_between()
                    .gap_4()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child(crate::tr!("显示行号", "Show line numbers")))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(crate::tr!("在正文和搜索结果中显示固定行号列", "Show a fixed line-number column in logs and search results")),
                            ),
                    )
                    .child(
                        Switch::new("settings-show-line-numbers")
                            .small()
                            .checked(self.draft.default_show_line_numbers)
                            .tooltip(crate::tr!("显示行号", "Show line numbers"))
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.draft.default_show_line_numbers = *checked;
                                SettingsDialog::draft_changed(cx);
                            })),
                    ),
            )
            .child(
                h_flex()
                    .justify_between()
                    .gap_4()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                div()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child(crate::tr!("显示行号行间分隔线", "Show line-number separators")),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(crate::tr!("只在相邻行号之间显示主题弱分隔线", "Show a subtle themed separator only between adjacent line numbers")),
                            ),
                    )
                    .child(
                        Switch::new("settings-line-number-row-separators")
                            .small()
                            .checked(self.draft.show_line_number_row_separators)
                            .tooltip(crate::tr!("显示行号行间分隔线", "Show line-number separators"))
                            .disabled(!self.draft.default_show_line_numbers)
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.draft.show_line_number_row_separators = *checked;
                                SettingsDialog::draft_changed(cx);
                            })),
                    ),
            )
            .child(
                h_flex()
                    .justify_between()
                    .gap_4()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child(crate::tr!("行号栏宽度", "Line-number column width")))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(crate::tr!("同时应用于日志列表、单文件结果和全局搜索结果", "Applies to the log, file results, and global results")),
                            ),
                    )
                    .child(
                        h_flex()
                            .w_56()
                            .gap_3()
                            .child(Slider::new(&self.line_number_width).flex_1())
                            .child(
                                div()
                                    .w_10()
                                    .text_right()
                                    .text_sm()
                                    .child(format!("{line_number_width}px")),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .justify_between()
                    .gap_4()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child(crate::tr!("行号文字颜色", "Line-number text color")))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(crate::tr!("默认随当前主题变化；选色后使用自定义颜色", "Uses the current theme by default; choosing a color sets a custom value")),
                            ),
                    )
                    .child(
                        h_flex()
                            .w_72()
                            .justify_end()
                            .gap_2()
                            .child(
                                div()
                                    .min_w_20()
                                    .text_right()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(if self.line_number_text_color_custom {
                                        line_number_text_color
                                            .clone()
                                            .unwrap_or_else(|| crate::tr!("未选择", "Not selected").to_string())
                                    } else {
                                        crate::tr!("跟随主题", "Follow theme").to_string()
                                    }),
                            )
                            .child(
                                ColorPicker::new(&self.line_number_text_color)
                                    .small()
                                    .label(crate::tr!("行号文字颜色", "Line-number text color")),
                            )
                            .child(
                                Button::new("settings-reset-line-number-text-color")
                                    .small()
                                    .ghost()
                                    .label(crate::tr!("恢复主题色", "Restore theme color"))
                                    .disabled(!self.line_number_text_color_custom)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.reset_line_number_text_color(window, cx)
                                    })),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .justify_between()
                    .gap_4()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child(crate::tr!("行号背景色", "Line-number background")))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(crate::tr!("默认使用低对比度主题底色，减少与日志正文的竞争", "Uses a low-contrast themed background by default to keep focus on the log")),
                            ),
                    )
                    .child(
                        h_flex()
                            .w_72()
                            .justify_end()
                            .gap_2()
                            .child(
                                div()
                                    .min_w_20()
                                    .text_right()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(if self.line_number_background_color_custom {
                                        line_number_background_color
                                            .clone()
                                            .unwrap_or_else(|| crate::tr!("未选择", "Not selected").to_string())
                                    } else {
                                        crate::tr!("跟随主题", "Follow theme").to_string()
                                    }),
                            )
                            .child(
                                ColorPicker::new(&self.line_number_background_color)
                                    .small()
                                    .label(crate::tr!("行号背景色", "Line-number background")),
                            )
                            .child(
                                Button::new("settings-reset-line-number-background-color")
                                    .small()
                                    .ghost()
                                    .label(crate::tr!("恢复主题色", "Restore theme color"))
                                    .disabled(!self.line_number_background_color_custom)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.reset_line_number_background_color(window, cx)
                                    })),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .justify_between()
                    .gap_4()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                div()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child(crate::tr!("日志级别着色", "Log-level coloring")),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(crate::tr!("突出 ERROR、WARN、INFO 与 DEBUG 等级的整行日志", "Highlight complete lines for ERROR, WARN, INFO, and DEBUG levels")),
                            ),
                    )
                    .child(
                        Switch::new("settings-highlight-log-levels")
                            .small()
                            .checked(self.draft.highlight_log_levels)
                            .tooltip(crate::tr!("日志级别着色", "Log-level coloring"))
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.draft.highlight_log_levels = *checked;
                                SettingsDialog::draft_changed(cx);
                            })),
                    ),
            )
            .child(
                h_flex()
                    .justify_between()
                    .gap_4()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child(crate::tr!("日志分隔线", "Log separators")))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(crate::tr!("在相邻日志正文单元格之间显示主题弱分隔线", "Show a subtle themed separator between adjacent log rows")),
                            ),
                    )
                    .child(
                        Switch::new("settings-row-separators")
                            .small()
                            .checked(self.draft.default_show_row_separators)
                            .tooltip(crate::tr!("显示日志分隔线", "Show log separators"))
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.draft.default_show_row_separators = *checked;
                                SettingsDialog::draft_changed(cx);
                            })),
                    ),
            )
            .child(
                h_flex()
                    .justify_between()
                    .gap_4()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child(crate::tr!("日志字体", "Log font")))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(crate::tr!("应用于正文、行号和搜索结果", "Applies to log text, line numbers, and search results")),
                            ),
                    )
                    .child(Select::new(&self.font_family).small().w_56()),
            )
            .child(
                h_flex()
                    .justify_between()
                    .gap_4()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child(crate::tr!("日志字号", "Log font size")))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(crate::tr!("也可在日志区域按住 Ctrl 滚动鼠标滚轮调整", "You can also hold Ctrl and use the mouse wheel over a log")),
                            ),
                    )
                    .child(
                        h_flex()
                            .w_56()
                            .gap_3()
                            .child(Slider::new(&self.font_size).flex_1())
                            .child(
                                div()
                                    .w_10()
                                    .text_right()
                                    .text_sm()
                                    .child(format!("{font_size}px")),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .justify_between()
                    .gap_4()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child(crate::tr!("日志行距", "Log line spacing")))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(crate::tr!("增加每行正文上下留白，字号变化时保持此间距", "Adds vertical space around each line and keeps it when the font size changes")),
                            ),
                    )
                    .child(
                        h_flex()
                            .w_56()
                            .gap_3()
                            .child(Slider::new(&self.line_spacing).flex_1())
                            .child(
                                div()
                                    .w_10()
                                    .text_right()
                                    .text_sm()
                                    .child(format!("{line_spacing}px")),
                            ),
                    ),
            )
                                        },
                                    )
                                    .when(
                                        has_matches
                                            && active_category == SettingsCategory::Search,
                                        |content| {
                                            content.child(
                                                v_flex()
                    .id("settings-search-section")
                    .gap_3()
                    .p_3()
                    .rounded(cx.theme().radius)
                    .border_1()
                    .border_color(cx.theme().border)
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                div()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child(crate::tr!("搜索", "Search")),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(crate::tr!("控制所有窗口共享的搜索选项、结果保留与高亮。", "Controls search options, result limits, and highlighting shared by all windows.")),
                            ),
                    )
                    .child(
                        h_flex()
                            .justify_between()
                            .gap_4()
                            .child(div().text_sm().child(crate::tr!("区分大小写", "Case-sensitive")))
                            .child(
                                Switch::new("settings-default-case-sensitive")
                                    .small()
                                    .checked(self.draft.default_case_sensitive)
                                    .tooltip(crate::tr!("区分大小写", "Case-sensitive"))
                                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                        this.draft.default_case_sensitive = *checked;
                                        SettingsDialog::draft_changed(cx);
                                    })),
                            ),
                    )
                    .child(
                        h_flex()
                            .justify_between()
                            .gap_4()
                            .child(div().text_sm().child(crate::tr!("使用正则表达式", "Use regular expressions")))
                            .child(
                                Switch::new("settings-default-use-regex")
                                    .small()
                                    .checked(self.draft.default_use_regex)
                                    .tooltip(crate::tr!("使用正则表达式", "Use regular expressions"))
                                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                        this.draft.default_use_regex = *checked;
                                        SettingsDialog::draft_changed(cx);
                                    })),
                            ),
                    )
                    .child(
                        h_flex()
                            .justify_between()
                            .gap_4()
                            .child(
                                v_flex()
                                    .gap_1()
                                    .child(
                                        div()
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .child(crate::tr!("最大搜索结果数", "Maximum search results")),
                                    )
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(crate::tr!("设为 0 表示不限制；每个参与文件独立应用上限。", "Set to 0 for no limit. The limit is applied separately to each participating file.")),
                                    ),
                            )
                            .child(
                                NumberInput::new(&self.max_search_results)
                                    .small()
                                    .w_56(),
                            ),
                    )
                    .child(
                        h_flex()
                            .justify_between()
                            .gap_4()
                            .child(div().text_sm().child(crate::tr!("高亮已提交搜索的匹配文字", "Highlight matches from submitted searches")))
                            .child(
                                Switch::new("settings-highlight-matches")
                                    .small()
                                    .checked(self.draft.highlight_matches)
                                    .tooltip(crate::tr!("高亮已提交搜索的匹配文字", "Highlight matches from submitted searches"))
                                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                        this.draft.highlight_matches = *checked;
                                        SettingsDialog::draft_changed(cx);
                                    })),
                            ),
                    ),
            )
                                            .child(
                                                self.render_search_history_management(cx),
                                            )
                                        },
                                    )
                                    .when(
                                        has_matches
                                            && active_category == SettingsCategory::General,
                                        |content| {
                                            content.child(
                                                v_flex()
                    .gap_5()
                    .child(
                        v_flex()
                            .id("settings-language-section")
                            .gap_3()
                            .p_3()
                            .rounded(cx.theme().radius)
                            .border_1()
                            .border_color(cx.theme().border)
                            .child(
                                v_flex()
                                    .gap_1()
                                    .child(
                                        div()
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .child(crate::tr!("语言", "Language")),
                                    )
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(crate::tr!(
                                                "选择应用界面使用的语言。",
                                                "Choose the language used by the application interface.",
                                            )),
                                    ),
                            )
                            .child(
                                RadioGroup::vertical("settings-language")
                                    .selected_index(Some(self.draft.language.select_index()))
                                    .children(Language::ALL.into_iter().map(|language| {
                                        Radio::new(format!(
                                            "settings-language-{}",
                                            language.database_value()
                                        ))
                                        .small()
                                        .label(language.native_name())
                                    }))
                                    .on_click(cx.listener(
                                        |this, selected_index: &usize, _, cx| {
                                            if let Some(language) =
                                                Language::ALL.get(*selected_index).copied()
                                            {
                                                this.draft.language = language;
                                                SettingsDialog::draft_changed(cx);
                                            }
                                        },
                                    )),
                            ),
                    )
                    .child(
                        v_flex()
                    .id("settings-files-section")
                    .gap_3()
                    .p_3()
                    .rounded(cx.theme().radius)
                    .border_1()
                    .border_color(cx.theme().border)
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                div()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child(crate::tr!("文件与标签", "Files & tabs")),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(crate::tr!("控制路径显示和标签关闭前的安全确认。", "Controls path display and confirmation before closing tabs.")),
                            ),
                    )
                    .child(
                        h_flex()
                            .justify_between()
                            .gap_4()
                            .child(div().text_sm().child(crate::tr!("在文件工具栏显示完整路径", "Show full path in the file toolbar")))
                            .child(
                                Switch::new("settings-show-full-path")
                                    .small()
                                    .checked(self.draft.show_full_path)
                                    .tooltip(crate::tr!("显示完整路径", "Show full path"))
                                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                        this.draft.show_full_path = *checked;
                                        SettingsDialog::draft_changed(cx);
                                    })),
                            ),
                    )
                    .child(
                        h_flex()
                            .justify_between()
                            .gap_4()
                            .child(div().text_sm().child(crate::tr!("关闭日志标签前确认", "Confirm before closing log tabs")))
                            .child(
                                Switch::new("settings-confirm-close-tab")
                                    .small()
                                    .checked(self.draft.confirm_close_tab)
                                    .tooltip(crate::tr!("关闭日志标签前确认", "Confirm before closing log tabs"))
                                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                        this.draft.confirm_close_tab = *checked;
                                        SettingsDialog::draft_changed(cx);
                                    })),
                            ),
                    )
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                div()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child(crate::tr!("打开目录命令", "Open-folder command")),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(
                                        crate::tr!(
                                            "留空使用系统默认程序；支持 {directory} 与 {path} 占位符，未使用时自动追加目录。",
                                            "Leave empty to use the system default. Supports {directory} and {path}; the folder is appended when neither placeholder is used.",
                                        ),
                                    ),
                            )
                            .child(
                                div()
                                    .id("settings-open-directory-command")
                                    .w_full()
                                    .child(Input::new(&self.open_directory_command).w_full()),
                            ),
                    ),
            ),
                    )
                                        },
                                    )
                                    .when(
                                        has_matches
                                            && active_category == SettingsCategory::Network,
                                        |content| {
                                            content.child(
                                                v_flex()
                                                    .id("settings-network-section")
                                                    .gap_4()
                                                    .p_3()
                                                    .rounded(cx.theme().radius)
                                                    .border_1()
                                                    .border_color(cx.theme().border)
                                                    .child(
                                                        v_flex()
                                                            .gap_1()
                                                            .child(
                                                                div()
                                                                    .font_weight(
                                                                        gpui::FontWeight::SEMIBOLD,
                                                                    )
                                                                    .child(crate::tr!("远程服务", "Remote service")),
                                                            )
                                                            .child(
                                                                div()
                                                                    .text_sm()
                                                                    .text_color(
                                                                        cx.theme()
                                                                            .muted_foreground,
                                                                    )
                                                                    .child(
                                                                        crate::tr!("服务器 Cookie 仅用于按需访问云端接口，不会建立或保持长连接。", "The server cookie is used only when accessing cloud APIs and does not create or maintain a persistent connection."),
                                                                    ),
                                                            ),
                                                    )
                                                    .child(
                                                        v_flex()
                                                            .gap_1()
                                                            .child(
                                                                div()
                                                                    .text_sm()
                                                                    .font_weight(
                                                                        gpui::FontWeight::SEMIBOLD,
                                                                    )
                                                                    .child(crate::tr!("服务器地址", "Server address")),
                                                            )
                                                            .child(
                                                                div()
                                                                    .text_xs()
                                                                    .text_color(
                                                                        cx.theme()
                                                                            .muted_foreground,
                                                                    )
                                                                    .child(
                                                                        crate::tr!("支持 HTTP 或 HTTPS；建议远程服务使用 HTTPS。", "Supports HTTP or HTTPS. HTTPS is recommended for remote services."),
                                                                    ),
                                                            )
                                                            .child(
                                                                Input::new(
                                                                    &self.network_server_url,
                                                                )
                                                                .w_full()
                                                                .content_type(
                                                                    InputContentType::Url,
                                                                )
                                                                .accessibility_id("A391")
                                                                .aria_label(crate::tr!("云端服务器地址", "Cloud server address"))
                                                                .disabled(network_busy),
                                                            ),
                                                    )
                                                    .child(
                                                        v_flex()
                                                            .gap_1()
                                                            .child(
                                                                div()
                                                                    .text_sm()
                                                                    .font_weight(
                                                                        gpui::FontWeight::SEMIBOLD,
                                                                    )
                                                                    .child(crate::tr!("用户名", "User name")),
                                                            )
                                                            .child(
                                                                div()
                                                                    .text_xs()
                                                                    .text_color(
                                                                        cx.theme()
                                                                            .muted_foreground,
                                                                    )
                                                                    .child(
                                                                        crate::tr!("可填写工号或昵称，不校验身份真实性，最多 64 个字符。", "Enter an employee ID or nickname, up to 64 characters. Identity is not verified."),
                                                                    ),
                                                            )
                                                            .child(
                                                                Input::new(
                                                                    &self.network_display_name,
                                                                )
                                                                .w_full()
                                                                .content_type(
                                                                    InputContentType::Username,
                                                                )
                                                                .accessibility_id("A392")
                                                                .aria_label(crate::tr!("云端用户名", "Cloud user name"))
                                                                .disabled(network_busy),
                                                            ),
                                                    )
                                                    .child(
                                                        h_flex()
                                                            .justify_end()
                                                            .child(
                                                                Button::new(
                                                                    "settings-network-connect",
                                                                )
                                                                .small()
                                                                .primary()
                                                                .label(crate::tr!("保存并测试", "Save and test"))
                                                                .loading(network_busy)
                                                                .disabled(network_busy)
                                                                .accessibility_id("A393")
                                                                .on_click(cx.listener(
                                                                    |this, _, window, cx| {
                                                                        this.connect_network(
                                                                            window, cx,
                                                                        )
                                                                    },
                                                                )),
                                                            ),
                                                    )
                                                    .child(
                                                        div()
                                                            .id("settings-network-status")
                                                            .text_sm()
                                                            .text_color(network_status_color)
                                                            .child(
                                                                self.network_status.clone(),
                                                            ),
                                                    ),
                                            )
                                        },
                                    )
                                    .when(
                                        has_matches
                                            && active_category == SettingsCategory::Scrolling,
                                        |content| {
                                            content.child(
                                                v_flex()
                    .id("settings-scroll-section")
                    .gap_3()
                    .p_3()
                    .rounded(cx.theme().radius)
                    .border_1()
                    .border_color(cx.theme().border)
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                div()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child(crate::tr!("滚动与动态效果", "Scrolling & motion")),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(crate::tr!("控制正文、当前结果和全局结果的鼠标滚轮行为。", "Controls mouse-wheel behavior in the log, current results, and global results.")),
                            ),
                    )
                    .child(
                        h_flex()
                            .justify_between()
                            .gap_4()
                            .child(div().text_sm().child(crate::tr!("按完整日志行滚动", "Scroll by complete log lines")))
                            .child(
                                Switch::new("settings-scroll-by-line")
                                    .small()
                                    .checked(self.draft.scroll_by_line)
                                    .tooltip(crate::tr!("按完整日志行滚动", "Scroll by complete log lines"))
                                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                        this.draft.scroll_by_line = *checked;
                                        SettingsDialog::draft_changed(cx);
                                    })),
                            ),
                    )
                    .when(self.draft.scroll_by_line, |section| {
                        section
                            .child(
                                h_flex()
                                    .justify_between()
                                    .gap_4()
                                    .child(
                                        v_flex()
                                            .gap_1()
                                            .child(
                                                div()
                                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                                    .child(crate::tr!("每次滚动行数", "Lines per scroll")),
                                            )
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .text_color(cx.theme().muted_foreground)
                                                    .child(crate::tr!("每次垂直滚轮输入移动 1–100 条逻辑日志。", "Move 1–100 logical log lines for each vertical wheel input.")),
                                            ),
                                    )
                                    .child(
                                        h_flex()
                                            .w_56()
                                            .gap_3()
                                            .child(Slider::new(&self.scroll_lines).flex_1())
                                            .child(
                                                div()
                                                    .w_10()
                                                    .text_right()
                                                    .text_sm()
                                                    .child(scroll_lines.to_string()),
                                            ),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .justify_between()
                                    .gap_4()
                                    .child(div().text_sm().child(crate::tr!("自动换行时仍按完整日志行滚动", "Scroll by complete log lines while wrapping")))
                                    .child(
                                        Switch::new("settings-scroll-by-line-word-wrap")
                                            .small()
                                            .checked(self.draft.scroll_by_line_when_word_wrap)
                                            .tooltip(crate::tr!("自动换行时按完整日志行滚动", "Scroll by complete log lines while wrapping"))
                                            .on_click(cx.listener(
                                                |this, checked: &bool, _, cx| {
                                                    this.draft.scroll_by_line_when_word_wrap =
                                                        *checked;
                                                    SettingsDialog::draft_changed(cx);
                                                },
                                            )),
                                    ),
                            )
                    })
                    .child(
                        h_flex()
                            .justify_between()
                            .gap_4()
                            .child(
                                v_flex()
                                    .gap_1()
                                    .child(
                                        div()
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .child(crate::tr!("像素滚动距离", "Pixel scroll distance")),
                                    )
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(crate::tr!("按设备原始输入的 1%–400% 调节像素滚动距离。", "Adjust pixel scrolling from 1% to 400% of the device’s raw input.")),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .w_56()
                                    .gap_3()
                                    .child(Slider::new(&self.scroll_percent).flex_1())
                                    .child(
                                        div()
                                            .w_10()
                                            .text_right()
                                            .text_sm()
                                            .child(format!("{scroll_percent}%")),
                                    ),
                            ),
                    )
                    .child(
                        h_flex()
                            .justify_between()
                            .gap_4()
                            .child(
                                v_flex()
                                    .gap_1()
                                    .child(
                                        div()
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .child(crate::tr!("分词边界字符", "Word-boundary characters")),
                                    )
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(crate::tr!("双击选词时，每个已配置字符与所有空白都会断开单词；最多 256 个 Unicode 字符。", "When selecting a word, every configured character and whitespace ends the word. Up to 256 Unicode characters.")),
                                    ),
                            )
                            .child(
                                div()
                                    .id("settings-word-boundary-characters")
                                    .w_72()
                                    .child(Input::new(&self.word_boundary_characters).w_full()),
                            ),
                    )
                    .child(
                        h_flex()
                            .justify_between()
                            .gap_4()
                            .child(
                                v_flex()
                                    .gap_1()
                                    .child(
                                        div()
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .child(crate::tr!("相邻行预读取", "Adjacent-line read-ahead")),
                                    )
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(
                                                crate::tr!("可见区域上下额外预读取并解码 4–40 行，组件仍只绘制当前虚拟范围。", "Read and decode 4–40 extra lines above and below the visible area while rendering only the current virtual range."),
                                            ),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .w_56()
                                    .gap_3()
                                    .child(Slider::new(&self.viewer_overscan).flex_1())
                                    .child(
                                        div()
                                            .w_10()
                                            .text_right()
                                            .text_sm()
                                            .child(viewer_overscan.to_string()),
                                    ),
                            ),
                    )
                    .child(
                        h_flex()
                            .justify_between()
                            .gap_4()
                            .child(div().text_sm().child(crate::tr!("减少动态效果", "Reduce motion")))
                            .child(
                                Switch::new("settings-reduce-motion")
                                    .small()
                                    .checked(self.draft.reduce_motion)
                                    .tooltip(crate::tr!("减少动态效果", "Reduce motion"))
                                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                        this.draft.reduce_motion = *checked;
                                        SettingsDialog::draft_changed(cx);
                                    })),
                            ),
                    ),
            )
                                        },
                                    )
                                    .when(
                                        has_matches
                                            && active_category == SettingsCategory::Storage,
                                        |content| {
                                            content.child(
                                                v_flex()
                    .id("settings-index-cache-section")
                    .gap_2()
                    .p_3()
                    .rounded(cx.theme().radius)
                    .border_1()
                    .border_color(cx.theme().border)
                    .child(
                        v_flex()
                            .gap_0p5()
                            .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child(crate::tr!("索引缓存", "Index cache")))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(crate::tr!("用于加快再次打开同一代文件；清理后会在需要时安全重建。", "Speeds up reopening the same file generation and is rebuilt safely when needed after cleanup.")),
                            ),
                    )
                    .child(
                        h_flex()
                            .justify_between()
                            .gap_4()
                            .child(
                                v_flex()
                                    .min_w_0()
                                    .gap_0p5()
                                    .child(cache_summary)
                                    .child(
                                        div()
                                            .truncate()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(cache_path),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(
                                        Button::new("settings-open-index-cache")
                                            .small()
                                            .ghost()
                                            .label(crate::tr!("打开文件夹", "Open folder"))
                                            .disabled(self.cache_busy || !cache_available)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.open_cache_directory(cx)
                                            })),
                                    )
                                    .child(
                                        Button::new("settings-clear-index-cache")
                                            .small()
                                            .ghost()
                                            .label(crate::tr!("清理缓存", "Clean cache"))
                                            .loading(self.cache_busy)
                                            .disabled(self.cache_busy || cache_empty || !cache_available)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.clear_cache(cx)
                                            })),
                                    ),
                            ),
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
                                        },
                                    )
                                    .when(
                                        has_matches
                                            && active_category == SettingsCategory::Advanced,
                                        |content| {
                                            content.child(
                                                v_flex()
                    .id("settings-application-log-section")
                    .gap_3()
                    .p_3()
                    .rounded(cx.theme().radius)
                    .border_1()
                    .border_color(cx.theme().border)
                    .child(
                        h_flex()
                            .justify_between()
                            .gap_4()
                            .child(
                                v_flex()
                                    .gap_0p5()
                                    .child(
                                        div()
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .child(crate::tr!("应用日志", "Application log")),
                                    )
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(
                                                crate::tr!("按所选最低等级打印并保留最近的诊断记录；关闭后不打印也不收集新日志。", "Print and retain recent diagnostics at or above the selected level. Off stops both printing and collection."),
                                            ),
                                    ),
                            )
                            .child(
                                div()
                                    .w_40()
                                    .child(Select::new(&self.app_log_level).small()),
                            ),
                    )
                    .child(
                        h_flex()
                            .justify_between()
                            .gap_4()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(crate::tr_args!(
                                        "当前缓冲区有 {log_entries} 条日志",
                                        "{log_entries} log entries are currently buffered",
                                    )),
                            )
                            .child(
                                Button::new("settings-export-application-log")
                                    .small()
                                    .outline()
                                    .label(crate::tr!("导出日志…", "Export log…"))
                                    .loading(self.log_export_task.is_some())
                                    .disabled(self.log_export_task.is_some())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.export_application_log(window, cx)
                                    })),
                            ),
                    )
                    .when_some(self.log_export_status.clone(), |section, status| {
                        section.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(status),
                        )
                    }),
            )
                                        },
                                    )
                                    .when(
                                        shortcuts_active,
                                        |content| {
                                            content.child(
                                                v_flex()
                    .id("settings-shortcut-section")
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .gap_2()
                    .child(
                        v_flex()
                            .gap_0p5()
                            .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child(crate::tr!("快捷键", "Shortcuts")))
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
                            .rounded(cx.theme().radius)
                            .border_1()
                            .border_color(cx.theme().border)
                            .children(ShortcutAction::ALL.into_iter().map(|action| {
                                self.render_shortcut_row(
                                    action,
                                    conflicts.get(&action).map(String::as_str),
                                    cx,
                                )
                            })),
                    ),
            )
                                        },
                                    )
                                    .when(
                                        has_matches
                                            && active_category == SettingsCategory::About,
                                        |content| content.child(self.render_about(cx)),
                                    ),
                            ),
                    ),
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

fn shortcut_from_event(event: &KeyDownEvent) -> String {
    let mut parts = Vec::with_capacity(5);
    if event.keystroke.modifiers.control {
        parts.push("Ctrl".to_string());
    }
    if event.keystroke.modifiers.alt {
        parts.push("Alt".to_string());
    }
    if event.keystroke.modifiers.platform {
        parts.push(
            if cfg!(target_os = "macos") {
                "Cmd"
            } else {
                "Meta"
            }
            .to_string(),
        );
    }
    if event.keystroke.modifiers.shift {
        parts.push("Shift".to_string());
    }

    let key = match event.keystroke.key.as_str() {
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
