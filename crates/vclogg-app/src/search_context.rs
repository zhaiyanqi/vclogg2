use std::{cmp::Reverse, collections::BTreeSet};

use serde::{Deserialize, Serialize};
use vclogg_core::CompressedRows;

use crate::{
    color_labels::KeywordColorRule,
    path_identity::{decode_persisted_path, normalized_path_match_key},
};

pub(crate) const SEARCH_CONTEXT_VERSION: u32 = 3;
const LEGACY_SEARCH_CONTEXT_VERSION: u32 = 1;
const PREVIOUS_SEARCH_CONTEXT_VERSION: u32 = 2;
pub(crate) const MAX_DIRECTORY_SEARCH_SESSIONS: usize = 20;
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
    pub keyword_color_rules: Vec<KeywordColorRule>,
    /// Sources captured by the last completed search. All-open-files restoration uses these
    /// paths even when the corresponding documents are not currently open as tabs.
    pub source_paths: Vec<String>,
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
            keyword_color_rules: Vec::new(),
            source_paths: Vec::new(),
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

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct PersistedDirectorySearchSession {
    pub directory: String,
    pub options: PersistedDirectorySearchOptions,
    pub context: PersistedGlobalSearchContext,
    pub last_used: u64,
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
    /// Version-1 compatibility fields. Version 2 writes the active directory here as well so a
    /// downgrade still recovers the most recently used directory search.
    pub directory: PersistedGlobalSearchContext,
    pub directory_options: PersistedDirectorySearchOptions,
    pub active_directory: Option<String>,
    pub directories: Vec<PersistedDirectorySearchSession>,
}

impl Default for WorkspaceSearchState {
    fn default() -> Self {
        Self {
            version: SEARCH_CONTEXT_VERSION,
            active_scope: PersistedSearchScope::CurrentFile,
            all_open: PersistedGlobalSearchContext::default(),
            directory: PersistedGlobalSearchContext::default(),
            directory_options: PersistedDirectorySearchOptions::default(),
            active_directory: None,
            directories: Vec::new(),
        }
    }
}

impl WorkspaceSearchState {
    pub fn migrated(mut self) -> Option<Self> {
        match self.version {
            LEGACY_SEARCH_CONTEXT_VERSION => {
                self.version = SEARCH_CONTEXT_VERSION;
                self.all_open.version = SEARCH_CONTEXT_VERSION;
                self.directory.version = SEARCH_CONTEXT_VERSION;
                if let Some(directory) = self.directory_options.directory.clone() {
                    self.active_directory = Some(directory.clone());
                    self.directories.push(PersistedDirectorySearchSession {
                        directory,
                        options: self.directory_options.clone(),
                        context: self.directory.clone(),
                        last_used: 1,
                    });
                }
            }
            PREVIOUS_SEARCH_CONTEXT_VERSION => {
                self.version = SEARCH_CONTEXT_VERSION;
                self.all_open.version = SEARCH_CONTEXT_VERSION;
                self.directory.version = SEARCH_CONTEXT_VERSION;
            }
            SEARCH_CONTEXT_VERSION => {}
            _ => return None,
        }
        if self.all_open.version != SEARCH_CONTEXT_VERSION {
            return None;
        }
        self.directory.version = SEARCH_CONTEXT_VERSION;
        for session in &mut self.directories {
            if !matches!(
                session.context.version,
                LEGACY_SEARCH_CONTEXT_VERSION
                    | PREVIOUS_SEARCH_CONTEXT_VERSION
                    | SEARCH_CONTEXT_VERSION
            ) {
                return None;
            }
            session.context.version = SEARCH_CONTEXT_VERSION;
            session.options.directory = Some(session.directory.clone());
        }
        self.normalize_directory_sessions();
        Some(self)
    }

    pub(crate) fn normalize_directory_sessions(&mut self) {
        self.directories
            .sort_by_key(|session| Reverse(session.last_used));
        let mut seen = BTreeSet::new();
        self.directories.retain(|session| {
            !session.directory.is_empty()
                && seen.insert(normalized_path_match_key(&decode_persisted_path(
                    &session.directory,
                )))
        });
        self.directories.truncate(MAX_DIRECTORY_SEARCH_SESSIONS);

        if let Some(active) = self.active_directory.as_deref() {
            let active_key = normalized_path_match_key(&decode_persisted_path(active));
            if !self.directories.iter().any(|session| {
                normalized_path_match_key(&decode_persisted_path(&session.directory)) == active_key
            }) {
                self.active_directory = self
                    .directories
                    .first()
                    .map(|session| session.directory.clone());
            }
        }
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

    #[test]
    fn version_one_directory_state_migrates_to_a_keyed_session() {
        let legacy = WorkspaceSearchState {
            version: LEGACY_SEARCH_CONTEXT_VERSION,
            active_scope: PersistedSearchScope::Directory,
            directory: PersistedGlobalSearchContext {
                version: LEGACY_SEARCH_CONTEXT_VERSION,
                results_visible: true,
                ..PersistedGlobalSearchContext::default()
            },
            directory_options: PersistedDirectorySearchOptions {
                directory: Some("logs".into()),
                ..PersistedDirectorySearchOptions::default()
            },
            ..WorkspaceSearchState::default()
        };

        let migrated = legacy.migrated().expect("version one should migrate");
        assert_eq!(migrated.version, SEARCH_CONTEXT_VERSION);
        assert_eq!(migrated.active_directory.as_deref(), Some("logs"));
        assert_eq!(migrated.directories.len(), 1);
        assert!(migrated.directories[0].context.results_visible);
    }

    #[test]
    fn version_two_state_migrates_with_an_empty_source_list() {
        let previous = WorkspaceSearchState {
            version: PREVIOUS_SEARCH_CONTEXT_VERSION,
            all_open: PersistedGlobalSearchContext {
                version: PREVIOUS_SEARCH_CONTEXT_VERSION,
                results_visible: true,
                ..PersistedGlobalSearchContext::default()
            },
            directory: PersistedGlobalSearchContext {
                version: PREVIOUS_SEARCH_CONTEXT_VERSION,
                ..PersistedGlobalSearchContext::default()
            },
            ..WorkspaceSearchState::default()
        };

        let migrated = previous.migrated().expect("version two should migrate");

        assert_eq!(migrated.version, SEARCH_CONTEXT_VERSION);
        assert_eq!(migrated.all_open.version, SEARCH_CONTEXT_VERSION);
        assert!(migrated.all_open.source_paths.is_empty());
    }

    #[test]
    fn directory_sessions_are_deduplicated_and_lru_bounded() {
        let mut state = WorkspaceSearchState {
            directories: (0..=MAX_DIRECTORY_SEARCH_SESSIONS)
                .map(|ix| PersistedDirectorySearchSession {
                    directory: format!("logs/{ix}"),
                    last_used: ix as u64,
                    ..PersistedDirectorySearchSession::default()
                })
                .chain([PersistedDirectorySearchSession {
                    directory: format!("logs/archive/../{}", MAX_DIRECTORY_SEARCH_SESSIONS),
                    last_used: 100,
                    ..PersistedDirectorySearchSession::default()
                }])
                .collect(),
            ..WorkspaceSearchState::default()
        };

        state.normalize_directory_sessions();

        assert_eq!(state.directories.len(), MAX_DIRECTORY_SEARCH_SESSIONS);
        assert_eq!(
            state.directories[0].directory,
            format!("logs/archive/../{}", MAX_DIRECTORY_SEARCH_SESSIONS)
        );
        assert_eq!(
            state
                .directories
                .iter()
                .filter(|session| normalized_path_match_key(&decode_persisted_path(
                    &session.directory
                )) == normalized_path_match_key(std::path::Path::new(&format!(
                    "logs/{}",
                    MAX_DIRECTORY_SEARCH_SESSIONS
                ))))
                .count(),
            1
        );
    }

    #[test]
    fn serialized_scopes_keep_independent_query_text_and_presentation_state() {
        let state = WorkspaceSearchState {
            active_scope: PersistedSearchScope::Directory,
            all_open: PersistedGlobalSearchContext {
                query: PersistedSearchQuery {
                    text: "all open".into(),
                },
                result_mode: 2,
                word_wrap: true,
                source_paths: vec!["logs/a.log".into(), "logs/b.log".into()],
                keyword_color_rules: vec![KeywordColorRule {
                    label_id: Some("warning".into()),
                    keyword: "needle".into(),
                    color: 0xf28e2b,
                    alpha: 179,
                    case_sensitive: true,
                    enabled: true,
                }],
                ..PersistedGlobalSearchContext::default()
            },
            active_directory: Some("logs/x".into()),
            directories: vec![PersistedDirectorySearchSession {
                directory: "logs/x".into(),
                context: PersistedGlobalSearchContext {
                    query: PersistedSearchQuery {
                        text: "directory".into(),
                    },
                    result_mode: 1,
                    results_visible: true,
                    collapsed_paths: vec!["logs/x/a.log".into()],
                    word_wrap: false,
                    ..PersistedGlobalSearchContext::default()
                },
                last_used: 4,
                ..PersistedDirectorySearchSession::default()
            }],
            ..WorkspaceSearchState::default()
        };

        let json = serde_json::to_string(&state).expect("workspace search state should serialize");
        let restored: WorkspaceSearchState =
            serde_json::from_str(&json).expect("workspace search state should deserialize");
        let restored = restored
            .migrated()
            .expect("version two should be compatible");

        assert_eq!(restored.all_open.keyword_color_rules.len(), 1);
        assert_eq!(restored.all_open.keyword_color_rules[0].keyword, "needle");

        assert_eq!(restored.all_open.query.text, "all open");
        assert!(restored.all_open.word_wrap);
        assert_eq!(restored.all_open.source_paths, ["logs/a.log", "logs/b.log"]);
        assert_eq!(restored.directories[0].context.query.text, "directory");
        assert!(restored.directories[0].context.results_visible);
        assert_eq!(
            restored.directories[0].context.collapsed_paths,
            ["logs/x/a.log"]
        );
        let serialized: serde_json::Value =
            serde_json::from_str(&json).expect("serialized state should be JSON");
        let all_open_query = &serialized["all_open"]["query"];
        let directory_query = &serialized["directories"][0]["context"]["query"];
        assert!(all_open_query.get("case_sensitive").is_none());
        assert!(all_open_query.get("regex").is_none());
        assert!(directory_query.get("case_sensitive").is_none());
        assert!(directory_query.get("regex").is_none());
    }

    #[test]
    fn legacy_query_options_are_ignored_and_not_written_again() {
        let query: PersistedSearchQuery =
            serde_json::from_str(r#"{"text":"needle","case_sensitive":true,"regex":true}"#)
                .expect("legacy query should deserialize");

        assert_eq!(query.text, "needle");
        assert_eq!(
            serde_json::to_string(&query).expect("query should serialize"),
            r#"{"text":"needle"}"#
        );
    }
}
