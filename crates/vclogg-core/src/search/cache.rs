use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::Mutex,
};

use super::{
    SearchCancellation, SearchMatcher, SearchProgress, SearchQuery, SearchRange, SearchResult,
    SearchRun, search_with_compiled_matcher_inner,
};
use crate::LogDocument;

const MAX_ENTRIES: usize = 16;
const MAX_RESULT_BYTES: usize = 8 * 1024 * 1024;

/// Bounded process-local completed results. No source handles or line indexes are retained.
/// Callers pass the query that produced the compiled matcher, including its result limit.
#[derive(Default)]
pub struct SearchResultCache {
    entries: Mutex<VecDeque<Entry>>,
}

struct Entry {
    path: PathBuf,
    identity: ([u8; 32], u64, String),
    query: SearchQuery,
    range: SearchRange,
    result: SearchResult,
    bytes: usize,
}

impl SearchResultCache {
    /// A scheduling hint only; a hit still requires snapshot validation in `search`.
    pub fn has_candidate(&self, path: &Path, query: &SearchQuery) -> bool {
        self.has_candidate_in_range(path, query, SearchRange::default())
    }

    pub fn has_candidate_in_range(
        &self,
        path: &Path,
        query: &SearchQuery,
        range: SearchRange,
    ) -> bool {
        self.entries.lock().is_ok_and(|entries| {
            entries
                .iter()
                .any(|entry| entry.path == path && entry.query == *query && entry.range == range)
        })
    }

    pub fn search(
        &self,
        document: &LogDocument,
        query: &SearchQuery,
        matcher: Option<&SearchMatcher>,
        cancellation: &SearchCancellation,
    ) -> SearchRun {
        self.search_in_range(
            document,
            query,
            matcher,
            cancellation,
            SearchRange::default(),
        )
    }

    pub fn search_in_range(
        &self,
        document: &LogDocument,
        query: &SearchQuery,
        matcher: Option<&SearchMatcher>,
        cancellation: &SearchCancellation,
        range: SearchRange,
    ) -> SearchRun {
        self.search_inner(document, query, matcher, cancellation, None, range)
    }

    pub fn search_with_progress(
        &self,
        document: &LogDocument,
        query: &SearchQuery,
        matcher: Option<&SearchMatcher>,
        cancellation: &SearchCancellation,
        progress: &SearchProgress,
    ) -> SearchRun {
        self.search_inner(
            document,
            query,
            matcher,
            cancellation,
            Some(progress),
            SearchRange::default(),
        )
    }

    fn search_inner(
        &self,
        document: &LogDocument,
        query: &SearchQuery,
        matcher: Option<&SearchMatcher>,
        cancellation: &SearchCancellation,
        progress: Option<&SearchProgress>,
        range: SearchRange,
    ) -> SearchRun {
        if cancellation.is_cancelled() {
            return SearchRun::Cancelled;
        }
        // Empty syntax has no scan or source-dependent result to reuse.
        if matcher.is_none() {
            return search_with_compiled_matcher_inner(
                document,
                None,
                query.max_results,
                cancellation,
                progress,
                false,
                range,
            );
        }
        let cached = document.search_cache_identity().and_then(|identity| {
            let mut entries = self.entries.lock().ok()?;
            let index = entries.iter().position(|entry| {
                entry.path == document.path()
                    && entry.identity == identity
                    && entry.query == *query
                    && entry.range == range
            })?;
            let entry = entries.remove(index)?;
            let result = entry.result.clone();
            entries.push_back(entry);
            Some(result)
        });
        if let Some(result) = cached {
            return match document.verify_cached_search(cancellation) {
                Some(true) => {
                    if let Some(progress) = progress {
                        progress.update(document.line_count(), result.len());
                    }
                    SearchRun::Completed(result)
                }
                Some(false) => SearchRun::SourceChanged,
                None => SearchRun::Cancelled,
            };
        }
        let run = search_with_compiled_matcher_inner(
            document,
            matcher,
            query.max_results,
            cancellation,
            progress,
            false,
            range,
        );
        if let SearchRun::Completed(result) = &run {
            self.remember_in_range(document, query, result, range);
        }
        run
    }

    /// Remember a completed, verified scan, including results from combined indexing.
    pub fn remember(&self, document: &LogDocument, query: &SearchQuery, result: &SearchResult) {
        self.remember_in_range(document, query, result, SearchRange::default());
    }

    pub fn remember_in_range(
        &self,
        document: &LogDocument,
        query: &SearchQuery,
        result: &SearchResult,
        range: SearchRange,
    ) {
        let Some(identity) = document.search_cache_identity() else {
            return;
        };
        let bytes = result
            .line_indices
            .rows
            .serialized_size()
            .saturating_add(query.text.len());
        if bytes > MAX_RESULT_BYTES {
            return;
        }
        let Ok(mut entries) = self.entries.lock() else {
            return;
        };
        entries.retain(|entry| {
            !(entry.path == document.path() && entry.query == *query && entry.range == range)
        });
        let mut retained = entries.iter().map(|entry| entry.bytes).sum::<usize>();
        while entries.len() >= MAX_ENTRIES || retained.saturating_add(bytes) > MAX_RESULT_BYTES {
            let Some(oldest) = entries.pop_front() else {
                break;
            };
            retained = retained.saturating_sub(oldest.bytes);
        }
        entries.push_back(Entry {
            path: document.path().to_path_buf(),
            identity,
            query: query.clone(),
            range,
            result: result.clone(),
            bytes,
        });
    }
}
