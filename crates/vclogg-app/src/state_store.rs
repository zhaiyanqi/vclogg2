use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};

#[cfg(windows)]
use std::os::windows::ffi::{OsStrExt as _, OsStringExt as _};

use anyhow::{Context as _, Result};
use rusqlite::{Connection, OptionalExtension as _, params, params_from_iter};

use crate::app_log::AppLogLevel;
use crate::color_labels::{
    ColorLabel, KeywordColorRule, decode_rules, default_color_labels, encode_rules,
};
use crate::i18n::Language;
use crate::predefined_filters::{
    PredefinedFilter, normalize_predefined_filters, parse_stored_filter,
};
use crate::search_context::WorkspaceSearchState;
use crate::tab_resume::TabResumeState;

pub(crate) fn normalize_search_history(history: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = HashSet::new();
    history
        .into_iter()
        .filter(|query| !query.is_empty() && seen.insert(query.clone()))
        .collect()
}

pub const DEFAULT_WORD_BOUNDARY_CHARACTERS: &str =
    ".,;:!?()[]{}<>/\\|\"'`~@#$%^&*+-=，。！？；：、（）【】《》“”‘’…—";
pub const MAX_WORD_BOUNDARY_CHARACTERS: usize = 256;
const STATE_SCHEMA_VERSION: u32 = 4;
const ENCODED_PATH_PREFIX: &str = "\0vclogg-path-v1:";

#[derive(Clone, Debug)]
pub struct RecentFile {
    pub id: i64,
    pub path: PathBuf,
    pub last_opened_at: i64,
}

#[derive(Clone, Debug)]
pub struct HistorySession {
    pub id: i64,
    pub path: PathBuf,
    pub last_opened_at: i64,
    pub revision: i64,
    pub selected_row: Option<usize>,
    pub query_text: String,
    pub marked_rows_count: usize,
    pub pinned: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DatabaseInfo {
    pub byte_size: u64,
    pub session_count: usize,
}

#[derive(Clone, Debug)]
pub struct LastWorkspaceFile {
    pub id: i64,
    pub path: PathBuf,
    pub last_opened_at: i64,
    pub was_active: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileSessionState {
    pub revision: i64,
    pub custom_title: Option<String>,
    pub selected_row: Option<usize>,
    pub query_text: String,
    pub case_sensitive: bool,
    pub regex: bool,
    pub result_mode: i64,
    pub marked_rows: Vec<usize>,
    pub show_line_numbers: bool,
    pub show_row_separators: bool,
    pub word_wrap: bool,
    pub keyword_color_rules: Vec<KeywordColorRule>,
    pub resume: TabResumeState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionSaveResult {
    pub state: FileSessionState,
    pub conflict_resolved: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShortcutSettings {
    pub open_file: String,
    pub focus_search: String,
    pub quick_find: String,
    pub close_tab: String,
    pub open_settings: String,
    pub toggle_case_sensitive: String,
    pub jump_to_bottom: String,
    pub cycle_color_label: String,
    pub toggle_word_wrap: String,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LogFontFamily {
    CascadiaMono,
    JetBrainsMono,
    #[default]
    Consolas,
    SystemMonospace,
}

impl LogFontFamily {
    pub const ALL: [Self; 4] = [
        Self::CascadiaMono,
        Self::JetBrainsMono,
        Self::Consolas,
        Self::SystemMonospace,
    ];

    pub fn database_value(self) -> &'static str {
        match self {
            Self::CascadiaMono => "cascadia-mono",
            Self::JetBrainsMono => "jetbrains-mono",
            Self::Consolas => "consolas",
            Self::SystemMonospace => "system-monospace",
        }
    }

    fn from_database(value: &str) -> Self {
        match value {
            "cascadia-mono" => Self::CascadiaMono,
            "jetbrains-mono" => Self::JetBrainsMono,
            "consolas" => Self::Consolas,
            _ => Self::SystemMonospace,
        }
    }

    pub fn select_index(self) -> usize {
        Self::ALL
            .iter()
            .position(|candidate| *candidate == self)
            .unwrap_or(3)
    }
}

#[cfg(test)]
mod log_font_family_tests {
    use super::{AppSettings, LogFontFamily};

    #[test]
    fn defaults_to_consolas() {
        assert_eq!(LogFontFamily::default(), LogFontFamily::Consolas);
        assert_eq!(
            AppSettings::default().log_font_family,
            LogFontFamily::Consolas
        );
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ThemePreference {
    System,
    #[default]
    Light,
    Dark,
}

impl ThemePreference {
    pub const ALL: [Self; 3] = [Self::System, Self::Light, Self::Dark];

    pub fn database_value(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    fn from_database(value: &str) -> Self {
        match value {
            "system" => Self::System,
            "dark" => Self::Dark,
            _ => Self::Light,
        }
    }

    pub fn select_index(self) -> usize {
        Self::ALL
            .iter()
            .position(|candidate| *candidate == self)
            .unwrap_or(1)
    }
}

#[cfg(test)]
mod theme_preference_tests {
    use super::ThemePreference;

    #[test]
    fn removed_glass_preference_falls_back_to_light() {
        assert_eq!(
            ThemePreference::from_database("glass"),
            ThemePreference::Light
        );
        assert_eq!(ThemePreference::ALL.len(), 3);
    }
}

impl Default for ShortcutSettings {
    fn default() -> Self {
        let primary = if cfg!(target_os = "macos") {
            "Cmd"
        } else {
            "Ctrl"
        };
        Self {
            open_file: format!("{primary}+O"),
            focus_search: format!("{primary}+F"),
            quick_find: format!("{primary}+Shift+F"),
            close_tab: format!("{primary}+W"),
            open_settings: format!("{primary}+,"),
            toggle_case_sensitive: "Alt+C".into(),
            jump_to_bottom: format!("{primary}+End"),
            cycle_color_label: format!("{primary}+D"),
            toggle_word_wrap: "W".into(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppSettings {
    pub app_log_level: AppLogLevel,
    pub language: Language,
    pub theme_preference: ThemePreference,
    pub default_show_line_numbers: bool,
    pub default_show_row_separators: bool,
    pub show_line_number_row_separators: bool,
    pub line_number_width: u16,
    pub line_number_text_color: Option<String>,
    pub line_number_background_color: Option<String>,
    pub highlight_log_levels: bool,
    pub log_font_size: u16,
    pub log_line_spacing: u16,
    pub log_font_family: LogFontFamily,
    pub mouse_wheel_scroll_percent: u16,
    pub scroll_by_line: bool,
    pub mouse_wheel_scroll_lines: u16,
    pub scroll_by_line_when_word_wrap: bool,
    pub viewer_overscan: u16,
    pub reduce_motion: bool,
    pub confirm_close_tab: bool,
    pub show_full_path: bool,
    pub open_directory_command: String,
    pub max_search_results: u32,
    pub highlight_matches: bool,
    pub word_boundary_characters: String,
    pub default_case_sensitive: bool,
    pub default_use_regex: bool,
    pub shortcuts: ShortcutSettings,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CloudSettings {
    pub server_url: String,
    pub display_name: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            app_log_level: AppLogLevel::default(),
            language: Language::Chinese,
            theme_preference: ThemePreference::Light,
            default_show_line_numbers: true,
            default_show_row_separators: false,
            show_line_number_row_separators: false,
            line_number_width: 60,
            line_number_text_color: None,
            line_number_background_color: None,
            highlight_log_levels: false,
            log_font_size: 13,
            log_line_spacing: 6,
            log_font_family: LogFontFamily::Consolas,
            mouse_wheel_scroll_percent: 100,
            scroll_by_line: false,
            mouse_wheel_scroll_lines: 1,
            scroll_by_line_when_word_wrap: false,
            viewer_overscan: 12,
            reduce_motion: false,
            confirm_close_tab: false,
            show_full_path: true,
            open_directory_command: String::new(),
            max_search_results: 0,
            highlight_matches: true,
            word_boundary_characters: DEFAULT_WORD_BOUNDARY_CHARACTERS.to_string(),
            default_case_sensitive: false,
            default_use_regex: false,
            shortcuts: ShortcutSettings::default(),
        }
    }
}

impl AppSettings {
    pub fn search_result_limit(&self) -> Option<usize> {
        usize::try_from(self.max_search_results)
            .ok()
            .filter(|limit| *limit > 0)
    }
}

impl Default for FileSessionState {
    fn default() -> Self {
        Self {
            revision: 0,
            custom_title: None,
            selected_row: None,
            query_text: String::new(),
            case_sensitive: false,
            regex: false,
            result_mode: 0,
            marked_rows: Vec::new(),
            show_line_numbers: true,
            show_row_separators: false,
            word_wrap: false,
            keyword_color_rules: Vec::new(),
            resume: TabResumeState::default(),
        }
    }
}

pub struct StateStore {
    connection: Mutex<Connection>,
    database_path: PathBuf,
}

impl StateStore {
    pub fn open_default() -> Result<Self> {
        let data_root = crate::app_paths::data_local_dir().context("无法确定本机应用数据目录")?;
        Self::open(
            data_root
                .join("VCLogg2")
                .join("sessions")
                .join("vclogg2-state.db"),
        )
    }

    fn open(database_path: PathBuf) -> Result<Self> {
        let sessions_dir = database_path.parent().context("状态库路径没有父目录")?;
        fs::create_dir_all(sessions_dir)
            .with_context(|| format!("无法创建状态目录：{}", sessions_dir.display()))?;
        let connection = Connection::open(&database_path)
            .with_context(|| format!("无法打开状态库：{}", database_path.display()))?;
        connection
            .busy_timeout(std::time::Duration::from_secs(5))
            .context("无法设置状态库忙等待")?;
        connection
            .execute_batch("PRAGMA foreign_keys = ON;")
            .context("无法启用状态库外键")?;
        let schema_version = connection
            .query_row("PRAGMA user_version", [], |row| row.get::<_, u32>(0))
            .context("无法读取状态库版本")?;
        if schema_version > STATE_SCHEMA_VERSION {
            anyhow::bail!("状态库版本 {schema_version} 高于当前支持的 {STATE_SCHEMA_VERSION}");
        }
        if schema_version < STATE_SCHEMA_VERSION {
            connection
            .execute_batch(
                "PRAGMA journal_mode = WAL;
                 CREATE TABLE IF NOT EXISTS file_sessions (
                     id INTEGER PRIMARY KEY,
                     path TEXT NOT NULL UNIQUE,
                     custom_title TEXT,
                     last_opened_at INTEGER NOT NULL,
                     revision INTEGER NOT NULL DEFAULT 1,
                     selected_row INTEGER,
                     query_text TEXT NOT NULL DEFAULT '',
                     case_sensitive INTEGER NOT NULL DEFAULT 0,
                     regex INTEGER NOT NULL DEFAULT 0,
                     result_mode INTEGER NOT NULL DEFAULT 0,
                     marked_rows TEXT NOT NULL DEFAULT '',
                     show_line_numbers INTEGER NOT NULL DEFAULT 1,
                     show_row_separators INTEGER NOT NULL DEFAULT 0,
                     word_wrap INTEGER NOT NULL DEFAULT 0,
                     keyword_color_rules TEXT NOT NULL DEFAULT '[]',
                     resume_state TEXT NOT NULL DEFAULT '',
                     pinned INTEGER NOT NULL DEFAULT 0
                 );
                 CREATE INDEX IF NOT EXISTS idx_file_sessions_recent
                     ON file_sessions(last_opened_at DESC);
                 CREATE TABLE IF NOT EXISTS last_workspace_files (
                     position INTEGER PRIMARY KEY,
                     file_session_id INTEGER NOT NULL UNIQUE
                         REFERENCES file_sessions(id) ON DELETE CASCADE,
                     was_active INTEGER NOT NULL DEFAULT 0
                 );
                 CREATE TABLE IF NOT EXISTS app_settings (
                     id INTEGER PRIMARY KEY CHECK(id = 1),
                     default_show_line_numbers INTEGER NOT NULL DEFAULT 1,
                     default_show_row_separators INTEGER NOT NULL DEFAULT 0,
                     highlight_log_levels INTEGER NOT NULL DEFAULT 0,
                     log_font_size INTEGER NOT NULL DEFAULT 13,
                     log_line_spacing INTEGER NOT NULL DEFAULT 6,
                     log_font_family TEXT NOT NULL DEFAULT 'consolas',
                     shortcut_open_file TEXT NOT NULL DEFAULT 'Ctrl+O',
                     shortcut_focus_search TEXT NOT NULL DEFAULT 'Ctrl+F',
                     shortcut_quick_find TEXT NOT NULL DEFAULT 'Ctrl+Shift+F',
                     shortcut_close_tab TEXT NOT NULL DEFAULT 'Ctrl+W',
                     shortcut_open_settings TEXT NOT NULL DEFAULT 'Ctrl+,',
                     shortcut_toggle_case_sensitive TEXT NOT NULL DEFAULT 'Alt+C',
                     shortcut_jump_to_bottom TEXT NOT NULL DEFAULT 'Ctrl+End',
                     shortcut_cycle_color_label TEXT NOT NULL DEFAULT 'Ctrl+D'
                     ,shortcut_toggle_word_wrap TEXT NOT NULL DEFAULT 'W',
                     mouse_wheel_scroll_percent INTEGER NOT NULL DEFAULT 100,
                     scroll_by_line INTEGER NOT NULL DEFAULT 0,
                     mouse_wheel_scroll_lines INTEGER NOT NULL DEFAULT 1,
                     scroll_by_line_when_word_wrap INTEGER NOT NULL DEFAULT 0,
                     reduce_motion INTEGER NOT NULL DEFAULT 0,
                     confirm_close_tab INTEGER NOT NULL DEFAULT 0,
                     show_full_path INTEGER NOT NULL DEFAULT 1,
                     max_search_results INTEGER NOT NULL DEFAULT 0,
                     highlight_matches INTEGER NOT NULL DEFAULT 1,
                     word_boundary_characters TEXT NOT NULL DEFAULT '.,;:!?()[]{}<>/\\|\"''`~@#$%^&*+-=，。！？；：、（）【】《》“”‘’…—',
                     default_case_sensitive INTEGER NOT NULL DEFAULT 0,
                     default_use_regex INTEGER NOT NULL DEFAULT 0,
                     show_line_number_row_separators INTEGER NOT NULL DEFAULT 0,
                     line_number_width INTEGER NOT NULL DEFAULT 60,
                     line_number_text_color TEXT,
                     line_number_background_color TEXT,
                     theme_preference TEXT NOT NULL DEFAULT 'light',
                     open_directory_command TEXT NOT NULL DEFAULT '',
                     viewer_overscan INTEGER NOT NULL DEFAULT 12,
                     language TEXT NOT NULL DEFAULT 'zh-CN',
                     app_log_level TEXT NOT NULL DEFAULT 'error'
                 );
                 CREATE TABLE IF NOT EXISTS ui_state (
                     key TEXT PRIMARY KEY,
                     value TEXT NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS global_search_preferences (
                     path TEXT PRIMARY KEY,
                     selected INTEGER NOT NULL DEFAULT 1
                 );
                 CREATE TABLE IF NOT EXISTS search_history (
                     position INTEGER PRIMARY KEY,
                     query_text TEXT NOT NULL UNIQUE
                 );
                 CREATE TABLE IF NOT EXISTS color_labels (
                     position INTEGER PRIMARY KEY,
                     label_id TEXT NOT NULL UNIQUE,
                     name TEXT NOT NULL,
                     color INTEGER NOT NULL,
                     alpha INTEGER NOT NULL DEFAULT 255
                 );
                 CREATE TABLE IF NOT EXISTS color_label_settings (
                     id INTEGER PRIMARY KEY CHECK(id = 1),
                     initialized INTEGER NOT NULL DEFAULT 0
                 );
                 CREATE TABLE IF NOT EXISTS predefined_filters (
                     position INTEGER PRIMARY KEY,
                     filter_id TEXT NOT NULL UNIQUE,
                     item_json TEXT NOT NULL
                 );
                 CREATE TABLE IF NOT EXISTS cloud_settings (
                     id INTEGER PRIMARY KEY CHECK(id = 1),
                     server_url TEXT NOT NULL DEFAULT '',
                     display_name TEXT NOT NULL DEFAULT ''
                 );",
            )
            .context("无法初始化状态库结构")?;
            ensure_session_columns(&connection)?;
            ensure_app_settings_columns(&connection)?;
            ensure_color_label_columns(&connection)?;
            connection
                .execute(
                    "CREATE INDEX IF NOT EXISTS idx_file_sessions_pinned_recent
                     ON file_sessions(pinned DESC, last_opened_at DESC)",
                    [],
                )
                .context("无法创建收藏文件索引")?;
            connection
                .pragma_update(None, "user_version", STATE_SCHEMA_VERSION)
                .context("无法更新状态库版本")?;
        }

        Ok(Self {
            connection: Mutex::new(connection),
            database_path,
        })
    }

    pub fn record_opened(&self, paths: &[PathBuf]) -> Result<()> {
        if paths.is_empty() {
            return Ok(());
        }
        let mut connection = self.lock()?;
        let transaction = connection.transaction().context("无法开始最近文件事务")?;
        let opened_at = unix_timestamp();
        for path in paths {
            transaction
                .execute(
                    "INSERT INTO file_sessions(path, last_opened_at)
                     VALUES (?1, ?2)
                     ON CONFLICT(path) DO UPDATE SET
                         last_opened_at = excluded.last_opened_at,
                         revision = file_sessions.revision + 1",
                    params![path_to_database(path), opened_at],
                )
                .with_context(|| format!("无法记录最近文件：{}", path.display()))?;
        }
        transaction.commit().context("无法提交最近文件事务")
    }

    pub fn load_app_settings(&self) -> Result<AppSettings> {
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT default_show_line_numbers, default_show_row_separators,
                        highlight_log_levels, log_font_size, log_line_spacing, log_font_family,
                        shortcut_open_file, shortcut_focus_search, shortcut_quick_find,
                        shortcut_close_tab, shortcut_open_settings,
                        shortcut_toggle_case_sensitive, shortcut_jump_to_bottom,
                        shortcut_cycle_color_label, shortcut_toggle_word_wrap,
                        mouse_wheel_scroll_percent, scroll_by_line,
                        mouse_wheel_scroll_lines, scroll_by_line_when_word_wrap,
                        reduce_motion, confirm_close_tab, show_full_path,
                        max_search_results, highlight_matches, word_boundary_characters,
                        default_case_sensitive, default_use_regex,
                        show_line_number_row_separators, line_number_width,
                        line_number_text_color, line_number_background_color,
                        theme_preference, open_directory_command, viewer_overscan, language,
                        app_log_level
                 FROM app_settings
                 WHERE id = 1",
                [],
                |row| {
                    Ok(AppSettings {
                        app_log_level: AppLogLevel::from_database(&row.get::<_, String>(35)?),
                        language: Language::from_database(&row.get::<_, String>(34)?),
                        theme_preference: ThemePreference::from_database(
                            &row.get::<_, String>(31)?,
                        ),
                        open_directory_command: row
                            .get::<_, String>(32)?
                            .chars()
                            .take(2048)
                            .collect(),
                        viewer_overscan: row.get::<_, u16>(33)?.clamp(4, 40),
                        default_show_line_numbers: row.get::<_, i64>(0)? != 0,
                        default_show_row_separators: row.get::<_, i64>(1)? != 0,
                        highlight_log_levels: row.get::<_, i64>(2)? != 0,
                        log_font_size: row.get::<_, u16>(3)?.clamp(8, 32),
                        log_line_spacing: row.get::<_, u16>(4)?.clamp(1, 40),
                        log_font_family: LogFontFamily::from_database(&row.get::<_, String>(5)?),
                        mouse_wheel_scroll_percent: row.get::<_, u16>(15)?.clamp(1, 400),
                        scroll_by_line: row.get::<_, i64>(16)? != 0,
                        mouse_wheel_scroll_lines: row.get::<_, u16>(17)?.clamp(1, 100),
                        scroll_by_line_when_word_wrap: row.get::<_, i64>(18)? != 0,
                        reduce_motion: row.get::<_, i64>(19)? != 0,
                        confirm_close_tab: row.get::<_, i64>(20)? != 0,
                        show_full_path: row.get::<_, i64>(21)? != 0,
                        max_search_results: u32::try_from(
                            row.get::<_, i64>(22)?.clamp(0, i64::from(u32::MAX)),
                        )
                        .unwrap_or(u32::MAX),
                        highlight_matches: row.get::<_, i64>(23)? != 0,
                        word_boundary_characters: row
                            .get::<_, String>(24)?
                            .chars()
                            .take(MAX_WORD_BOUNDARY_CHARACTERS)
                            .collect(),
                        default_case_sensitive: row.get::<_, i64>(25)? != 0,
                        default_use_regex: row.get::<_, i64>(26)? != 0,
                        show_line_number_row_separators: row.get::<_, i64>(27)? != 0,
                        line_number_width: row.get::<_, u16>(28)?.clamp(40, 160),
                        line_number_text_color: normalize_optional_hex_color(row.get(29)?),
                        line_number_background_color: normalize_optional_hex_color(row.get(30)?),
                        shortcuts: ShortcutSettings {
                            open_file: row.get(6)?,
                            focus_search: row.get(7)?,
                            quick_find: row.get(8)?,
                            close_tab: row.get(9)?,
                            open_settings: row.get(10)?,
                            toggle_case_sensitive: row.get(11)?,
                            jump_to_bottom: row.get(12)?,
                            cycle_color_label: row.get(13)?,
                            toggle_word_wrap: row.get(14)?,
                        },
                    })
                },
            )
            .optional()
            .context("无法读取应用设置")
            .map(|settings| settings.unwrap_or_default())
    }

    pub fn save_app_settings(&self, settings: AppSettings) -> Result<()> {
        let connection = self.lock()?;
        connection
            .execute(
                "INSERT INTO app_settings(
                     id, default_show_line_numbers, default_show_row_separators,
                     highlight_log_levels,
                     log_font_size, log_line_spacing, log_font_family,
                     shortcut_open_file, shortcut_focus_search, shortcut_quick_find,
                     shortcut_close_tab, shortcut_open_settings,
                     shortcut_toggle_case_sensitive, shortcut_jump_to_bottom,
                     shortcut_cycle_color_label, shortcut_toggle_word_wrap,
                     mouse_wheel_scroll_percent, scroll_by_line,
                     mouse_wheel_scroll_lines, scroll_by_line_when_word_wrap,
                     reduce_motion, confirm_close_tab, show_full_path,
                     max_search_results, highlight_matches, word_boundary_characters,
                     default_case_sensitive, default_use_regex,
                     show_line_number_row_separators, line_number_width,
                     line_number_text_color, line_number_background_color,
                     theme_preference, open_directory_command, viewer_overscan, language,
                     app_log_level
                 ) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32, ?33, ?34, ?35, ?36)
                 ON CONFLICT(id) DO UPDATE SET
                     default_show_line_numbers = excluded.default_show_line_numbers,
                     default_show_row_separators = excluded.default_show_row_separators,
                     highlight_log_levels = excluded.highlight_log_levels,
                     log_font_size = excluded.log_font_size,
                     log_line_spacing = excluded.log_line_spacing,
                     log_font_family = excluded.log_font_family,
                     shortcut_open_file = excluded.shortcut_open_file,
                     shortcut_focus_search = excluded.shortcut_focus_search,
                     shortcut_quick_find = excluded.shortcut_quick_find,
                     shortcut_close_tab = excluded.shortcut_close_tab,
                     shortcut_open_settings = excluded.shortcut_open_settings,
                     shortcut_toggle_case_sensitive = excluded.shortcut_toggle_case_sensitive,
                     shortcut_jump_to_bottom = excluded.shortcut_jump_to_bottom,
                     shortcut_cycle_color_label = excluded.shortcut_cycle_color_label,
                     shortcut_toggle_word_wrap = excluded.shortcut_toggle_word_wrap,
                     mouse_wheel_scroll_percent = excluded.mouse_wheel_scroll_percent,
                     scroll_by_line = excluded.scroll_by_line,
                     mouse_wheel_scroll_lines = excluded.mouse_wheel_scroll_lines,
                     scroll_by_line_when_word_wrap = excluded.scroll_by_line_when_word_wrap,
                     reduce_motion = excluded.reduce_motion,
                     confirm_close_tab = excluded.confirm_close_tab,
                     show_full_path = excluded.show_full_path,
                     max_search_results = excluded.max_search_results,
                     highlight_matches = excluded.highlight_matches,
                     word_boundary_characters = excluded.word_boundary_characters,
                     default_case_sensitive = excluded.default_case_sensitive,
                     default_use_regex = excluded.default_use_regex,
                     show_line_number_row_separators = excluded.show_line_number_row_separators,
                     line_number_width = excluded.line_number_width,
                     line_number_text_color = excluded.line_number_text_color,
                     line_number_background_color = excluded.line_number_background_color,
                     theme_preference = excluded.theme_preference,
                     open_directory_command = excluded.open_directory_command,
                     viewer_overscan = excluded.viewer_overscan,
                     language = excluded.language,
                     app_log_level = excluded.app_log_level",
                params![
                    settings.default_show_line_numbers,
                    settings.default_show_row_separators,
                    settings.highlight_log_levels,
                    settings.log_font_size,
                    settings.log_line_spacing,
                    settings.log_font_family.database_value(),
                    settings.shortcuts.open_file,
                    settings.shortcuts.focus_search,
                    settings.shortcuts.quick_find,
                    settings.shortcuts.close_tab,
                    settings.shortcuts.open_settings,
                    settings.shortcuts.toggle_case_sensitive,
                    settings.shortcuts.jump_to_bottom,
                    settings.shortcuts.cycle_color_label,
                    settings.shortcuts.toggle_word_wrap,
                    settings.mouse_wheel_scroll_percent,
                    settings.scroll_by_line,
                    settings.mouse_wheel_scroll_lines,
                    settings.scroll_by_line_when_word_wrap,
                    settings.reduce_motion,
                    settings.confirm_close_tab,
                    settings.show_full_path,
                    settings.max_search_results,
                    settings.highlight_matches,
                    settings.word_boundary_characters,
                    settings.default_case_sensitive,
                    settings.default_use_regex,
                    settings.show_line_number_row_separators,
                    settings.line_number_width.clamp(40, 160),
                    normalize_optional_hex_color(settings.line_number_text_color),
                    normalize_optional_hex_color(settings.line_number_background_color),
                    settings.theme_preference.database_value(),
                    settings
                        .open_directory_command
                        .chars()
                        .take(2048)
                        .collect::<String>(),
                    settings.viewer_overscan.clamp(4, 40),
                    settings.language.database_value(),
                    settings.app_log_level.database_value(),
                ],
            )
            .context("无法保存应用设置")?;
        Ok(())
    }

    pub fn load_last_settings_category(&self) -> Result<Option<String>> {
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT value FROM ui_state WHERE key = 'settings.active_category'",
                [],
                |row| row.get(0),
            )
            .optional()
            .context("无法读取上次设置分类")
    }

    pub fn save_last_settings_category(&self, category: &str) -> Result<()> {
        let connection = self.lock()?;
        connection
            .execute(
                "INSERT INTO ui_state(key, value)
                 VALUES ('settings.active_category', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![category],
            )
            .context("无法保存当前设置分类")?;
        Ok(())
    }

    pub fn load_search_panel_height(&self) -> Result<Option<f32>> {
        let connection = self.lock()?;
        let value = connection
            .query_row(
                "SELECT value FROM ui_state WHERE key = 'workspace.search_panel_height'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .context("无法读取搜索面板高度")?;
        Ok(value.and_then(|value| {
            value
                .parse::<f32>()
                .ok()
                .filter(|height| height.is_finite() && *height > 0.)
        }))
    }

    pub fn save_search_panel_height(&self, height: f32) -> Result<()> {
        if !height.is_finite() || height <= 0. {
            anyhow::bail!("搜索面板高度无效：{height}");
        }
        let connection = self.lock()?;
        connection
            .execute(
                "INSERT INTO ui_state(key, value)
                 VALUES ('workspace.search_panel_height', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![height.to_string()],
            )
            .context("无法保存搜索面板高度")?;
        Ok(())
    }

    pub fn load_workspace_search_state(&self) -> Result<WorkspaceSearchState> {
        let connection = self.lock()?;
        let value = connection
            .query_row(
                "SELECT value FROM ui_state WHERE key = 'workspace.search_contexts'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .context("无法读取搜索上下文")?;
        Ok(value
            .as_deref()
            .and_then(|value| serde_json::from_str(value).ok())
            .filter(WorkspaceSearchState::is_compatible)
            .unwrap_or_default())
    }

    pub fn save_workspace_search_state(&self, state: &WorkspaceSearchState) -> Result<()> {
        let value = serde_json::to_string(state).context("无法序列化搜索上下文")?;
        let connection = self.lock()?;
        connection
            .execute(
                "INSERT INTO ui_state(key, value)
                 VALUES ('workspace.search_contexts', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![value],
            )
            .context("无法保存搜索上下文")?;
        Ok(())
    }

    pub fn global_search_preferences(&self) -> Result<BTreeMap<PathBuf, bool>> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                "SELECT path, selected
                 FROM global_search_preferences
                 ORDER BY path",
            )
            .context("无法读取全局搜索参与偏好")?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    path_from_database(row.get::<_, String>(0)?),
                    row.get::<_, i64>(1)? != 0,
                ))
            })
            .context("无法查询全局搜索参与偏好")?;
        rows.collect::<rusqlite::Result<BTreeMap<_, _>>>()
            .context("无法解析全局搜索参与偏好")
    }

    pub fn save_global_search_preferences(&self, preferences: &[(PathBuf, bool)]) -> Result<()> {
        if preferences.is_empty() {
            return Ok(());
        }
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction()
            .context("无法开始全局搜索参与偏好事务")?;
        for (path, selected) in preferences {
            transaction
                .execute(
                    "INSERT INTO global_search_preferences(path, selected)
                     VALUES (?1, ?2)
                     ON CONFLICT(path) DO UPDATE SET selected = excluded.selected",
                    params![path_to_database(path), selected],
                )
                .with_context(|| format!("无法保存全局搜索参与偏好：{}", path.display()))?;
        }
        transaction.commit().context("无法提交全局搜索参与偏好事务")
    }

    pub fn load_search_history(&self) -> Result<Vec<String>> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                "SELECT query_text
                 FROM search_history
                 ORDER BY position ASC",
            )
            .context("无法读取搜索历史")?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .context("无法查询搜索历史")?;
        let history = rows
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("无法解析搜索历史")?;
        Ok(normalize_search_history(history))
    }

    pub fn save_search_history(&self, history: &[String]) -> Result<()> {
        let history = normalize_search_history(history.iter().cloned());
        let mut connection = self.lock()?;
        let transaction = connection.transaction().context("无法开始搜索历史事务")?;
        transaction
            .execute("DELETE FROM search_history", [])
            .context("无法重置搜索历史")?;
        for (position, query) in history.iter().enumerate() {
            transaction
                .execute(
                    "INSERT INTO search_history(position, query_text) VALUES (?1, ?2)",
                    params![i64::try_from(position).unwrap_or(i64::MAX), query],
                )
                .context("无法写入搜索历史")?;
        }
        transaction.commit().context("无法提交搜索历史事务")
    }

    pub fn load_predefined_filters(&self) -> Result<Vec<PredefinedFilter>> {
        let (filters, needs_migration) = {
            let connection = self.lock()?;
            let mut statement = connection
                .prepare(
                    "SELECT filter_id, item_json
                     FROM predefined_filters
                     ORDER BY position ASC",
                )
                .context("无法读取预定义过滤器")?;
            let rows = statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .context("无法查询预定义过滤器")?;
            let mut filters = Vec::new();
            let mut needs_migration = false;
            for row in rows {
                let (stored_id, json) = row.context("无法解析预定义过滤器记录")?;
                let value = serde_json::from_str::<serde_json::Value>(&json)
                    .context("预定义过滤器记录格式无效")?;
                let filter = parse_stored_filter(&json).context("预定义过滤器记录格式无效")?;
                needs_migration |= value.get("id").is_some()
                    || value.get("source").is_some()
                    || value.get("published").is_some()
                    || value
                        .get("uuid")
                        .and_then(serde_json::Value::as_str)
                        .and_then(crate::predefined_filters::FilterBranchId::parse)
                        .is_none()
                    || stored_id != filter.id.to_string();
                filters.push(filter);
            }
            (normalize_predefined_filters(filters), needs_migration)
        };
        if needs_migration {
            self.save_predefined_filters(&filters)
                .context("无法将预定义过滤器迁移到 v5")?;
        }
        Ok(filters)
    }

    pub fn save_predefined_filters(&self, filters: &[PredefinedFilter]) -> Result<()> {
        let filters = normalize_predefined_filters(filters.to_vec());
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction()
            .context("无法开始预定义过滤器事务")?;
        transaction
            .execute("DELETE FROM predefined_filters", [])
            .context("无法重置预定义过滤器")?;
        for (position, filter) in filters.iter().enumerate() {
            transaction
                .execute(
                    "INSERT INTO predefined_filters(position, filter_id, item_json)
                     VALUES (?1, ?2, ?3)",
                    params![
                        i64::try_from(position).unwrap_or(i64::MAX),
                        filter.id.to_string(),
                        serde_json::to_string(filter).context("无法编码预定义过滤器")?,
                    ],
                )
                .with_context(|| format!("无法保存预定义过滤器：{}", filter.name))?;
        }
        transaction.commit().context("无法提交预定义过滤器事务")
    }

    pub fn load_cloud_settings(&self) -> Result<CloudSettings> {
        let connection = self.lock()?;
        connection
            .query_row(
                "SELECT server_url, display_name FROM cloud_settings WHERE id = 1",
                [],
                |row| {
                    Ok(CloudSettings {
                        server_url: row.get(0)?,
                        display_name: row.get(1)?,
                    })
                },
            )
            .optional()
            .context("无法读取云端过滤器设置")
            .map(|settings| settings.unwrap_or_default())
    }

    pub fn save_cloud_settings(&self, settings: &CloudSettings) -> Result<()> {
        let connection = self.lock()?;
        connection
            .execute(
                "INSERT INTO cloud_settings(id, server_url, display_name)
                 VALUES (1, ?1, ?2)
                 ON CONFLICT(id) DO UPDATE SET
                     server_url = excluded.server_url,
                     display_name = excluded.display_name",
                params![settings.server_url.trim(), settings.display_name.trim()],
            )
            .context("无法保存云端过滤器设置")?;
        Ok(())
    }

    pub fn load_color_labels(&self) -> Result<Vec<ColorLabel>> {
        let connection = self.lock()?;
        let initialized = connection
            .query_row(
                "SELECT initialized FROM color_label_settings WHERE id = 1",
                [],
                |row| row.get::<_, bool>(0),
            )
            .optional()
            .context("无法读取颜色标签初始化状态")?
            .unwrap_or(false);
        if !initialized {
            return Ok(default_color_labels());
        }
        let mut statement = connection
            .prepare(
                "SELECT label_id, name, color, alpha
                 FROM color_labels
                 ORDER BY position ASC",
            )
            .context("无法读取颜色标签")?;
        let rows = statement
            .query_map([], |row| {
                Ok(ColorLabel {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    color: row.get(2)?,
                    alpha: u8::try_from(row.get::<_, i64>(3)?).unwrap_or(u8::MAX),
                })
            })
            .context("无法查询颜色标签")?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("无法解析颜色标签")
    }

    pub fn save_color_labels(&self, labels: &[ColorLabel]) -> Result<()> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction().context("无法开始颜色标签事务")?;
        transaction
            .execute("DELETE FROM color_labels", [])
            .context("无法重置颜色标签")?;
        for (position, label) in labels.iter().enumerate() {
            transaction
                .execute(
                    "INSERT INTO color_labels(position, label_id, name, color, alpha)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        i64::try_from(position).unwrap_or(i64::MAX),
                        label.id,
                        label.name,
                        label.color,
                        label.alpha,
                    ],
                )
                .with_context(|| format!("无法保存颜色标签：{}", label.name))?;
        }
        transaction
            .execute(
                "INSERT INTO color_label_settings(id, initialized) VALUES (1, 1)
                 ON CONFLICT(id) DO UPDATE SET initialized = 1",
                [],
            )
            .context("无法保存颜色标签初始化状态")?;
        transaction.commit().context("无法提交颜色标签事务")
    }

    pub fn recent_files(&self, limit: usize) -> Result<Vec<RecentFile>> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                "SELECT id, path, last_opened_at
                 FROM file_sessions
                 ORDER BY pinned DESC, last_opened_at DESC, id DESC
                 LIMIT ?1",
            )
            .context("无法读取最近文件查询")?;
        let rows = statement
            .query_map([i64::try_from(limit).unwrap_or(i64::MAX)], |row| {
                Ok(RecentFile {
                    id: row.get(0)?,
                    path: path_from_database(row.get::<_, String>(1)?),
                    last_opened_at: row.get(2)?,
                })
            })
            .context("无法查询最近文件")?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("无法解析最近文件")
    }

    pub fn pinned_files(&self) -> Result<Vec<RecentFile>> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                "SELECT id, path, last_opened_at
                 FROM file_sessions
                 WHERE pinned = 1
                 ORDER BY last_opened_at DESC, id DESC",
            )
            .context("无法读取收藏文件查询")?;
        let rows = statement
            .query_map([], |row| {
                Ok(RecentFile {
                    id: row.get(0)?,
                    path: path_from_database(row.get::<_, String>(1)?),
                    last_opened_at: row.get(2)?,
                })
            })
            .context("无法查询收藏文件")?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("无法解析收藏文件")
    }

    pub fn session_history(&self) -> Result<Vec<HistorySession>> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                "SELECT id, path, last_opened_at, revision, selected_row,
                        query_text, marked_rows, pinned
                 FROM file_sessions
                 ORDER BY pinned DESC, last_opened_at DESC, id DESC",
            )
            .context("无法读取文件会话历史查询")?;
        let rows = statement
            .query_map([], |row| {
                let selected_row = row
                    .get::<_, Option<i64>>(4)?
                    .and_then(|value| usize::try_from(value).ok());
                let marked_rows = row.get::<_, String>(6)?;
                Ok(HistorySession {
                    id: row.get(0)?,
                    path: path_from_database(row.get::<_, String>(1)?),
                    last_opened_at: row.get(2)?,
                    revision: row.get(3)?,
                    selected_row,
                    query_text: row.get(5)?,
                    marked_rows_count: count_marked_rows(&marked_rows),
                    pinned: row.get::<_, i64>(7)? != 0,
                })
            })
            .context("无法查询文件会话历史")?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("无法解析文件会话历史")
    }

    pub fn set_pinned(&self, path: &Path, pinned: bool) -> Result<()> {
        let connection = self.lock()?;
        connection
            .execute(
                "INSERT INTO file_sessions(path, last_opened_at, pinned)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(path) DO UPDATE SET
                     pinned = excluded.pinned,
                     revision = file_sessions.revision + 1",
                params![path_to_database(path), unix_timestamp(), pinned],
            )
            .with_context(|| format!("无法更新文件收藏状态：{}", path.display()))?;
        Ok(())
    }

    pub fn clear_pinned(&self) -> Result<()> {
        let connection = self.lock()?;
        connection
            .execute(
                "UPDATE file_sessions
                 SET pinned = 0, revision = revision + 1
                 WHERE pinned = 1",
                [],
            )
            .context("无法清空收藏文件")?;
        Ok(())
    }

    pub fn clear_history(&self, open_paths: &[PathBuf]) -> Result<usize> {
        let protected_paths = open_paths
            .iter()
            .map(|path| path_to_database(path))
            .collect::<HashSet<_>>();
        let mut connection = self.lock()?;
        let transaction = connection.transaction().context("无法开始历史清理事务")?;
        let removable_ids = {
            let mut statement = transaction
                .prepare(
                    "SELECT id, path
                     FROM file_sessions
                     WHERE pinned = 0 AND marked_rows = ''",
                )
                .context("无法准备历史清理查询")?;
            let rows = statement
                .query_map([], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                })
                .context("无法查询可清理历史")?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .context("无法解析可清理历史")?
                .into_iter()
                .filter_map(|(id, path)| (!protected_paths.contains(&path)).then_some(id))
                .collect::<Vec<_>>()
        };
        for id in &removable_ids {
            transaction
                .execute("DELETE FROM file_sessions WHERE id = ?1", [id])
                .context("无法删除历史记录")?;
        }
        transaction.commit().context("无法提交历史清理事务")?;
        Ok(removable_ids.len())
    }

    pub fn delete_history_session(&self, id: i64, open_paths: &[PathBuf]) -> Result<bool> {
        let protected_paths = open_paths
            .iter()
            .map(|path| path_to_database(path))
            .collect::<HashSet<_>>();
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction()
            .context("无法开始历史记录删除事务")?;
        let session = {
            let mut statement = transaction
                .prepare(
                    "SELECT path, pinned, marked_rows
                     FROM file_sessions
                     WHERE id = ?1",
                )
                .context("无法准备历史记录删除查询")?;
            let mut rows = statement.query([id]).context("无法读取待删除历史记录")?;
            rows.next()
                .context("无法解析待删除历史记录")?
                .map(|row| {
                    Ok::<_, rusqlite::Error>((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)? != 0,
                        row.get::<_, String>(2)?,
                    ))
                })
                .transpose()
                .context("无法解析历史记录保护状态")?
        };
        let Some((path, pinned, marked_rows)) = session else {
            return Ok(false);
        };
        if pinned || !marked_rows.is_empty() || protected_paths.contains(&path) {
            return Ok(false);
        }
        let removed = transaction
            .execute("DELETE FROM file_sessions WHERE id = ?1", [id])
            .context("无法删除文件会话历史")?;
        transaction.commit().context("无法提交历史记录删除事务")?;
        Ok(removed > 0)
    }

    pub fn load_session(&self, path: &Path) -> Result<Option<FileSessionState>> {
        let connection = self.lock()?;
        load_session_row(&connection, path)
    }

    pub fn load_sessions(&self, paths: &[PathBuf]) -> Result<BTreeMap<PathBuf, FileSessionState>> {
        const BATCH_SIZE: usize = 500;

        if paths.is_empty() {
            return Ok(BTreeMap::new());
        }
        let database_paths = paths
            .iter()
            .map(|path| path_to_database(path))
            .collect::<BTreeSet<_>>();
        let database_paths = database_paths.into_iter().collect::<Vec<_>>();
        let connection = self.lock()?;
        let mut sessions = BTreeMap::new();
        for batch in database_paths.chunks(BATCH_SIZE) {
            let placeholders = std::iter::repeat_n("?", batch.len())
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!(
                "SELECT path, revision, custom_title, selected_row, query_text,
                        case_sensitive, regex, result_mode, marked_rows,
                        show_line_numbers, show_row_separators, keyword_color_rules, word_wrap,
                        resume_state
                 FROM file_sessions
                 WHERE path IN ({placeholders})"
            );
            let mut statement = connection
                .prepare(&sql)
                .context("无法准备批量文件会话查询")?;
            let rows = statement
                .query_map(params_from_iter(batch), |row| {
                    Ok((
                        path_from_database(row.get::<_, String>(0)?),
                        file_session_from_row(row, 1)?,
                    ))
                })
                .context("无法批量查询文件会话")?;
            for row in rows {
                let (path, session) = row.context("无法解析批量文件会话")?;
                sessions.insert(path, session);
            }
        }
        Ok(sessions)
    }

    pub fn last_workspace(&self) -> Result<Vec<LastWorkspaceFile>> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare(
                "SELECT file_sessions.id, file_sessions.path,
                        file_sessions.last_opened_at,
                        last_workspace_files.was_active
                 FROM last_workspace_files
                 JOIN file_sessions
                   ON file_sessions.id = last_workspace_files.file_session_id
                 ORDER BY last_workspace_files.position ASC",
            )
            .context("无法准备上一次工作区查询")?;
        let rows = statement
            .query_map([], |row| {
                Ok(LastWorkspaceFile {
                    id: row.get(0)?,
                    path: path_from_database(row.get::<_, String>(1)?),
                    last_opened_at: row.get(2)?,
                    was_active: row.get::<_, i64>(3)? != 0,
                })
            })
            .context("无法查询上一次工作区")?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("无法解析上一次工作区")
    }

    pub fn save_session(
        &self,
        path: &Path,
        base: &FileSessionState,
        state: &FileSessionState,
    ) -> Result<SessionSaveResult> {
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .context("无法开始文件会话保存事务")?;
        let current = load_session_row(&transaction, path)?;
        let current_revision = current.as_ref().map_or(0, |current| current.revision);
        let conflict_resolved = current
            .as_ref()
            .is_some_and(|current| current.revision != base.revision);
        let mut candidate = match current {
            Some(current) if conflict_resolved => merge_session_changes(base, state, current),
            Some(_) | None => state.clone(),
        };
        candidate.revision = current_revision.saturating_add(1);
        save_session_row(&transaction, path, &candidate)?;
        transaction.commit().context("无法提交文件会话保存事务")?;
        Ok(SessionSaveResult {
            state: candidate,
            conflict_resolved,
        })
    }

    pub fn database_info(&self) -> Result<DatabaseInfo> {
        let connection = self.lock()?;
        _ = connection.execute_batch("PRAGMA wal_checkpoint(PASSIVE);");
        let session_count = connection
            .query_row("SELECT COUNT(*) FROM file_sessions", [], |row| {
                row.get::<_, i64>(0)
            })
            .context("无法统计文件会话数量")?;
        drop(connection);
        let sidecar = |suffix: &str| {
            let mut path = self.database_path.as_os_str().to_os_string();
            path.push(suffix);
            PathBuf::from(path)
        };
        let byte_size = [self.database_path.clone(), sidecar("-wal"), sidecar("-shm")]
            .into_iter()
            .filter_map(|path| fs::metadata(path).ok().map(|metadata| metadata.len()))
            .fold(0_u64, u64::saturating_add);
        Ok(DatabaseInfo {
            byte_size,
            session_count: usize::try_from(session_count).unwrap_or(usize::MAX),
        })
    }

    pub fn delete_session_for_path(&self, path: &Path) -> Result<bool> {
        let connection = self.lock()?;
        connection
            .execute(
                "DELETE FROM file_sessions WHERE path = ?1",
                [path_to_database(path)],
            )
            .with_context(|| format!("无法删除文件会话：{}", path.display()))
            .map(|removed| removed > 0)
    }

    pub fn save_sessions(&self, sessions: &[(PathBuf, FileSessionState)]) -> Result<()> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction().context("无法开始窗口会话事务")?;
        for (path, state) in sessions {
            save_session_row(&transaction, path, state)?;
        }
        transaction.commit().context("无法提交窗口会话事务")
    }

    pub fn save_workspace(
        &self,
        sessions: &[(PathBuf, FileSessionState)],
        open_paths: &[PathBuf],
        active_path: Option<&Path>,
    ) -> Result<()> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction().context("无法开始退出状态事务")?;
        for (path, state) in sessions {
            save_session_row(&transaction, path, state)?;
        }
        transaction
            .execute("DELETE FROM last_workspace_files", [])
            .context("无法重置上一次工作区")?;
        for (position, path) in open_paths.iter().enumerate() {
            let session_id = transaction
                .query_row(
                    "SELECT id FROM file_sessions WHERE path = ?1",
                    [path_to_database(path)],
                    |row| row.get::<_, i64>(0),
                )
                .with_context(|| format!("无法定位工作区文件会话：{}", path.display()))?;
            transaction
                .execute(
                    "INSERT INTO last_workspace_files(position, file_session_id, was_active)
                     VALUES (?1, ?2, ?3)",
                    params![
                        i64::try_from(position).unwrap_or(i64::MAX),
                        session_id,
                        active_path.is_some_and(|active| active == path)
                    ],
                )
                .with_context(|| format!("无法保存工作区文件：{}", path.display()))?;
        }
        transaction.commit().context("无法提交退出状态事务")
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>> {
        self.connection
            .lock()
            .map_err(|_| anyhow::anyhow!("状态库锁已损坏"))
    }
}

fn load_session_row(connection: &Connection, path: &Path) -> Result<Option<FileSessionState>> {
    connection
        .query_row(
            "SELECT revision, custom_title, selected_row, query_text, case_sensitive, regex,
                    result_mode, marked_rows, show_line_numbers, show_row_separators,
                    keyword_color_rules, word_wrap, resume_state
             FROM file_sessions
             WHERE path = ?1",
            [path_to_database(path)],
            |row| file_session_from_row(row, 0),
        )
        .optional()
        .with_context(|| format!("无法读取文件会话：{}", path.display()))
}

fn file_session_from_row(
    row: &rusqlite::Row<'_>,
    offset: usize,
) -> rusqlite::Result<FileSessionState> {
    let selected_row = row
        .get::<_, Option<i64>>(offset + 2)?
        .and_then(|row| usize::try_from(row).ok());
    let marked_rows = row
        .get::<_, String>(offset + 7)?
        .split(',')
        .filter_map(|value| value.parse::<usize>().ok())
        .collect::<Vec<_>>();
    let query_text = row.get::<_, String>(offset + 3)?;
    let result_mode = row.get::<_, i64>(offset + 6)?;
    let resume_state = row.get::<_, String>(offset + 12)?;
    let legacy_results_visible =
        !query_text.is_empty() || (result_mode != 1 && !marked_rows.is_empty());
    let resume = serde_json::from_str(&resume_state)
        .ok()
        .filter(TabResumeState::is_compatible)
        .unwrap_or_else(|| TabResumeState::from_legacy(selected_row, legacy_results_visible));
    Ok(FileSessionState {
        revision: row.get(offset)?,
        custom_title: row.get(offset + 1)?,
        selected_row,
        query_text,
        case_sensitive: row.get::<_, i64>(offset + 4)? != 0,
        regex: row.get::<_, i64>(offset + 5)? != 0,
        result_mode,
        marked_rows,
        show_line_numbers: row.get::<_, i64>(offset + 8)? != 0,
        show_row_separators: row.get::<_, i64>(offset + 9)? != 0,
        keyword_color_rules: decode_rules(&row.get::<_, String>(offset + 10)?),
        word_wrap: row.get::<_, i64>(offset + 11)? != 0,
        resume,
    })
}

fn merge_session_changes(
    base: &FileSessionState,
    desired: &FileSessionState,
    mut latest: FileSessionState,
) -> FileSessionState {
    if base.custom_title != desired.custom_title {
        latest.custom_title.clone_from(&desired.custom_title);
    }
    if base.selected_row != desired.selected_row {
        latest.selected_row = desired.selected_row;
    }
    if base.query_text != desired.query_text {
        latest.query_text.clone_from(&desired.query_text);
    }
    if base.case_sensitive != desired.case_sensitive {
        latest.case_sensitive = desired.case_sensitive;
    }
    if base.regex != desired.regex {
        latest.regex = desired.regex;
    }
    if base.result_mode != desired.result_mode {
        latest.result_mode = desired.result_mode;
    }
    if base.marked_rows != desired.marked_rows {
        latest.marked_rows.clone_from(&desired.marked_rows);
    }
    if base.show_line_numbers != desired.show_line_numbers {
        latest.show_line_numbers = desired.show_line_numbers;
    }
    if base.show_row_separators != desired.show_row_separators {
        latest.show_row_separators = desired.show_row_separators;
    }
    if base.word_wrap != desired.word_wrap {
        latest.word_wrap = desired.word_wrap;
    }
    if base.keyword_color_rules != desired.keyword_color_rules {
        latest
            .keyword_color_rules
            .clone_from(&desired.keyword_color_rules);
    }
    if base.resume != desired.resume {
        latest.resume.clone_from(&desired.resume);
    }
    latest
}

fn save_session_row(connection: &Connection, path: &Path, state: &FileSessionState) -> Result<()> {
    let selected_row = state.selected_row.and_then(|row| i64::try_from(row).ok());
    let marked_rows = state
        .marked_rows
        .iter()
        .map(usize::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let keyword_color_rules = encode_rules(&state.keyword_color_rules);
    let resume_state = serde_json::to_string(&state.resume).context("无法序列化标签恢复状态")?;
    connection
        .execute(
            "INSERT INTO file_sessions(
                     path, custom_title, last_opened_at, selected_row, query_text,
                     case_sensitive, regex, result_mode, marked_rows,
                     show_line_numbers, show_row_separators, keyword_color_rules, word_wrap,
                     resume_state
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
                 ON CONFLICT(path) DO UPDATE SET
                     custom_title = excluded.custom_title,
                     selected_row = excluded.selected_row,
                     query_text = excluded.query_text,
                     case_sensitive = excluded.case_sensitive,
                     regex = excluded.regex,
                     result_mode = excluded.result_mode,
                     marked_rows = excluded.marked_rows,
                     show_line_numbers = excluded.show_line_numbers,
                     show_row_separators = excluded.show_row_separators,
                     keyword_color_rules = excluded.keyword_color_rules,
                     word_wrap = excluded.word_wrap,
                     resume_state = excluded.resume_state,
                     revision = file_sessions.revision + 1",
            params![
                path_to_database(path),
                state.custom_title,
                unix_timestamp(),
                selected_row,
                state.query_text,
                state.case_sensitive,
                state.regex,
                state.result_mode,
                marked_rows,
                state.show_line_numbers,
                state.show_row_separators,
                keyword_color_rules,
                state.word_wrap,
                resume_state,
            ],
        )
        .with_context(|| format!("无法保存文件会话：{}", path.display()))?;
    Ok(())
}

fn ensure_session_columns(connection: &Connection) -> Result<()> {
    const COLUMNS: [(&str, &str); 13] = [
        ("custom_title", "TEXT"),
        ("selected_row", "INTEGER"),
        ("query_text", "TEXT NOT NULL DEFAULT ''"),
        ("case_sensitive", "INTEGER NOT NULL DEFAULT 0"),
        ("regex", "INTEGER NOT NULL DEFAULT 0"),
        ("result_mode", "INTEGER NOT NULL DEFAULT 0"),
        ("marked_rows", "TEXT NOT NULL DEFAULT ''"),
        ("show_line_numbers", "INTEGER NOT NULL DEFAULT 1"),
        ("show_row_separators", "INTEGER NOT NULL DEFAULT 0"),
        ("keyword_color_rules", "TEXT NOT NULL DEFAULT '[]'"),
        ("word_wrap", "INTEGER NOT NULL DEFAULT 0"),
        ("resume_state", "TEXT NOT NULL DEFAULT ''"),
        ("pinned", "INTEGER NOT NULL DEFAULT 0"),
    ];
    let existing = table_columns(connection, "file_sessions")?;
    for (name, declaration) in COLUMNS {
        if !existing.contains(name) {
            connection
                .execute(
                    &format!("ALTER TABLE file_sessions ADD COLUMN {name} {declaration}"),
                    [],
                )
                .with_context(|| format!("无法迁移状态库字段：{name}"))?;
        }
    }
    Ok(())
}

fn ensure_color_label_columns(connection: &Connection) -> Result<()> {
    if table_columns(connection, "color_labels")?.contains("alpha") {
        return Ok(());
    }

    connection
        .execute(
            "ALTER TABLE color_labels ADD COLUMN alpha INTEGER NOT NULL DEFAULT 255",
            [],
        )
        .context("无法迁移颜色标签透明度字段")?;
    for label in default_color_labels() {
        connection
            .execute(
                "UPDATE color_labels
                 SET alpha = ?1
                 WHERE label_id = ?2 AND color = ?3",
                params![label.alpha, label.id, label.color],
            )
            .with_context(|| format!("无法迁移颜色标签透明度：{}", label.name))?;
    }
    Ok(())
}

fn ensure_app_settings_columns(connection: &Connection) -> Result<()> {
    const COLUMNS: [(&str, &str); 33] = [
        ("highlight_log_levels", "INTEGER NOT NULL DEFAULT 0"),
        ("log_font_size", "INTEGER NOT NULL DEFAULT 13"),
        ("log_line_spacing", "INTEGER NOT NULL DEFAULT 6"),
        ("log_font_family", "TEXT NOT NULL DEFAULT 'consolas'"),
        ("shortcut_open_file", "TEXT NOT NULL DEFAULT 'Ctrl+O'"),
        ("shortcut_focus_search", "TEXT NOT NULL DEFAULT 'Ctrl+F'"),
        (
            "shortcut_quick_find",
            "TEXT NOT NULL DEFAULT 'Ctrl+Shift+F'",
        ),
        ("shortcut_close_tab", "TEXT NOT NULL DEFAULT 'Ctrl+W'"),
        ("shortcut_open_settings", "TEXT NOT NULL DEFAULT 'Ctrl+,'"),
        (
            "shortcut_toggle_case_sensitive",
            "TEXT NOT NULL DEFAULT 'Alt+C'",
        ),
        (
            "shortcut_jump_to_bottom",
            "TEXT NOT NULL DEFAULT 'Ctrl+End'",
        ),
        (
            "shortcut_cycle_color_label",
            "TEXT NOT NULL DEFAULT 'Ctrl+D'",
        ),
        ("shortcut_toggle_word_wrap", "TEXT NOT NULL DEFAULT 'W'"),
        ("mouse_wheel_scroll_percent", "INTEGER NOT NULL DEFAULT 100"),
        ("scroll_by_line", "INTEGER NOT NULL DEFAULT 0"),
        ("mouse_wheel_scroll_lines", "INTEGER NOT NULL DEFAULT 1"),
        (
            "scroll_by_line_when_word_wrap",
            "INTEGER NOT NULL DEFAULT 0",
        ),
        ("reduce_motion", "INTEGER NOT NULL DEFAULT 0"),
        ("confirm_close_tab", "INTEGER NOT NULL DEFAULT 0"),
        ("show_full_path", "INTEGER NOT NULL DEFAULT 1"),
        ("max_search_results", "INTEGER NOT NULL DEFAULT 0"),
        ("highlight_matches", "INTEGER NOT NULL DEFAULT 1"),
        (
            "word_boundary_characters",
            r#"TEXT NOT NULL DEFAULT '.,;:!?()[]{}<>/\|"''`~@#$%^&*+-=，。！？；：、（）【】《》“”‘’…—'"#,
        ),
        ("default_case_sensitive", "INTEGER NOT NULL DEFAULT 0"),
        ("default_use_regex", "INTEGER NOT NULL DEFAULT 0"),
        (
            "show_line_number_row_separators",
            "INTEGER NOT NULL DEFAULT 0",
        ),
        ("line_number_width", "INTEGER NOT NULL DEFAULT 60"),
        ("line_number_text_color", "TEXT"),
        ("line_number_background_color", "TEXT"),
        ("theme_preference", "TEXT NOT NULL DEFAULT 'light'"),
        ("open_directory_command", "TEXT NOT NULL DEFAULT ''"),
        ("viewer_overscan", "INTEGER NOT NULL DEFAULT 12"),
        ("language", "TEXT NOT NULL DEFAULT 'zh-CN'"),
    ];
    let existing = table_columns(connection, "app_settings")?;
    for (name, declaration) in COLUMNS {
        if !existing.contains(name) {
            connection
                .execute(
                    &format!("ALTER TABLE app_settings ADD COLUMN {name} {declaration}"),
                    [],
                )
                .with_context(|| format!("无法迁移应用设置字段：{name}"))?;
        }
    }
    if !existing.contains("app_log_level") {
        connection
            .execute(
                "ALTER TABLE app_settings ADD COLUMN app_log_level TEXT NOT NULL DEFAULT 'error'",
                [],
            )
            .context("无法迁移应用日志等级字段")?;
        connection
            .execute(
                "UPDATE app_settings SET app_log_level = ?1",
                [AppLogLevel::default().database_value()],
            )
            .context("无法初始化应用日志等级")?;
    }
    Ok(())
}

fn table_columns(connection: &Connection, table: &str) -> Result<HashSet<String>> {
    let mut statement = connection
        .prepare("SELECT name FROM pragma_table_info(?1)")
        .with_context(|| format!("无法准备状态库字段检查：{table}"))?;
    let columns = statement
        .query_map([table], |row| row.get::<_, String>(0))
        .with_context(|| format!("无法查询状态库字段：{table}"))?;
    columns
        .collect::<rusqlite::Result<HashSet<_>>>()
        .with_context(|| format!("无法解析状态库字段：{table}"))
}

fn normalize_optional_hex_color(value: Option<String>) -> Option<String> {
    let value = value?.trim().to_ascii_lowercase();
    (value.len() == 7
        && value.starts_with('#')
        && value[1..].bytes().all(|byte| byte.is_ascii_hexdigit()))
    .then_some(value)
}

fn path_to_database(path: &Path) -> String {
    if let Some(path) = path.to_str()
        && !path.starts_with(ENCODED_PATH_PREFIX)
    {
        return path.to_owned();
    }

    #[cfg(unix)]
    let (platform, bytes) = ("u:", path.as_os_str().as_bytes().to_vec());
    #[cfg(windows)]
    let (platform, bytes) = (
        "w:",
        path.as_os_str()
            .encode_wide()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>(),
    );
    #[cfg(not(any(unix, windows)))]
    let (platform, bytes) = ("o:", path.to_string_lossy().as_bytes().to_vec());

    let mut encoded = String::with_capacity(ENCODED_PATH_PREFIX.len() + 2 + bytes.len() * 2);
    encoded.push_str(ENCODED_PATH_PREFIX);
    encoded.push_str(platform);
    for byte in bytes {
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}

fn path_from_database(stored: String) -> PathBuf {
    #[cfg(unix)]
    if let Some(encoded) = stored
        .strip_prefix(ENCODED_PATH_PREFIX)
        .and_then(|encoded| encoded.strip_prefix("u:"))
        && let Some(bytes) = decode_hex(encoded)
    {
        return PathBuf::from(std::ffi::OsString::from_vec(bytes));
    }
    #[cfg(windows)]
    if let Some(encoded) = stored
        .strip_prefix(ENCODED_PATH_PREFIX)
        .and_then(|encoded| encoded.strip_prefix("w:"))
        && let Some(bytes) = decode_hex(encoded)
    {
        let (units, remainder) = bytes.as_slice().as_chunks::<2>();
        if remainder.is_empty() {
            let wide = units
                .iter()
                .map(|bytes| u16::from_le_bytes(*bytes))
                .collect::<Vec<_>>();
            return PathBuf::from(std::ffi::OsString::from_wide(&wide));
        }
    }
    PathBuf::from(stored)
}

fn decode_hex(encoded: &str) -> Option<Vec<u8>> {
    if !encoded.len().is_multiple_of(2) {
        return None;
    }
    let (pairs, remainder) = encoded.as_bytes().as_chunks::<2>();
    debug_assert!(remainder.is_empty());
    pairs
        .iter()
        .map(|digits| {
            let high = (digits[0] as char).to_digit(16)?;
            let low = (digits[1] as char).to_digit(16)?;
            Some(((high << 4) | low) as u8)
        })
        .collect()
}

fn count_marked_rows(marked_rows: &str) -> usize {
    if marked_rows.is_empty() {
        0
    } else {
        marked_rows.split(',').count()
    }
}

fn unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or_default()
}

#[cfg(test)]
mod session_load_tests {
    use std::{fs, hint::black_box, path::PathBuf, time::Instant};

    #[cfg(unix)]
    use std::{ffi::OsString, os::unix::ffi::OsStringExt as _};

    use super::{FileSessionState, StateStore, path_from_database, path_to_database};

    struct TemporaryDatabase(PathBuf);

    impl TemporaryDatabase {
        fn new(label: &str) -> Self {
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .expect("系统时间应晚于 Unix 纪元")
                .as_nanos();
            let directory = std::env::temp_dir()
                .join(format!("vclogg2-{label}-{}-{nonce}", std::process::id()));
            fs::create_dir_all(&directory).expect("应能创建临时状态库目录");
            Self(directory.join("state.db"))
        }
    }

    impl Drop for TemporaryDatabase {
        fn drop(&mut self) {
            if let Some(directory) = self.0.parent() {
                _ = fs::remove_dir_all(directory);
            }
        }
    }

    fn session(query: &str) -> FileSessionState {
        FileSessionState {
            query_text: query.into(),
            selected_row: Some(42),
            marked_rows: vec![3, 9],
            ..FileSessionState::default()
        }
    }

    #[test]
    fn bulk_load_returns_only_existing_sessions() {
        let database = TemporaryDatabase::new("bulk-session-correctness");
        let store = StateStore::open(database.0.clone()).expect("应能打开测试状态库");
        let first = database.0.with_file_name("first.log");
        let second = database.0.with_file_name("second.log");
        let missing = database.0.with_file_name("missing.log");
        store
            .save_sessions(&[
                (first.clone(), session("first")),
                (second.clone(), session("second")),
            ])
            .expect("应能保存测试会话");

        let loaded = store
            .load_sessions(&[
                first.clone(),
                missing.clone(),
                second.clone(),
                first.clone(),
            ])
            .expect("应能批量读取测试会话");

        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[&first].query_text, "first");
        assert_eq!(loaded[&second].query_text, "second");
        assert!(!loaded.contains_key(&missing));
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_paths_keep_distinct_persisted_sessions_and_preferences() {
        let database = TemporaryDatabase::new("non-utf8-path-correctness");
        let directory = database.0.parent().expect("测试数据库应有父目录");
        let first = directory.join(OsString::from_vec(b"source-\x80.log".to_vec()));
        let second = directory.join(OsString::from_vec(b"source-\x81.log".to_vec()));
        assert_eq!(first.to_string_lossy(), second.to_string_lossy());
        assert_ne!(path_to_database(&first), path_to_database(&second));
        assert_eq!(
            path_from_database(path_to_database(&first)),
            first,
            "原生路径编码应能无损往返"
        );

        let store = StateStore::open(database.0.clone()).expect("应能打开测试状态库");
        store
            .save_sessions(&[
                (first.clone(), session("first")),
                (second.clone(), session("second")),
            ])
            .expect("应能保存非 UTF-8 路径会话");
        store
            .save_global_search_preferences(&[(first.clone(), true), (second.clone(), false)])
            .expect("应能保存非 UTF-8 路径搜索偏好");
        drop(store);

        let reopened = StateStore::open(database.0.clone()).expect("应能重新打开测试状态库");
        let sessions = reopened
            .load_sessions(&[first.clone(), second.clone()])
            .expect("应能读回非 UTF-8 路径会话");
        assert_eq!(sessions[&first].query_text, "first");
        assert_eq!(sessions[&second].query_text, "second");
        let preferences = reopened
            .global_search_preferences()
            .expect("应能读回非 UTF-8 路径搜索偏好");
        assert_eq!(preferences.get(&first), Some(&true));
        assert_eq!(preferences.get(&second), Some(&false));
    }

    #[test]
    fn initialized_database_records_current_schema_version() {
        let database = TemporaryDatabase::new("schema-version-correctness");
        let store = StateStore::open(database.0.clone()).expect("应能初始化测试状态库");

        let version = store
            .lock()
            .expect("应能锁定测试状态库")
            .query_row("PRAGMA user_version", [], |row| row.get::<_, u32>(0))
            .expect("应能读取测试状态库版本");

        assert_eq!(version, super::STATE_SCHEMA_VERSION);
    }

    #[test]
    fn search_panel_height_round_trips() {
        let database = TemporaryDatabase::new("search-panel-height");
        let store = StateStore::open(database.0.clone()).expect("应能打开测试状态库");

        assert_eq!(
            store
                .load_search_panel_height()
                .expect("应能读取空的搜索面板高度"),
            None
        );
        store
            .save_search_panel_height(312.5)
            .expect("应能保存搜索面板高度");
        drop(store);
        let reopened = StateStore::open(database.0.clone()).expect("应能重新打开测试状态库");
        assert_eq!(
            reopened
                .load_search_panel_height()
                .expect("应能读取重启后的搜索面板高度"),
            Some(312.5)
        );
    }

    #[test]
    fn predefined_filter_rows_migrate_atomically_to_uuid_v5_shape() {
        let database = TemporaryDatabase::new("predefined-filter-v5");
        let store = StateStore::open(database.0.clone()).expect("应能打开测试状态库");
        let uuid = "550e8400-e29b-41d4-a716-446655440000";
        let legacy = format!(
            r#"{{"id":"filter-1","uuid":"{uuid}","name":"Camera","value":"CAMERA","useRegex":false,"note":"","collaborative":true,"published":{{"serverUrl":"https://example.test","filterId":"{uuid}","revision":3,"ownerId":"owner","ownerName":"Owner","note":"","snapshot":{{"name":"Camera","value":"CAMERA","useRegex":false,"note":"","collaborative":true}}}}}}"#
        );
        store
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO predefined_filters(position,filter_id,item_json) VALUES(0,'filter-1',?1)",
                [legacy],
            )
            .unwrap();

        let loaded = store.load_predefined_filters().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id.to_string(), uuid);
        assert_eq!(loaded[0].remote_references.len(), 1);

        let (stored_id, stored_json): (String, String) = store
            .lock()
            .unwrap()
            .query_row(
                "SELECT filter_id,item_json FROM predefined_filters WHERE position=0",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(stored_id, uuid);
        assert!(stored_json.contains("\"remoteReferences\""));
        assert!(!stored_json.contains("\"id\""));
        assert!(!stored_json.contains("\"published\""));
    }

    #[test]
    #[ignore = "手动性能基准：cargo test -p vclogg2 --release benchmark_bulk_session_load -- --ignored --nocapture"]
    fn benchmark_bulk_session_load() {
        const SESSION_COUNT: usize = 500;
        let database = TemporaryDatabase::new("bulk-session-performance");
        let store = StateStore::open(database.0.clone()).expect("应能打开测试状态库");
        let sessions = (0..SESSION_COUNT)
            .map(|index| {
                (
                    database.0.with_file_name(format!("file-{index:04}.log")),
                    session(&format!("query-{index:04}")),
                )
            })
            .collect::<Vec<_>>();
        store
            .save_sessions(&sessions)
            .expect("应能保存性能测试会话");
        let paths = sessions
            .iter()
            .map(|(path, _)| path.clone())
            .collect::<Vec<_>>();

        let individual_started = Instant::now();
        for path in &paths {
            black_box(
                store
                    .load_session(black_box(path))
                    .expect("单条会话读取应成功"),
            );
        }
        let individual = individual_started.elapsed();

        let bulk_started = Instant::now();
        let loaded = store
            .load_sessions(black_box(&paths))
            .expect("批量会话读取应成功");
        let bulk = bulk_started.elapsed();

        assert_eq!(loaded.len(), SESSION_COUNT);
        eprintln!("读取 {SESSION_COUNT} 条会话：逐条 {individual:?}；批量 {bulk:?}");
    }

    #[test]
    #[ignore = "手动性能基准：cargo test -p vclogg2 --release benchmark_reopen_state_store -- --ignored --nocapture"]
    fn benchmark_reopen_state_store() {
        const RUNS: usize = 50;
        let database = TemporaryDatabase::new("state-store-open-performance");
        drop(StateStore::open(database.0.clone()).expect("应能初始化测试状态库"));

        let started = Instant::now();
        for _ in 0..RUNS {
            black_box(StateStore::open(black_box(database.0.clone())).expect("状态库重开应成功"));
        }
        let elapsed = started.elapsed();

        eprintln!(
            "重复打开状态库 {RUNS} 次：{elapsed:?}，平均：{:?}",
            elapsed / RUNS as u32
        );
    }
}
