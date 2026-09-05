//! Bounded, provisional feedback; completed result projections have separate owners.
use std::{path::PathBuf, sync::Mutex};
use vclogg_core::{LogDocument, SearchProgress, SearchResult};

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct SearchFeedbackSnapshot {
    pub(crate) percent: Option<u8>,
    pub(crate) matches: usize,
    pub(crate) previews: Vec<String>,
}

#[derive(Default)]
pub(crate) struct SearchFeedback {
    documents: Vec<(PathBuf, SearchProgress)>,
    directory: Mutex<DirectoryFeedback>,
}

#[derive(Default)]
struct DirectoryFeedback {
    total: Option<usize>,
    completed: usize,
    matches: usize,
    previews: Vec<String>,
}

impl SearchFeedback {
    pub(crate) fn for_documents(documents: impl IntoIterator<Item = (PathBuf, usize)>) -> Self {
        Self {
            documents: documents
                .into_iter()
                .map(|(path, lines)| (path, SearchProgress::new(lines)))
                .collect(),
            ..Self::default()
        }
    }

    pub(crate) fn progress(&self, index: usize) -> &SearchProgress {
        &self.documents[index].1
    }

    pub(crate) fn set_file_total(&self, total: usize) {
        if let Ok(mut state) = self.directory.lock() {
            state.total = Some(total);
        }
    }

    pub(crate) fn file_finished(&self, result: Option<(&LogDocument, &SearchResult)>) {
        let needs_preview = self
            .directory
            .lock()
            .is_ok_and(|state| state.previews.len() < 3);
        let previews = if needs_preview {
            result
                .map(|(document, result)| {
                    result
                        .line_indices
                        .iter()
                        .take(3)
                        .filter_map(|row| {
                            document.line_preview(row, 512).map(|line| {
                                format!(
                                    "{}:{}  {}",
                                    document.file_name(),
                                    row + 1,
                                    line.text().chars().take(160).collect::<String>()
                                )
                            })
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        if let Ok(mut state) = self.directory.lock() {
            state.completed += 1;
            state.matches = state
                .matches
                .saturating_add(result.map_or(0, |(_, result)| result.len()));
            let remaining = 3_usize.saturating_sub(state.previews.len());
            state.previews.extend(previews.into_iter().take(remaining));
        }
    }

    pub(crate) fn snapshot(&self) -> SearchFeedbackSnapshot {
        if self.documents.is_empty() {
            let Ok(state) = self.directory.lock() else {
                return SearchFeedbackSnapshot::default();
            };
            return SearchFeedbackSnapshot {
                percent: state.total.map(|total| percent(state.completed, total)),
                matches: state.matches,
                previews: state.previews.clone(),
            };
        }
        let (mut scanned, mut total, mut matches) = (0_usize, 0_usize, 0_usize);
        let mut previews = Vec::new();
        for (path, progress) in &self.documents {
            let snapshot = progress.snapshot();
            scanned = scanned.saturating_add(snapshot.scanned_lines);
            total = total.saturating_add(snapshot.total_lines);
            matches = matches.saturating_add(snapshot.matched_lines);
            if previews.len() < 3 {
                previews.extend(
                    progress
                        .previews()
                        .into_iter()
                        .take(3 - previews.len())
                        .map(|row| {
                            format!(
                                "{}:{}  {}",
                                path.file_name().unwrap_or_default().to_string_lossy(),
                                row.source_row + 1,
                                row.text
                            )
                        }),
                );
            }
        }
        SearchFeedbackSnapshot {
            percent: Some(percent(scanned, total)),
            matches,
            previews,
        }
    }
}

fn percent(done: usize, total: usize) -> u8 {
    if total == 0 {
        100
    } else {
        ((done as u128 * 100) / total as u128).min(100) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn directory_progress_is_bounded_and_starts_indeterminate() {
        let feedback = SearchFeedback::default();
        assert_eq!(feedback.snapshot().percent, None);
        feedback.set_file_total(2);
        feedback.file_finished(None);
        assert_eq!(feedback.snapshot().percent, Some(50));
        feedback.file_finished(None);
        feedback.file_finished(None);
        assert_eq!(feedback.snapshot().percent, Some(100));
    }
}
