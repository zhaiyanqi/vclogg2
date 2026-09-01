//! Stable records returned by state persistence repositories.

use std::path::PathBuf;

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
