use std::{
    collections::{BTreeMap, HashSet},
    fmt::Write as _,
    path::{Path, PathBuf},
};

use anyhow::{Context as _, Result};
use vclogg_core::CompressedRows;
use vclogg_data::{
    AppSettingsRecord, ColorLabelRecord, FileSessionRecord, PredefinedFilterRecord,
    StateMigrationDefaults, StateRepository,
};
pub use vclogg_data::{CloudSettings, DatabaseInfo, HistorySession, LastWorkspaceFile, RecentFile};

use crate::app_log::AppLogLevel;
use crate::color_labels::{
    ColorLabel, KeywordColorRule, LogLevelColorRule, decode_rules, default_color_labels,
    default_log_level_rules, encode_rules,
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
pub(crate) const SEARCH_TOOLBAR_HEIGHT_RANGE: std::ops::RangeInclusive<u16> = 24..=48;
pub(crate) const SEARCH_TOOLBAR_FONT_SIZE_RANGE: std::ops::RangeInclusive<u16> = 10..=20;
const COMPRESSED_MARKED_ROWS_PREFIX: &str = "rb1:";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileSessionState {
    pub revision: i64,
    pub custom_title: Option<String>,
    pub selected_row: Option<usize>,
    pub query_text: String,
    pub result_mode: i64,
    pub marked_rows: CompressedRows,
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

    #[test]
    fn log_colors_are_scoped_to_the_resolved_theme() {
        let settings = AppSettings {
            light_log_text_color: Some("#112233".into()),
            light_log_background_color: Some("#f8f7f4".into()),
            dark_log_text_color: Some("#ddeeff".into()),
            dark_log_background_color: Some("#101722".into()),
            ..AppSettings::default()
        };

        assert_eq!(settings.log_text_color(false), Some("#112233"));
        assert_eq!(settings.log_background_color(false), Some("#f8f7f4"));
        assert_eq!(settings.log_text_color(true), Some("#ddeeff"));
        assert_eq!(settings.log_background_color(true), Some("#101722"));
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
    pub light_log_text_color: Option<String>,
    pub light_log_background_color: Option<String>,
    pub dark_log_text_color: Option<String>,
    pub dark_log_background_color: Option<String>,
    pub highlight_log_levels: bool,
    pub log_level_color_rules: Vec<LogLevelColorRule>,
    pub(crate) selection_styles: crate::selection_style::SelectionStyles,
    pub log_font_size: u16,
    pub search_toolbar_height: u16,
    pub search_toolbar_font_size: u16,
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
    /// Application-wide search option. The legacy field name is retained for database mapping.
    pub default_case_sensitive: bool,
    /// Application-wide search option. The legacy field name is retained for database mapping.
    pub default_use_regex: bool,
    pub shortcuts: ShortcutSettings,
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
            light_log_text_color: None,
            light_log_background_color: None,
            dark_log_text_color: None,
            dark_log_background_color: None,
            highlight_log_levels: false,
            log_level_color_rules: default_log_level_rules(),
            selection_styles: Default::default(),
            log_font_size: 13,
            search_toolbar_height: 28,
            search_toolbar_font_size: 13,
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
    pub(crate) fn search_toolbar_control_height(&self) -> u16 {
        let font_size = self.search_toolbar_font_size.clamp(
            *SEARCH_TOOLBAR_FONT_SIZE_RANGE.start(),
            *SEARCH_TOOLBAR_FONT_SIZE_RANGE.end(),
        );
        // One line at 1.25 times the font size, plus padding and borders.
        self.search_toolbar_height
            .clamp(
                *SEARCH_TOOLBAR_HEIGHT_RANGE.start(),
                *SEARCH_TOOLBAR_HEIGHT_RANGE.end(),
            )
            .max((font_size * 5).div_ceil(4) + 6)
    }

    pub fn search_result_limit(&self) -> Option<usize> {
        usize::try_from(self.max_search_results)
            .ok()
            .filter(|limit| *limit > 0)
    }

    pub(crate) fn log_text_color(&self, dark: bool) -> Option<&str> {
        if dark {
            self.dark_log_text_color.as_deref()
        } else {
            self.light_log_text_color.as_deref()
        }
    }

    pub(crate) fn log_background_color(&self, dark: bool) -> Option<&str> {
        if dark {
            self.dark_log_background_color.as_deref()
        } else {
            self.light_log_background_color.as_deref()
        }
    }
}

impl Default for FileSessionState {
    fn default() -> Self {
        Self {
            revision: 0,
            custom_title: None,
            selected_row: None,
            query_text: String::new(),
            result_mode: 0,
            marked_rows: CompressedRows::default(),
            show_line_numbers: true,
            show_row_separators: false,
            word_wrap: false,
            keyword_color_rules: Vec::new(),
            resume: TabResumeState::default(),
        }
    }
}

pub struct StateStore {
    repository: StateRepository,
}

impl StateStore {
    pub fn open_default() -> Result<Self> {
        let data_root =
            crate::app_paths::application_data_dir().context("无法确定本机应用数据目录")?;
        Self::open(data_root.join("sessions").join("vclogg2-state.db"))
    }

    fn open(database_path: PathBuf) -> Result<Self> {
        let defaults = StateMigrationDefaults {
            app_log_level: AppLogLevel::default().database_value().into(),
            color_labels: default_color_labels()
                .into_iter()
                .map(|label| ColorLabelRecord {
                    id: label.id,
                    name: label.name,
                    text_color: label.text_color,
                    text_alpha: label.text_alpha,
                    background_color: label.background_color,
                    background_alpha: label.background_alpha,
                })
                .collect(),
        };
        StateRepository::open(database_path, &defaults).map(|repository| Self { repository })
    }

    pub fn record_opened(&self, paths: &[PathBuf]) -> Result<()> {
        self.repository.record_opened(paths)
    }

    pub fn load_app_settings(&self) -> Result<AppSettings> {
        self.repository
            .load_app_settings()
            .map(|settings| settings.map(app_settings_from_record).unwrap_or_default())
    }

    pub(crate) fn save_highlight_settings(
        &self,
        settings: AppSettings,
        labels: &[ColorLabel],
    ) -> Result<()> {
        let labels = labels
            .iter()
            .map(|label| ColorLabelRecord {
                id: label.id.clone(),
                name: label.name.clone(),
                text_color: label.text_color,
                text_alpha: label.text_alpha,
                background_color: label.background_color,
                background_alpha: label.background_alpha,
            })
            .collect::<Vec<_>>();
        self.repository
            .save_highlight_settings(&app_settings_to_record(settings), &labels)
    }

    pub fn save_app_settings(&self, settings: AppSettings) -> Result<()> {
        self.repository
            .save_app_settings(&app_settings_to_record(settings))
    }

    pub fn load_last_settings_category(&self) -> Result<Option<String>> {
        self.repository.load_ui_value("settings.active_category")
    }

    pub fn save_last_settings_category(&self, category: &str) -> Result<()> {
        self.repository
            .save_ui_value("settings.active_category", category)
    }

    pub fn load_search_panel_height(&self) -> Result<Option<f32>> {
        let value = self
            .repository
            .load_ui_value("workspace.search_panel_height")?;
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
        self.repository
            .save_ui_value("workspace.search_panel_height", &height.to_string())
    }

    pub fn load_workspace_search_state(&self) -> Result<WorkspaceSearchState> {
        let value = self.repository.load_ui_value("workspace.search_contexts")?;
        Ok(value
            .as_deref()
            .and_then(|value| serde_json::from_str(value).ok())
            .and_then(WorkspaceSearchState::migrated)
            .unwrap_or_default())
    }

    pub fn save_workspace_search_state(&self, state: &WorkspaceSearchState) -> Result<()> {
        let value = serde_json::to_string(state).context("无法序列化搜索上下文")?;
        self.repository
            .save_ui_value("workspace.search_contexts", &value)
    }

    pub fn global_search_preferences(&self) -> Result<BTreeMap<PathBuf, bool>> {
        self.repository.global_search_preferences()
    }

    pub fn save_global_search_preferences(&self, preferences: &[(PathBuf, bool)]) -> Result<()> {
        self.repository.save_global_search_preferences(preferences)
    }

    pub fn load_search_history(&self) -> Result<Vec<String>> {
        self.repository
            .load_search_history()
            .map(normalize_search_history)
    }

    pub fn save_search_history(&self, history: &[String]) -> Result<()> {
        let history = normalize_search_history(history.iter().cloned());
        self.repository.save_search_history(&history)
    }

    pub fn load_predefined_filters(&self) -> Result<Vec<PredefinedFilter>> {
        let records = self.repository.load_predefined_filters()?;
        let mut filters = Vec::with_capacity(records.len());
        let mut needs_migration = false;
        for record in records {
            let value = serde_json::from_str::<serde_json::Value>(&record.json)
                .context("预定义过滤器记录格式无效")?;
            let filter = parse_stored_filter(&record.json).context("预定义过滤器记录格式无效")?;
            needs_migration |= value.get("id").is_some()
                || value.get("source").is_some()
                || value.get("published").is_some()
                || value
                    .get("uuid")
                    .and_then(serde_json::Value::as_str)
                    .and_then(crate::predefined_filters::FilterBranchId::parse)
                    .is_none()
                || record.id != filter.id.to_string();
            filters.push(filter);
        }
        let filters = normalize_predefined_filters(filters);
        if needs_migration {
            self.save_predefined_filters(&filters)
                .context("无法将预定义过滤器迁移到 v5")?;
        }
        Ok(filters)
    }

    pub fn save_predefined_filters(&self, filters: &[PredefinedFilter]) -> Result<()> {
        let filters = normalize_predefined_filters(filters.to_vec());
        let records = filters
            .iter()
            .map(|filter| {
                Ok(PredefinedFilterRecord {
                    id: filter.id.to_string(),
                    json: serde_json::to_string(filter).context("无法编码预定义过滤器")?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        self.repository.save_predefined_filters(&records)
    }

    pub fn load_cloud_settings(&self) -> Result<CloudSettings> {
        self.repository.load_cloud_settings()
    }

    pub fn save_cloud_settings(&self, settings: &CloudSettings) -> Result<()> {
        self.repository.save_cloud_settings(settings)
    }

    pub fn load_color_labels(&self) -> Result<Vec<ColorLabel>> {
        self.repository.load_color_labels().map(|labels| {
            labels.map_or_else(default_color_labels, |labels| {
                labels
                    .into_iter()
                    .map(|label| ColorLabel {
                        id: label.id,
                        name: label.name,
                        text_color: label.text_color,
                        text_alpha: label.text_alpha,
                        background_color: label.background_color,
                        background_alpha: label.background_alpha,
                    })
                    .collect()
            })
        })
    }

    pub fn recent_files(&self, limit: usize) -> Result<Vec<RecentFile>> {
        self.repository.recent_files(limit)
    }

    pub fn pinned_files(&self) -> Result<Vec<RecentFile>> {
        self.repository.pinned_files()
    }

    pub fn session_history(&self) -> Result<Vec<HistorySession>> {
        self.repository.session_history()
    }

    pub fn set_pinned(&self, path: &Path, pinned: bool) -> Result<()> {
        self.repository.set_pinned(path, pinned)
    }

    pub fn clear_pinned(&self) -> Result<()> {
        self.repository.clear_pinned()
    }

    pub fn clear_history(&self, open_paths: &[PathBuf]) -> Result<usize> {
        self.repository.clear_history(open_paths)
    }

    pub fn delete_history_session(&self, id: i64, open_paths: &[PathBuf]) -> Result<bool> {
        self.repository.delete_history_session(id, open_paths)
    }

    pub fn load_session(&self, path: &Path) -> Result<Option<FileSessionState>> {
        self.repository
            .load_session(path)
            .map(|record| record.map(file_session_from_record))
    }

    pub fn load_sessions(&self, paths: &[PathBuf]) -> Result<BTreeMap<PathBuf, FileSessionState>> {
        self.repository.load_sessions(paths).map(|sessions| {
            sessions
                .into_iter()
                .map(|(path, record)| (path, file_session_from_record(record)))
                .collect()
        })
    }

    pub fn last_workspace(&self) -> Result<Vec<LastWorkspaceFile>> {
        self.repository.last_workspace()
    }

    pub fn save_session(
        &self,
        path: &Path,
        base: &FileSessionState,
        state: &FileSessionState,
    ) -> Result<SessionSaveResult> {
        let result = self.repository.save_session(
            path,
            &file_session_to_record(base)?,
            &file_session_to_record(state)?,
        )?;
        Ok(SessionSaveResult {
            state: file_session_from_record(result.record),
            conflict_resolved: result.conflict_resolved,
        })
    }

    pub fn database_info(&self) -> Result<DatabaseInfo> {
        self.repository.database_info()
    }

    pub fn save_sessions(&self, sessions: &[(PathBuf, FileSessionState)]) -> Result<()> {
        self.repository.save_sessions(&session_records(sessions)?)
    }

    pub fn save_workspace(
        &self,
        sessions: &[(PathBuf, FileSessionState)],
        open_paths: &[PathBuf],
        active_path: Option<&Path>,
    ) -> Result<()> {
        self.repository
            .save_workspace(&session_records(sessions)?, open_paths, active_path)
    }
}

fn app_settings_from_record(record: AppSettingsRecord) -> AppSettings {
    AppSettings {
        app_log_level: AppLogLevel::from_database(&record.app_log_level),
        language: Language::from_database(&record.language),
        theme_preference: ThemePreference::from_database(&record.theme_preference),
        default_show_line_numbers: record.default_show_line_numbers,
        default_show_row_separators: record.default_show_row_separators,
        show_line_number_row_separators: record.show_line_number_row_separators,
        line_number_width: bounded_u16(record.line_number_width, 40, 160),
        line_number_text_color: normalize_optional_hex_color(record.line_number_text_color),
        line_number_background_color: normalize_optional_hex_color(
            record.line_number_background_color,
        ),
        light_log_text_color: normalize_optional_hex_color(record.light_log_text_color),
        light_log_background_color: normalize_optional_hex_color(record.light_log_background_color),
        dark_log_text_color: normalize_optional_hex_color(record.dark_log_text_color),
        dark_log_background_color: normalize_optional_hex_color(record.dark_log_background_color),
        highlight_log_levels: record.highlight_log_levels,
        selection_styles: serde_json::from_str(&record.selection_styles).unwrap_or_default(),
        log_level_color_rules: if record.log_level_color_rules.trim().is_empty() {
            default_log_level_rules()
        } else {
            serde_json::from_str(&record.log_level_color_rules)
                .unwrap_or_else(|_| default_log_level_rules())
        },
        log_font_size: bounded_u16(record.log_font_size, 8, 32),
        search_toolbar_height: bounded_u16(
            record.search_toolbar_height,
            *SEARCH_TOOLBAR_HEIGHT_RANGE.start(),
            *SEARCH_TOOLBAR_HEIGHT_RANGE.end(),
        ),
        search_toolbar_font_size: bounded_u16(
            record.search_toolbar_font_size,
            *SEARCH_TOOLBAR_FONT_SIZE_RANGE.start(),
            *SEARCH_TOOLBAR_FONT_SIZE_RANGE.end(),
        ),
        log_line_spacing: bounded_u16(record.log_line_spacing, 1, 40),
        log_font_family: LogFontFamily::from_database(&record.log_font_family),
        mouse_wheel_scroll_percent: bounded_u16(record.mouse_wheel_scroll_percent, 1, 400),
        scroll_by_line: record.scroll_by_line,
        mouse_wheel_scroll_lines: bounded_u16(record.mouse_wheel_scroll_lines, 1, 100),
        scroll_by_line_when_word_wrap: record.scroll_by_line_when_word_wrap,
        viewer_overscan: bounded_u16(record.viewer_overscan, 4, 40),
        reduce_motion: record.reduce_motion,
        confirm_close_tab: record.confirm_close_tab,
        show_full_path: record.show_full_path,
        open_directory_command: record.open_directory_command.chars().take(2048).collect(),
        max_search_results: u32::try_from(record.max_search_results.clamp(0, i64::from(u32::MAX)))
            .unwrap_or(u32::MAX),
        highlight_matches: record.highlight_matches,
        word_boundary_characters: record
            .word_boundary_characters
            .chars()
            .take(MAX_WORD_BOUNDARY_CHARACTERS)
            .collect(),
        default_case_sensitive: record.default_case_sensitive,
        default_use_regex: record.default_use_regex,
        shortcuts: ShortcutSettings {
            open_file: record.shortcut_open_file,
            focus_search: record.shortcut_focus_search,
            quick_find: record.shortcut_quick_find,
            close_tab: record.shortcut_close_tab,
            open_settings: record.shortcut_open_settings,
            toggle_case_sensitive: record.shortcut_toggle_case_sensitive,
            jump_to_bottom: record.shortcut_jump_to_bottom,
            cycle_color_label: record.shortcut_cycle_color_label,
            toggle_word_wrap: record.shortcut_toggle_word_wrap,
        },
    }
}

fn app_settings_to_record(settings: AppSettings) -> AppSettingsRecord {
    AppSettingsRecord {
        default_show_line_numbers: settings.default_show_line_numbers,
        default_show_row_separators: settings.default_show_row_separators,
        highlight_log_levels: settings.highlight_log_levels,
        selection_styles: serde_json::to_string(&settings.selection_styles).unwrap_or_default(),
        log_level_color_rules: serde_json::to_string(&settings.log_level_color_rules)
            .unwrap_or_else(|_| "[]".to_string()),
        log_font_size: i64::from(settings.log_font_size),
        search_toolbar_height: i64::from(settings.search_toolbar_control_height()),
        search_toolbar_font_size: i64::from(settings.search_toolbar_font_size),
        log_line_spacing: i64::from(settings.log_line_spacing),
        log_font_family: settings.log_font_family.database_value().into(),
        shortcut_open_file: settings.shortcuts.open_file,
        shortcut_focus_search: settings.shortcuts.focus_search,
        shortcut_quick_find: settings.shortcuts.quick_find,
        shortcut_close_tab: settings.shortcuts.close_tab,
        shortcut_open_settings: settings.shortcuts.open_settings,
        shortcut_toggle_case_sensitive: settings.shortcuts.toggle_case_sensitive,
        shortcut_jump_to_bottom: settings.shortcuts.jump_to_bottom,
        shortcut_cycle_color_label: settings.shortcuts.cycle_color_label,
        shortcut_toggle_word_wrap: settings.shortcuts.toggle_word_wrap,
        mouse_wheel_scroll_percent: i64::from(settings.mouse_wheel_scroll_percent),
        scroll_by_line: settings.scroll_by_line,
        mouse_wheel_scroll_lines: i64::from(settings.mouse_wheel_scroll_lines),
        scroll_by_line_when_word_wrap: settings.scroll_by_line_when_word_wrap,
        reduce_motion: settings.reduce_motion,
        confirm_close_tab: settings.confirm_close_tab,
        show_full_path: settings.show_full_path,
        max_search_results: i64::from(settings.max_search_results),
        highlight_matches: settings.highlight_matches,
        word_boundary_characters: settings.word_boundary_characters,
        default_case_sensitive: settings.default_case_sensitive,
        default_use_regex: settings.default_use_regex,
        show_line_number_row_separators: settings.show_line_number_row_separators,
        line_number_width: i64::from(settings.line_number_width.clamp(40, 160)),
        line_number_text_color: normalize_optional_hex_color(settings.line_number_text_color),
        line_number_background_color: normalize_optional_hex_color(
            settings.line_number_background_color,
        ),
        light_log_text_color: normalize_optional_hex_color(settings.light_log_text_color),
        light_log_background_color: normalize_optional_hex_color(
            settings.light_log_background_color,
        ),
        dark_log_text_color: normalize_optional_hex_color(settings.dark_log_text_color),
        dark_log_background_color: normalize_optional_hex_color(settings.dark_log_background_color),
        theme_preference: settings.theme_preference.database_value().into(),
        open_directory_command: settings.open_directory_command.chars().take(2048).collect(),
        viewer_overscan: i64::from(settings.viewer_overscan.clamp(4, 40)),
        language: settings.language.database_value().into(),
        app_log_level: settings.app_log_level.database_value().into(),
    }
}

fn bounded_u16(value: i64, minimum: u16, maximum: u16) -> u16 {
    u16::try_from(value.clamp(i64::from(minimum), i64::from(maximum))).unwrap_or(maximum)
}

fn file_session_from_record(record: FileSessionRecord) -> FileSessionState {
    let marked_rows = decode_marked_rows(&record.marked_rows);
    let legacy_results_visible =
        !record.query_text.is_empty() || (record.result_mode != 1 && !marked_rows.is_empty());
    let resume = serde_json::from_str(&record.resume_state)
        .ok()
        .filter(TabResumeState::is_compatible)
        .unwrap_or_else(|| {
            TabResumeState::from_legacy(record.selected_row, legacy_results_visible)
        });
    FileSessionState {
        revision: record.revision,
        custom_title: record.custom_title,
        selected_row: record.selected_row,
        query_text: record.query_text,
        result_mode: record.result_mode,
        marked_rows,
        show_line_numbers: record.show_line_numbers,
        show_row_separators: record.show_row_separators,
        keyword_color_rules: decode_rules(&record.keyword_color_rules),
        word_wrap: record.word_wrap,
        resume,
    }
}

fn file_session_to_record(state: &FileSessionState) -> Result<FileSessionRecord> {
    Ok(FileSessionRecord {
        revision: state.revision,
        custom_title: state.custom_title.clone(),
        selected_row: state.selected_row,
        query_text: state.query_text.clone(),
        result_mode: state.result_mode,
        marked_rows: encode_marked_rows(&state.marked_rows),
        show_line_numbers: state.show_line_numbers,
        show_row_separators: state.show_row_separators,
        word_wrap: state.word_wrap,
        keyword_color_rules: encode_rules(&state.keyword_color_rules),
        resume_state: serde_json::to_string(&state.resume).context("无法序列化标签恢复状态")?,
    })
}

fn session_records(
    sessions: &[(PathBuf, FileSessionState)],
) -> Result<Vec<(PathBuf, FileSessionRecord)>> {
    sessions
        .iter()
        .map(|(path, state)| Ok((path.clone(), file_session_to_record(state)?)))
        .collect()
}

fn normalize_optional_hex_color(value: Option<String>) -> Option<String> {
    let value = value?.trim().to_ascii_lowercase();
    (value.len() == 7
        && value.starts_with('#')
        && value[1..].bytes().all(|byte| byte.is_ascii_hexdigit()))
    .then_some(value)
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

#[cfg(test)]
fn count_marked_rows(marked_rows: &str) -> usize {
    decode_marked_rows(marked_rows).len()
}

fn encode_marked_rows(rows: &CompressedRows) -> String {
    if rows.is_empty() {
        return String::new();
    }
    let bytes = rows.to_portable_bytes();
    let mut encoded = String::with_capacity(COMPRESSED_MARKED_ROWS_PREFIX.len() + bytes.len() * 2);
    encoded.push_str(COMPRESSED_MARKED_ROWS_PREFIX);
    for byte in bytes {
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}

fn decode_marked_rows(stored: &str) -> CompressedRows {
    if let Some(encoded) = stored.strip_prefix(COMPRESSED_MARKED_ROWS_PREFIX) {
        return decode_hex(encoded)
            .and_then(|bytes| CompressedRows::from_portable_bytes(&bytes))
            .unwrap_or_default();
    }
    stored
        .split(',')
        .filter_map(|value| value.parse::<usize>().ok())
        .collect()
}

#[cfg(test)]
mod session_load_tests {
    use std::{fs, hint::black_box, path::PathBuf, time::Instant};

    #[cfg(unix)]
    use std::{ffi::OsString, os::unix::ffi::OsStringExt as _};

    use super::{
        AppSettings, COMPRESSED_MARKED_ROWS_PREFIX, FileSessionState, PredefinedFilterRecord,
        StateStore, count_marked_rows, decode_marked_rows, encode_marked_rows,
    };
    use vclogg_core::CompressedRows;
    use vclogg_data::{decode_persisted_path, encode_persisted_path};

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
            marked_rows: [3, 9].into_iter().collect(),
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

    #[test]
    fn marked_rows_use_compressed_storage_and_accept_legacy_lists() {
        let rows = CompressedRows::from_inclusive_ranges([(0, 999_999), (2_000_000, 2_000_010)]);

        let encoded = encode_marked_rows(&rows);

        assert!(encoded.starts_with(COMPRESSED_MARKED_ROWS_PREFIX));
        assert!(encoded.len() < 2048);
        assert_eq!(decode_marked_rows(&encoded), rows);
        assert_eq!(count_marked_rows(&encoded), 1_000_011);
        assert_eq!(
            decode_marked_rows("3,9,10"),
            [3, 9, 10].into_iter().collect()
        );

        let database = TemporaryDatabase::new("compressed-marked-rows");
        let path = database.0.with_file_name("dense-marks.log");
        let store = StateStore::open(database.0.clone()).expect("应能打开测试状态库");
        let mut state = session("dense");
        state.marked_rows = rows.clone();
        store
            .save_sessions(&[(path.clone(), state)])
            .expect("应能保存压缩标记会话");
        let persisted = store
            .repository
            .load_session(&path)
            .expect("应能读取原始会话记录")
            .expect("原始会话记录应存在")
            .marked_rows;
        assert_eq!(persisted, encoded);
        assert_eq!(
            store
                .load_session(&path)
                .expect("应能读取压缩标记会话")
                .expect("压缩标记会话应存在")
                .marked_rows,
            rows
        );
        assert_eq!(
            store.session_history().expect("应能读取压缩标记历史")[0].marked_rows_count,
            1_000_011
        );
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_paths_keep_distinct_persisted_sessions_and_preferences() {
        let database = TemporaryDatabase::new("non-utf8-path-correctness");
        let directory = database.0.parent().expect("测试数据库应有父目录");
        let first = directory.join(OsString::from_vec(b"source-\x80.log".to_vec()));
        let second = directory.join(OsString::from_vec(b"source-\x81.log".to_vec()));
        assert_eq!(first.to_string_lossy(), second.to_string_lossy());
        assert_ne!(
            encode_persisted_path(&first),
            encode_persisted_path(&second)
        );
        assert_eq!(
            decode_persisted_path(&encode_persisted_path(&first)),
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
            .repository
            .schema_version()
            .expect("应能读取测试状态库版本");

        assert_eq!(version, vclogg_data::STATE_SCHEMA_VERSION);
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
    fn app_settings_round_trip_through_data_record() {
        let database = TemporaryDatabase::new("app-settings-record");
        let store = StateStore::open(database.0.clone()).expect("应能打开测试状态库");
        let mut expected = AppSettings {
            log_font_size: 18,
            log_line_spacing: 9,
            word_boundary_characters: "._-".into(),
            open_directory_command: "tool --path {directory}".into(),
            line_number_text_color: Some("#AABBCC".into()),
            light_log_text_color: Some("#112233".into()),
            light_log_background_color: Some("#F8F7F4".into()),
            dark_log_text_color: Some("#DDEEFF".into()),
            dark_log_background_color: Some("#101722".into()),
            default_case_sensitive: true,
            default_use_regex: true,
            ..AppSettings::default()
        };
        store
            .save_app_settings(expected.clone())
            .expect("应能保存应用设置记录");
        drop(store);

        let actual = StateStore::open(database.0.clone())
            .expect("应能重新打开测试状态库")
            .load_app_settings()
            .expect("应能读取应用设置记录");

        expected.line_number_text_color = Some("#aabbcc".into());
        expected.light_log_text_color = Some("#112233".into());
        expected.light_log_background_color = Some("#f8f7f4".into());
        expected.dark_log_text_color = Some("#ddeeff".into());
        expected.dark_log_background_color = Some("#101722".into());
        assert_eq!(actual, expected);
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
            .repository
            .save_predefined_filters(&[PredefinedFilterRecord {
                id: "filter-1".into(),
                json: legacy,
            }])
            .unwrap();

        let loaded = store.load_predefined_filters().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id.to_string(), uuid);
        assert_eq!(loaded[0].remote_references.len(), 1);

        let stored = store.repository.load_predefined_filters().unwrap();
        assert_eq!(stored[0].id, uuid);
        assert!(stored[0].json.contains("\"remoteReferences\""));
        assert!(!stored[0].json.contains("\"id\""));
        assert!(!stored[0].json.contains("\"published\""));
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
