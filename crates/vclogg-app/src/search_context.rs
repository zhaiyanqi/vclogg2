use serde::{Deserialize, Serialize};

pub(crate) const SEARCH_CONTEXT_VERSION: u32 = 1;
const PIXEL_SCALE: f32 = 1_000.;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PersistedSearchScope {
    #[default]
    CurrentFile,
    AllOpenFiles,
    Directory,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct PersistedSearchQuery {
    pub text: String,
    pub case_sensitive: bool,
    pub regex: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct PersistedRowRange {
    pub start: u64,
    pub end: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct PersistedPathSelection {
    pub path: String,
    pub rows: Vec<PersistedRowRange>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct PersistedSearchRowKey {
    pub path: String,
    pub source_row: Option<usize>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct PersistedSearchViewport {
    pub key: PersistedSearchRowKey,
    pub viewport_y_milli: i64,
    pub horizontal_offset_milli: i64,
    pub at_end: bool,
    pub fallback_ix: usize,
}

impl PersistedSearchViewport {
    pub fn new(
        key: PersistedSearchRowKey,
        viewport_y: f32,
        horizontal_offset: f32,
        at_end: bool,
        fallback_ix: usize,
    ) -> Self {
        Self {
            key,
            viewport_y_milli: encode_pixels(viewport_y),
            horizontal_offset_milli: encode_pixels(horizontal_offset),
            at_end,
            fallback_ix,
        }
    }

    pub fn viewport_y(&self) -> f32 {
        decode_pixels(self.viewport_y_milli)
    }

    pub fn horizontal_offset(&self) -> f32 {
        decode_pixels(self.horizontal_offset_milli)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct PersistedGlobalSearchContext {
    pub version: u32,
    pub query: PersistedSearchQuery,
    pub result_mode: i64,
    pub results_visible: bool,
    pub word_wrap: bool,
    pub collapsed_paths: Vec<String>,
    pub selection: Vec<PersistedPathSelection>,
    pub selected_row: Option<PersistedSearchRowKey>,
    pub viewport: Option<PersistedSearchViewport>,
    pub active: bool,
}

impl Default for PersistedGlobalSearchContext {
    fn default() -> Self {
        Self {
            version: SEARCH_CONTEXT_VERSION,
            query: PersistedSearchQuery::default(),
            result_mode: 0,
            results_visible: false,
            word_wrap: false,
            collapsed_paths: Vec::new(),
            selection: Vec::new(),
            selected_row: None,
            viewport: None,
            active: false,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct PersistedDirectorySearchOptions {
    pub directory: Option<String>,
    // Kept so version-1 state written by the former preset selector can migrate.
    pub file_type: u8,
    pub file_type_filter_enabled: Option<bool>,
    pub file_type_patterns: Option<String>,
    pub include_subdirectories: bool,
    pub include_hidden_directories: bool,
}

impl Default for PersistedDirectorySearchOptions {
    fn default() -> Self {
        Self {
            directory: None,
            file_type: 0,
            file_type_filter_enabled: None,
            file_type_patterns: None,
            include_subdirectories: true,
            include_hidden_directories: false,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct WorkspaceSearchState {
    pub version: u32,
    pub active_scope: PersistedSearchScope,
    pub all_open: PersistedGlobalSearchContext,
    pub directory: PersistedGlobalSearchContext,
    pub directory_options: PersistedDirectorySearchOptions,
}

impl Default for WorkspaceSearchState {
    fn default() -> Self {
        Self {
            version: SEARCH_CONTEXT_VERSION,
            active_scope: PersistedSearchScope::CurrentFile,
            all_open: PersistedGlobalSearchContext::default(),
            directory: PersistedGlobalSearchContext::default(),
            directory_options: PersistedDirectorySearchOptions::default(),
        }
    }
}

impl WorkspaceSearchState {
    pub fn is_compatible(&self) -> bool {
        self.version == SEARCH_CONTEXT_VERSION
            && self.all_open.version == SEARCH_CONTEXT_VERSION
            && self.directory.version == SEARCH_CONTEXT_VERSION
    }
}

pub(crate) fn compress_rows(rows: impl IntoIterator<Item = usize>) -> Vec<PersistedRowRange> {
    let mut ranges = Vec::<PersistedRowRange>::new();
    for row in rows.into_iter().filter_map(|row| u64::try_from(row).ok()) {
        if let Some(last) = ranges.last_mut()
            && row <= last.end.saturating_add(1)
        {
            last.end = last.end.max(row);
        } else {
            ranges.push(PersistedRowRange {
                start: row,
                end: row,
            });
        }
    }
    ranges
}

fn encode_pixels(value: f32) -> i64 {
    if value.is_finite() {
        (value * PIXEL_SCALE).round() as i64
    } else {
        0
    }
}

fn decode_pixels(value: i64) -> f32 {
    value as f32 / PIXEL_SCALE
}
