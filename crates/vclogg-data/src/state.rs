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
    pub case_sensitive: bool,
    pub regex: bool,
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
