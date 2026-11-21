use super::*;

#[derive(Clone, Copy)]
pub(super) struct SearchOption {
    pub(super) category: SettingsCategory,
    pub(super) key: &'static str,
    zh: &'static str,
    en: &'static str,
    aliases: &'static str,
}

const fn option(
    category: SettingsCategory,
    key: &'static str,
    zh: &'static str,
    en: &'static str,
    aliases: &'static str,
) -> SearchOption {
    SearchOption {
        category,
        key,
        zh,
        en,
        aliases,
    }
}

macro_rules! entry {
    ($category:ident, $key:literal, $zh:literal, $en:literal, $aliases:literal) => {
        option(SettingsCategory::$category, $key, $zh, $en, $aliases)
    };
}

const OPTIONS: &[SearchOption] = &[
    entry!(
        General,
        "language",
        "界面语言",
        "Language",
        "中文 英语 简体中文 English Chinese"
    ),
    entry!(
        General,
        "theme",
        "界面主题",
        "Interface theme",
        "浅色 深色 跟随系统 主题 light dark system"
    ),
    entry!(General, "icon", "应用图标", "App icon", ""),
    entry!(
        General,
        "motion",
        "减少动态效果",
        "Reduce motion",
        "动画 animation"
    ),
    entry!(
        General,
        "full-path",
        "显示完整路径",
        "Show full path",
        "文件工具栏 file toolbar"
    ),
    entry!(
        General,
        "close-tab",
        "关闭日志标签前确认",
        "Confirm before closing log tabs",
        "关闭确认"
    ),
    entry!(
        General,
        "paste",
        "粘贴时确认",
        "Confirm clipboard paste",
        "剪贴板"
    ),
    entry!(
        General,
        "directory",
        "打开目录命令",
        "Open-folder command",
        "打开文件夹 directory path"
    ),
    #[cfg(windows)]
    entry!(
        General,
        "explorer",
        "文件右键菜单",
        "Windows context menu",
        "Explorer 资源管理器"
    ),
    entry!(
        Reading,
        "wrap",
        "新文件默认换行",
        "Wrap new files by default",
        "自动换行 word wrap long lines"
    ),
    entry!(Reading, "font", "日志字体", "Log font", "字体系列 family"),
    entry!(Reading, "size", "日志字号", "Log font size", "字体大小"),
    entry!(
        Reading,
        "spacing",
        "日志行距",
        "Log line spacing",
        "行高 line height"
    ),
    entry!(
        Reading,
        "numbers",
        "显示行号",
        "Show line numbers",
        "行号栏"
    ),
    entry!(
        Reading,
        "number-width",
        "行号栏宽度",
        "Line-number column width",
        "行号宽度"
    ),
    entry!(
        Reading,
        "number-ink",
        "行号文字颜色",
        "Line-number text color",
        "行号颜色 行号字体颜色 line number color foreground"
    ),
    entry!(
        Reading,
        "number-bg",
        "行号背景色",
        "Line-number background",
        "行号底色 行号背景颜色 line number background color"
    ),
    entry!(
        Reading,
        "number-lines",
        "行号行间分隔线",
        "Line-number separators",
        "行号分隔线"
    ),
    entry!(
        Reading,
        "row-lines",
        "日志分隔线",
        "Log separators",
        "行间分隔线 row separators"
    ),
    entry!(
        Reading,
        "log-ink",
        "日志文字颜色",
        "Log text color",
        "正文颜色 字体颜色 text foreground"
    ),
    entry!(
        Reading,
        "log-bg",
        "日志背景色",
        "Log background",
        "日志背景颜色 正文背景色 background color"
    ),
    entry!(
        Reading,
        "horizontal",
        "显示横向滚动条",
        "Show horizontal scrollbar",
        "水平滚动条"
    ),
    entry!(
        Reading,
        "line-scroll",
        "按完整日志行滚动",
        "Scroll by complete log lines",
        "整行滚动 鼠标滚轮 mouse wheel"
    ),
    entry!(
        Reading,
        "scroll-count",
        "每次滚动行数",
        "Lines per scroll",
        "滚轮行数"
    ),
    entry!(
        Reading,
        "wrap-scroll",
        "自动换行时仍按行滚动",
        "Scroll by line while wrapping",
        "按完整日志行滚动"
    ),
    entry!(
        Reading,
        "pixel",
        "像素滚动距离",
        "Pixel scroll distance",
        "滚动速度 滚轮灵敏度 speed"
    ),
    entry!(
        Reading,
        "words",
        "分词边界字符",
        "Word-boundary characters",
        "双击选词 double click"
    ),
    entry!(
        Highlight,
        "labels",
        "颜色标签",
        "Color labels",
        "关键词颜色 keyword colors"
    ),
    entry!(
        Highlight,
        "selection",
        "选中样式",
        "Selection style",
        "选区 行选择 文字选择 边框 圆角 失去焦点 selected text border corners"
    ),
    entry!(
        Highlight,
        "matches",
        "搜索匹配样式",
        "Search match styles",
        "页内查找 匹配颜色 快速查找 quick find underline"
    ),
    entry!(
        Highlight,
        "selection-row-background",
        "整行选区背景色",
        "Selected row background",
        "选中样式"
    ),
    entry!(
        Highlight,
        "selection-row-foreground",
        "整行选区文字颜色",
        "Selected row text color",
        "选中样式"
    ),
    entry!(
        Highlight,
        "selection-border",
        "选区边框",
        "Selection border",
        "选中样式"
    ),
    entry!(
        Highlight,
        "selection-border-color",
        "选区边框颜色",
        "Selection border color",
        "选中样式"
    ),
    entry!(
        Highlight,
        "selection-weight",
        "选区边框粗细",
        "Selection border weight",
        "选中样式"
    ),
    entry!(
        Highlight,
        "selection-radius",
        "选区圆角",
        "Selection corners",
        "选中样式"
    ),
    entry!(
        Highlight,
        "selection-text-background",
        "文字选区背景色",
        "Selected text background",
        "选中样式"
    ),
    entry!(
        Highlight,
        "selection-text-foreground",
        "文字选区文字颜色",
        "Selected text color",
        "选中样式"
    ),
    entry!(
        Highlight,
        "selection-underline",
        "文字选区下划线",
        "Selected text underline",
        "选中样式"
    ),
    entry!(
        Highlight,
        "selection-dim-inactive",
        "失去焦点时淡化选区",
        "Dim unfocused selection",
        "选中样式"
    ),
    entry!(
        Highlight,
        "selection-inactive-opacity",
        "失焦选区保留强度",
        "Unfocused selection opacity",
        "选中样式"
    ),
    entry!(
        Highlight,
        "match-foreground",
        "搜索匹配文字颜色",
        "Search match text color",
        "匹配样式"
    ),
    entry!(
        Highlight,
        "match-background",
        "搜索匹配背景颜色",
        "Search match background color",
        "匹配样式"
    ),
    entry!(
        Highlight,
        "match-bold",
        "搜索匹配加粗",
        "Search match bold",
        "匹配样式"
    ),
    entry!(
        Highlight,
        "match-italic",
        "搜索匹配斜体",
        "Search match italic",
        "匹配样式"
    ),
    entry!(
        Highlight,
        "match-underline",
        "搜索匹配下划线",
        "Search match underline",
        "匹配样式"
    ),
    entry!(
        Highlight,
        "quick-foreground",
        "页内查找文字颜色",
        "Quick find text color",
        "匹配样式"
    ),
    entry!(
        Highlight,
        "quick-background",
        "页内查找背景颜色",
        "Quick find background color",
        "匹配样式"
    ),
    entry!(
        Highlight,
        "quick-bold",
        "页内查找加粗",
        "Quick find bold",
        "匹配样式"
    ),
    entry!(
        Highlight,
        "quick-italic",
        "页内查找斜体",
        "Quick find italic",
        "匹配样式"
    ),
    entry!(
        Highlight,
        "quick-underline",
        "页内查找下划线",
        "Quick find underline",
        "匹配样式"
    ),
    entry!(
        Highlight,
        "levels",
        "日志级别着色",
        "Log level coloring",
        "日志着色 着色分组 启用着色 规则 groups rules regex levels"
    ),
    entry!(
        Search,
        "case",
        "区分大小写",
        "Case-sensitive search",
        "case sensitivity"
    ),
    entry!(
        Search,
        "regex",
        "使用正则表达式",
        "Use regular expressions",
        "regex 正则"
    ),
    entry!(
        Search,
        "limit",
        "每个文件的结果上限",
        "Result limit per file",
        "最大搜索结果数 maximum search results"
    ),
    entry!(
        Search,
        "hits",
        "高亮已提交搜索的匹配文字",
        "Highlight search matches",
        "命中高亮"
    ),
    entry!(
        Search,
        "height",
        "共用控件高度",
        "Shared control height",
        "工具栏高度 搜索栏高度 toolbar height search input height"
    ),
    entry!(
        Search,
        "toolbar-font",
        "工具栏字号",
        "Toolbar text size",
        "工具栏字体大小 toolbar font size"
    ),
    entry!(
        Search,
        "input-font",
        "输入框字号",
        "Search field text size",
        "输入框字体大小 search input font size"
    ),
    entry!(
        History,
        "files",
        "文件记录",
        "File entries",
        "最近文件 recent files"
    ),
    entry!(
        History,
        "temp",
        "临时搜索结果",
        "Temporary search results",
        "临时文件"
    ),
    entry!(
        History,
        "history",
        "搜索历史",
        "Search history",
        "清除搜索记录 清空历史 删除历史 clear history"
    ),
    entry!(
        History,
        "cache",
        "索引缓存",
        "Index cache",
        "缓存大小 清理缓存 清空缓存 打开缓存目录 clear cache folder"
    ),
    entry!(
        Network,
        "server",
        "服务器地址",
        "Server address",
        "服务器网址 URL HTTP HTTPS"
    ),
    entry!(
        Network,
        "name",
        "云端用户名",
        "Cloud user name",
        "工号 昵称 nickname"
    ),
    entry!(
        Network,
        "connection",
        "测试连接",
        "Test connection",
        "保存并测试 cookie connect"
    ),
    entry!(
        Diagnostics,
        "app-log",
        "应用日志等级",
        "Application log level",
        "诊断日志 debug error warn info off"
    ),
    entry!(
        Diagnostics,
        "log-clear",
        "清理应用日志",
        "Clear application logs",
        "清理日志"
    ),
    entry!(
        Diagnostics,
        "log-export",
        "导出应用日志",
        "Export application log",
        "导出日志"
    ),
    entry!(
        Diagnostics,
        "overlay",
        "性能叠加层",
        "Performance overlay",
        "帧耗时 FPS P99"
    ),
    entry!(
        Diagnostics,
        "trace",
        "性能录制",
        "Performance recording",
        "性能跟踪 开始录制 停止录制 trace perfetto"
    ),
    entry!(
        About,
        "build",
        "版本与构建",
        "Version and build",
        "编译时间 构建配置 提交 commit"
    ),
    entry!(About, "stack", "技术栈", "Technology", "技术架构"),
    entry!(
        About,
        "libs",
        "主要开源库",
        "Open-source libraries",
        "开源组件 GPUI SQLite"
    ),
    entry!(About, "repo", "项目仓库", "Project repository", "GitHub"),
    entry!(
        About,
        "license",
        "作者与许可",
        "Author and licenses",
        "许可证 copyright"
    ),
    entry!(
        Shortcuts,
        "key-open-file",
        "打开文件",
        "Open file",
        "快捷键 按键绑定 shortcut key binding"
    ),
    entry!(
        Shortcuts,
        "key-close-tab",
        "关闭当前标签",
        "Close current tab",
        "快捷键 按键绑定 shortcut key binding"
    ),
    entry!(
        Shortcuts,
        "key-open-settings",
        "打开设置",
        "Open settings",
        "快捷键 按键绑定 shortcut key binding"
    ),
    entry!(
        Shortcuts,
        "key-focus-search",
        "聚焦搜索框",
        "Focus search",
        "快捷键 按键绑定 shortcut key binding"
    ),
    entry!(
        Shortcuts,
        "key-quick-find",
        "快速查找",
        "Quick find",
        "快捷键 按键绑定 shortcut key binding"
    ),
    entry!(
        Shortcuts,
        "key-toggle-case",
        "切换区分大小写",
        "Toggle case sensitivity",
        "快捷键 按键绑定 shortcut key binding"
    ),
    entry!(
        Shortcuts,
        "key-jump-bottom",
        "跳到日志底部",
        "Jump to end",
        "快捷键 按键绑定 shortcut key binding"
    ),
    entry!(
        Shortcuts,
        "key-toggle-word-wrap",
        "切换自动换行",
        "Toggle word wrap",
        "快捷键 按键绑定 shortcut key binding"
    ),
    entry!(
        Shortcuts,
        "key-cycle-color-label",
        "轮换颜色标签",
        "Cycle color label",
        "快捷键 按键绑定 shortcut key binding"
    ),
    entry!(
        Shortcuts,
        "key-add-text-mark",
        "新增文字标记",
        "New text mark",
        "快捷键 按键绑定 shortcut key binding"
    ),
    entry!(
        Shortcuts,
        "key-enter-edit-mode",
        "进入编辑模式",
        "Enter edit mode",
        "快捷键 按键绑定 shortcut key binding"
    ),
];

impl SearchOption {
    pub(super) fn label(self) -> &'static str {
        crate::tr!(self.zh, self.en)
    }
}

pub(super) fn results(query: &str) -> Vec<SearchOption> {
    let query = query.trim().to_lowercase().replace('-', " ");
    if query.is_empty() {
        return Vec::new();
    }
    let mut hits = Vec::new();
    for &option in OPTIONS {
        let en = option.en.to_lowercase().replace('-', " ");
        let category = option.category.label().to_lowercase();
        let text = format!("{} {} {} {}", option.zh, en, option.aliases, category).to_lowercase();
        if query.split_whitespace().all(|term| text.contains(term)) {
            let rank = if query == option.zh || query == en {
                0
            } else if option.zh.contains(&query) || en.contains(&query) {
                1
            } else {
                2
            };
            hits.push((rank, option));
        }
    }
    hits.sort_by_key(|(rank, _)| *rank);
    hits.into_iter().map(|(_, option)| option).collect()
}

/// Keeps a located control visible inside its own scroll container without moving keyboard focus.
#[derive(Default)]
pub(crate) struct OptionLocation {
    key: Option<&'static str>,
    anchor: Option<gpui_kit::ScrollAnchor>,
    pending: bool,
}

impl OptionLocation {
    pub(crate) fn select(&mut self, key: Option<&'static str>, scroll: &gpui_kit::ScrollHandle) {
        self.key = key;
        self.anchor = key.map(|_| gpui_kit::ScrollAnchor::for_handle(scroll.clone()));
        self.pending = key.is_some();
    }

    pub(crate) fn is(&self, key: &str) -> bool {
        self.key == Some(key)
    }

    pub(crate) fn reveal(&mut self, window: &mut Window, cx: &mut gpui_kit::App) {
        if std::mem::take(&mut self.pending)
            && let Some(anchor) = &self.anchor
        {
            anchor.scroll_to(window, cx);
            window.on_next_frame(|window, _| window.refresh());
        }
    }

    pub(crate) fn mark(
        &self,
        key: &str,
        control: impl IntoElement,
        cx: &gpui_kit::App,
    ) -> AnyElement {
        if !self.is(key) {
            return control.into_any_element();
        }
        div()
            .id("settings-located-option")
            .anchor_scroll(self.anchor.clone())
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().ring)
            .bg(cx.theme().accent.opacity(0.35))
            .child(control)
            .into_any_element()
    }
}
