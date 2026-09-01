use serde::{Deserialize, Serialize};
use vclogg_core::CompressedRows;

pub(crate) const SEARCH_CONTEXT_VERSION: u32 = 1;
const PIXEL_SCALE: f32 = 1_000.;
const COMPRESSED_ROWS_PREFIX: &str = "rb1:";

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compressed_rows: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub rows: Vec<PersistedRowRange>,
}

impl PersistedPathSelection {
    pub(crate) fn new(path: String, rows: &CompressedRows) -> Self {
        Self {
            path,
            compressed_rows: Some(encode_compressed_rows(rows)),
            rows: Vec::new(),
        }
    }

    pub(crate) fn decoded_rows(&self) -> CompressedRows {
        self.compressed_rows
            .as_deref()
            .and_then(decode_compressed_rows)
            .unwrap_or_else(|| {
                CompressedRows::from_inclusive_ranges(self.rows.iter().filter_map(|range| {
                    Some((
                        usize::try_from(range.start).ok()?,
                        usize::try_from(range.end).ok()?,
                    ))
                }))
            })
    }
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

fn encode_compressed_rows(rows: &CompressedRows) -> String {
    let bytes = rows.to_portable_bytes();
    let mut encoded = String::with_capacity(COMPRESSED_ROWS_PREFIX.len() + bytes.len() * 2);
    encoded.push_str(COMPRESSED_ROWS_PREFIX);
    push_hex(&mut encoded, &bytes);
    encoded
}

fn decode_compressed_rows(encoded: &str) -> Option<CompressedRows> {
    let bytes = decode_hex(encoded.strip_prefix(COMPRESSED_ROWS_PREFIX)?)?;
    CompressedRows::from_portable_bytes(&bytes)
}

fn push_hex(destination: &mut String, bytes: &[u8]) {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    for byte in bytes {
        destination.push(DIGITS[usize::from(byte >> 4)] as char);
        destination.push(DIGITS[usize::from(byte & 0x0f)] as char);
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persisted_selection_keeps_dense_rows_compressed() {
        let rows = CompressedRows::from_inclusive_ranges([(0, 999_999), (2_000_000, 2_000_010)]);
        let selection = PersistedPathSelection::new("source.log".to_owned(), &rows);

        let json = serde_json::to_string(&selection).expect("selection should serialize");
        let restored: PersistedPathSelection =
            serde_json::from_str(&json).expect("selection should deserialize");

        assert_eq!(restored.decoded_rows(), rows);
        assert!(json.len() < 2048);
        assert!(!json.contains("\"rows\""));
    }

    #[test]
    fn persisted_selection_accepts_legacy_ranges() {
        let selection: PersistedPathSelection = serde_json::from_str(
            r#"{"path":"source.log","rows":[{"start":3,"end":5},{"start":9,"end":9}]}"#,
        )
        .expect("legacy selection should deserialize");

        assert_eq!(selection.decoded_rows(), [3, 4, 5, 9].into_iter().collect());
    }
}
