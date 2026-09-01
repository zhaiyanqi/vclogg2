use std::{collections::BTreeMap, sync::Arc};

use gpui::SharedString;
use vclogg_core::CompressedRows;

use crate::{
    color_labels::{KeywordColorRule, ResolvedColorRules},
    path_identity::{PathMatchKey, decode_persisted_path, normalized_path_match_key},
    search_context::{MAX_DIRECTORY_SEARCH_SESSIONS, PersistedDirectorySearchSession},
};

use super::{SelectionTable, TabResumeState};

/// File-owned annotations and naming. Every log/result projection borrows a snapshot from this
/// single source instead of keeping scope-specific copies.
pub(super) struct FileState {
    pub(super) title: SharedString,
    pub(super) custom_title: Option<String>,
    pub(super) marked_rows: CompressedRows,
    pub(super) pending_restore_marked_rows: CompressedRows,
    pub(super) keyword_color_rules: Vec<KeywordColorRule>,
    pub(super) resolved_color_rules: Arc<ResolvedColorRules>,
}

/// Lightweight per-file presentation state. The shared log/result displayers install this state
/// when a tab becomes active; decoded rows and GPUI surface entities deliberately do not live
/// here.
pub(super) struct FileViewState {
    pub(super) auto_follow: bool,
    pub(super) show_line_numbers: bool,
    pub(super) show_row_separators: bool,
    pub(super) word_wrap: bool,
    pub(super) selection_table: SelectionTable,
    pub(super) uses_default_view_options: bool,
    pub(super) pending_restore_row: Option<usize>,
    pub(super) pending_resume: Option<TabResumeState>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum SearchSessionKey {
    CurrentFile(u64),
    AllOpenFiles,
    Directory(PathMatchKey),
}

#[derive(Default)]
pub(super) struct WorkspaceViewState {
    pub(super) active_search: Option<SearchSessionKey>,
    directory_sessions: BTreeMap<PathMatchKey, (PersistedDirectorySearchSession, u64)>,
    directory_clock: u64,
}

impl WorkspaceViewState {
    pub(super) fn restore_directory_sessions(
        &mut self,
        sessions: impl IntoIterator<Item = PersistedDirectorySearchSession>,
    ) {
        self.directory_sessions.clear();
        self.directory_clock = 0;
        for session in sessions {
            self.directory_clock = self.directory_clock.max(session.last_used);
            let key = normalized_path_match_key(&decode_persisted_path(&session.directory));
            let last_used = session.last_used;
            match self.directory_sessions.get(&key) {
                Some((_, existing_last_used)) if *existing_last_used > last_used => {}
                _ => {
                    self.directory_sessions.insert(key, (session, last_used));
                }
            }
        }
        self.prune_directory_sessions();
    }

    pub(super) fn remember_directory_session(
        &mut self,
        mut session: PersistedDirectorySearchSession,
    ) {
        if session.directory.is_empty() {
            return;
        }
        self.directory_clock = self
            .directory_clock
            .max(session.last_used)
            .saturating_add(1);
        session.last_used = self.directory_clock;
        let key = normalized_path_match_key(&decode_persisted_path(&session.directory));
        self.directory_sessions
            .insert(key, (session, self.directory_clock));
        self.prune_directory_sessions();
    }

    pub(super) fn directory_session(
        &mut self,
        directory: &std::path::Path,
    ) -> Option<PersistedDirectorySearchSession> {
        let key = normalized_path_match_key(directory);
        let (session, last_used) = self.directory_sessions.get_mut(&key)?;
        self.directory_clock = self.directory_clock.saturating_add(1);
        *last_used = self.directory_clock;
        session.last_used = self.directory_clock;
        Some(session.clone())
    }

    pub(super) fn directory_sessions(&self) -> Vec<PersistedDirectorySearchSession> {
        let mut sessions = self
            .directory_sessions
            .values()
            .map(|(session, _)| session.clone())
            .collect::<Vec<_>>();
        sessions.sort_by_key(|session| std::cmp::Reverse(session.last_used));
        sessions
    }

    fn prune_directory_sessions(&mut self) {
        while self.directory_sessions.len() > MAX_DIRECTORY_SEARCH_SESSIONS {
            let Some(oldest) = self
                .directory_sessions
                .iter()
                .min_by_key(|(_, (_, last_used))| *last_used)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            self.directory_sessions.remove(&oldest);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directory_registry_keeps_only_the_most_recent_twenty_sessions() {
        let mut state = WorkspaceViewState::default();
        for ix in 0..=MAX_DIRECTORY_SEARCH_SESSIONS {
            state.remember_directory_session(PersistedDirectorySearchSession {
                directory: format!("logs/{ix}"),
                ..PersistedDirectorySearchSession::default()
            });
        }

        let sessions = state.directory_sessions();
        assert_eq!(sessions.len(), MAX_DIRECTORY_SEARCH_SESSIONS);
        assert_eq!(
            sessions.first().map(|session| session.directory.as_str()),
            Some("logs/20")
        );
        assert!(!sessions.iter().any(|session| session.directory == "logs/0"));
    }

    #[test]
    fn activating_a_directory_updates_its_recency() {
        let mut state = WorkspaceViewState::default();
        for directory in ["logs/a", "logs/b"] {
            state.remember_directory_session(PersistedDirectorySearchSession {
                directory: directory.into(),
                ..PersistedDirectorySearchSession::default()
            });
        }

        let restored = state
            .directory_session(std::path::Path::new("logs/a"))
            .expect("directory should exist");

        assert_eq!(restored.directory, "logs/a");
        assert_eq!(state.directory_sessions()[0].directory, "logs/a");
    }
}
