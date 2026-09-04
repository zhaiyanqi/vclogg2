//! Stable records returned by state persistence repositories.

use std::{collections::BTreeMap, path::PathBuf};

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

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CloudSettings {
    pub server_url: String,
    pub display_name: String,
}

/// Storage representation of one file session. Presentation payloads remain opaque to data.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FileSessionRecord {
    pub revision: i64,
    pub custom_title: Option<String>,
    pub selected_row: Option<usize>,
    pub query_text: String,
    pub result_mode: i64,
    pub marked_rows: String,
    pub show_line_numbers: bool,
    pub show_row_separators: bool,
    pub word_wrap: bool,
    pub keyword_color_rules: String,
    pub resume_state: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionRecordSaveResult {
    pub record: FileSessionRecord,
    pub conflict_resolved: bool,
}

pub type FileSessionRecords = BTreeMap<PathBuf, FileSessionRecord>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PredefinedFilterRecord {
    pub id: String,
    pub json: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ColorLabelRecord {
    pub id: String,
    pub name: String,
    pub text_color: u32,
    pub text_alpha: u8,
    pub background_color: u32,
    pub background_alpha: u8,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppSettingsRecord {
    pub default_show_line_numbers: bool,
    pub default_show_row_separators: bool,
    pub highlight_log_levels: bool,
    pub log_level_color_rules: String,
    pub log_font_size: i64,
    pub log_line_spacing: i64,
    pub log_font_family: String,
    pub shortcut_open_file: String,
    pub shortcut_focus_search: String,
    pub shortcut_quick_find: String,
    pub shortcut_close_tab: String,
    pub shortcut_open_settings: String,
    pub shortcut_toggle_case_sensitive: String,
    pub shortcut_jump_to_bottom: String,
    pub shortcut_cycle_color_label: String,
    pub shortcut_toggle_word_wrap: String,
    pub mouse_wheel_scroll_percent: i64,
    pub scroll_by_line: bool,
    pub mouse_wheel_scroll_lines: i64,
    pub scroll_by_line_when_word_wrap: bool,
    pub reduce_motion: bool,
    pub confirm_close_tab: bool,
    pub show_full_path: bool,
    pub max_search_results: i64,
    pub highlight_matches: bool,
    pub word_boundary_characters: String,
    pub default_case_sensitive: bool,
    pub default_use_regex: bool,
    pub show_line_number_row_separators: bool,
    pub line_number_width: i64,
    pub line_number_text_color: Option<String>,
    pub line_number_background_color: Option<String>,
    pub light_log_text_color: Option<String>,
    pub light_log_background_color: Option<String>,
    pub dark_log_text_color: Option<String>,
    pub dark_log_background_color: Option<String>,
    pub theme_preference: String,
    pub open_directory_command: String,
    pub viewer_overscan: i64,
    pub language: String,
    pub app_log_level: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StateMigrationDefaults {
    pub app_log_level: String,
    pub color_labels: Vec<ColorLabelRecord>,
}
