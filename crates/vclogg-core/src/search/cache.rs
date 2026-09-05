use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::Mutex,
};

use super::{
    SearchCancellation, SearchMatcher, SearchQuery, SearchResult, SearchRun,
    search_with_compiled_matcher,
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
    result: SearchResult,
    bytes: usize,
}

impl SearchResultCache {
    /// A scheduling hint only; a hit still requires snapshot validation in `search`.
    pub fn has_candidate(&self, path: &Path, query: &SearchQuery) -> bool {
        self.entries.lock().is_ok_and(|entries| {
            entries
                .iter()
                .any(|entry| entry.path == path && entry.query == *query)
        })
    }

    pub fn search(
        &self,
        document: &LogDocument,
        query: &SearchQuery,
        matcher: Option<&SearchMatcher>,
        cancellation: &SearchCancellation,
    ) -> SearchRun {
        if cancellation.is_cancelled() {
            return SearchRun::Cancelled;
        }
        let cached = document.search_cache_identity().and_then(|identity| {
            let mut entries = self.entries.lock().ok()?;
            let index = entries.iter().position(|entry| {
                entry.path == document.path() && entry.identity == identity && entry.query == *query
            })?;
            let entry = entries.remove(index)?;
            let result = entry.result.clone();
            entries.push_back(entry);
            Some(result)
        });
        if let Some(result) = cached {
            return match document.verify_cached_search(cancellation) {
                Some(true) => SearchRun::Completed(result),
                Some(false) => SearchRun::SourceChanged,
                None => SearchRun::Cancelled,
            };
        }
        let run = search_with_compiled_matcher(document, matcher, query.max_results, cancellation);
        if let SearchRun::Completed(result) = &run {
            self.remember(document, query, result);
        }
        run
    }

    /// Remember a completed, verified scan, including results from combined indexing.
    pub fn remember(&self, document: &LogDocument, query: &SearchQuery, result: &SearchResult) {
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
        entries.retain(|entry| !(entry.path == document.path() && entry.query == *query));
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
            result: result.clone(),
            bytes,
        });
    }
}
