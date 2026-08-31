use std::{
    ops::Range,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use aho_corasick::{AhoCorasick, AhoCorasickBuilder};
use anyhow::{Context as _, Result};
use rayon::prelude::{IntoParallelIterator as _, ParallelIterator as _};
use regex::bytes::{Regex, RegexBuilder};
use roaring::RoaringTreemap;

use crate::LogDocument;

const PARALLEL_SEARCH_MIN_BYTES: u64 = 1024 * 1024;
const SEARCH_TASKS_PER_THREAD: usize = 4;
const SEARCH_PROGRESS_BATCH_LINES: usize = 1024;

/// Search options owned by the feature layer and passed to the core as one snapshot.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SearchQuery {
    pub text: String,
    pub case_sensitive: bool,
    pub regex: bool,
    /// `None` keeps every match; `Some(n)` stops after `n` rows.
    pub max_results: Option<usize>,
}

/// Ordered source rows backed by a compressed roaring treemap.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CompressedRows {
    rows: Arc<RoaringTreemap>,
}

impl CompressedRows {
    pub fn from_inclusive_ranges(ranges: impl IntoIterator<Item = (usize, usize)>) -> Self {
        let mut rows = RoaringTreemap::new();
        for (start, end) in ranges {
            let (Ok(start), Ok(end)) = (u64::try_from(start), u64::try_from(end)) else {
                continue;
            };
            if start <= end {
                rows.insert_range(start..=end);
            }
        }
        Self {
            rows: Arc::new(rows),
        }
    }

    pub fn len(&self) -> usize {
        usize::try_from(self.rows.len()).unwrap_or(usize::MAX)
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn get(&self, index: usize) -> Option<usize> {
        self.rows
            .select(u64::try_from(index).ok()?)
            .and_then(|row| usize::try_from(row).ok())
    }

    pub fn first(&self) -> Option<usize> {
        self.get(0)
    }

    pub fn contains(&self, row: usize) -> bool {
        u64::try_from(row)
            .ok()
            .is_some_and(|row| self.rows.contains(row))
    }

    pub fn position(&self, row: usize) -> Option<usize> {
        let row = u64::try_from(row).ok()?;
        self.rows
            .contains(row)
            .then(|| self.rows.rank(row).saturating_sub(1))
            .and_then(|rank| usize::try_from(rank).ok())
    }

    pub fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        self.rows.iter().filter_map(|row| usize::try_from(row).ok())
    }

    pub fn union(&self, rows: impl IntoIterator<Item = usize>) -> Self {
        let mut rows = rows.into_iter().filter_map(|row| u64::try_from(row).ok());
        while let Some(row) = rows.next() {
            if self.rows.contains(row) {
                continue;
            }
            let mut merged = self.rows.as_ref().clone();
            merged.insert(row);
            merged.extend(rows);
            return Self {
                rows: Arc::new(merged),
            };
        }
        self.clone()
    }
}

impl FromIterator<usize> for CompressedRows {
    fn from_iter<T: IntoIterator<Item = usize>>(iter: T) -> Self {
        let mut rows = RoaringTreemap::new();
        for row in iter {
            if let Ok(row) = u64::try_from(row) {
                rows.insert(row);
            }
        }
        Self {
            rows: Arc::new(rows),
        }
    }
}

/// Source row coordinates returned by a completed search.
#[derive(Clone, Debug, Default)]
pub struct SearchResult {
    pub line_indices: CompressedRows,
    pub truncated: bool,
}

impl SearchResult {
    pub fn len(&self) -> usize {
        self.line_indices.len()
    }

    pub fn is_empty(&self) -> bool {
        self.line_indices.is_empty()
    }
}

/// Cooperative cancellation shared between the UI task and the blocking scan.
#[derive(Clone, Debug, Default)]
pub struct SearchCancellation {
    cancelled: Arc<AtomicBool>,
}

impl SearchCancellation {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

/// Outcome of a cancellable scan. Cancellation is expected control flow, not an error.
#[derive(Clone, Debug)]
pub enum SearchRun {
    Completed(SearchResult),
    Cancelled,
}

/// A cheap, read-only presentation snapshot for an in-flight search.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub struct SearchProgressSnapshot {
    pub scanned_lines: usize,
    pub total_lines: usize,
    pub matched_lines: usize,
}

impl SearchProgressSnapshot {
    pub fn percent(self) -> f32 {
        if self.total_lines == 0 {
            100.
        } else {
            (self.scanned_lines as f32 / self.total_lines as f32 * 100.).clamp(0., 100.)
        }
    }
}

/// Shared progress counters written by the blocking scanner and sampled by the UI.
#[derive(Clone, Debug)]
pub struct SearchProgress {
    counters: Arc<SearchProgressCounters>,
}

#[derive(Debug)]
struct SearchProgressCounters {
    scanned_lines: AtomicUsize,
    matched_lines: AtomicUsize,
    total_lines: usize,
}

impl SearchProgress {
    pub fn new(total_lines: usize) -> Self {
        Self {
            counters: Arc::new(SearchProgressCounters {
                scanned_lines: AtomicUsize::new(0),
                matched_lines: AtomicUsize::new(0),
                total_lines,
            }),
        }
    }

    pub fn snapshot(&self) -> SearchProgressSnapshot {
        let matched_lines = self.counters.matched_lines.load(Ordering::Relaxed);
        let scanned_lines = self
            .counters
            .scanned_lines
            .load(Ordering::Relaxed)
            .max(matched_lines)
            .min(self.counters.total_lines);
        SearchProgressSnapshot {
            scanned_lines,
            total_lines: self.counters.total_lines,
            matched_lines,
        }
    }

    fn update(&self, scanned_lines: usize, matched_lines: usize) {
        self.counters.scanned_lines.store(
            scanned_lines.min(self.counters.total_lines),
            Ordering::Relaxed,
        );
        self.counters
            .matched_lines
            .store(matched_lines, Ordering::Relaxed);
    }

    fn advance(&self, scanned_lines: usize, matched_lines: usize, max_results: Option<usize>) {
        _ = self.counters.scanned_lines.fetch_update(
            Ordering::Relaxed,
            Ordering::Relaxed,
            |current| Some(current.saturating_add(scanned_lines)),
        );
        _ = self.counters.matched_lines.fetch_update(
            Ordering::Relaxed,
            Ordering::Relaxed,
            |current| {
                let next = current.saturating_add(matched_lines);
                Some(max_results.map_or(next, |limit| next.min(limit)))
            },
        );
    }
}

/// Scan a document without materializing its text or matched rows.
pub fn search(document: &LogDocument, query: &SearchQuery) -> Result<SearchResult> {
    match search_cancellable(document, query, &SearchCancellation::default())? {
        SearchRun::Completed(result) => Ok(result),
        SearchRun::Cancelled => Ok(SearchResult::default()),
    }
}

/// Scan a document while periodically observing a cooperative cancellation token.
pub fn search_cancellable(
    document: &LogDocument,
    query: &SearchQuery,
    cancellation: &SearchCancellation,
) -> Result<SearchRun> {
    let progress = SearchProgress::new(document.line_count());
    search_with_progress(document, query, cancellation, &progress)
}

/// Scan a document while publishing bounded progress snapshots for presentation.
pub fn search_with_progress(
    document: &LogDocument,
    query: &SearchQuery,
    cancellation: &SearchCancellation,
    progress: &SearchProgress,
) -> Result<SearchRun> {
    if cancellation.is_cancelled() {
        return Ok(SearchRun::Cancelled);
    }
    let matcher = SearchMatcher::new(query)?;
    Ok(search_with_compiled_matcher(
        document,
        matcher.as_ref(),
        query.max_results,
        cancellation,
        progress,
    ))
}

/// Scan with an already compiled matcher so one query can be shared across
/// several documents without repeating regex or automaton construction.
pub fn search_with_compiled_matcher(
    document: &LogDocument,
    matcher: Option<&SearchMatcher>,
    max_results: Option<usize>,
    cancellation: &SearchCancellation,
    progress: &SearchProgress,
) -> SearchRun {
    if cancellation.is_cancelled() {
        return SearchRun::Cancelled;
    }
    let Some(matcher) = matcher else {
        progress.update(document.line_count(), 0);
        return SearchRun::Completed(SearchResult::default());
    };

    progress.update(0, 0);
    let line_count = document.line_count();
    let chunk_count = search_chunk_count(document);
    let chunk_size = line_count.div_ceil(chunk_count);
    let chunks = if chunk_count == 1 {
        vec![scan_search_chunk(
            document,
            matcher,
            0..line_count,
            max_results,
            cancellation,
            progress,
        )]
    } else {
        (0..chunk_count)
            .into_par_iter()
            .map(|chunk_ix| {
                let start = chunk_ix.saturating_mul(chunk_size).min(line_count);
                let end = start.saturating_add(chunk_size).min(line_count);
                scan_search_chunk(
                    document,
                    matcher,
                    start..end,
                    max_results,
                    cancellation,
                    progress,
                )
            })
            .collect::<Vec<_>>()
    };

    if chunks
        .iter()
        .any(|chunk| matches!(chunk, SearchChunkRun::Cancelled))
    {
        return SearchRun::Cancelled;
    }

    let mut line_indices = RoaringTreemap::new();
    let mut truncated = false;
    for chunk in chunks {
        let SearchChunkRun::Completed(chunk) = chunk else {
            unreachable!("cancelled chunks are handled before result merging");
        };
        match max_results {
            None => {
                line_indices |= chunk.line_indices;
                truncated |= chunk.truncated;
            }
            Some(limit) => {
                let current_len = usize::try_from(line_indices.len()).unwrap_or(usize::MAX);
                let remaining = limit.saturating_sub(current_len);
                let chunk_len = usize::try_from(chunk.line_indices.len()).unwrap_or(usize::MAX);
                line_indices.extend(chunk.line_indices.iter().take(remaining));
                truncated |= chunk.truncated || chunk_len > remaining;
            }
        }
    }

    progress.update(
        line_count,
        usize::try_from(line_indices.len()).unwrap_or(usize::MAX),
    );

    SearchRun::Completed(SearchResult {
        line_indices: CompressedRows {
            rows: Arc::new(line_indices),
        },
        truncated,
    })
}

fn search_chunk_count(document: &LogDocument) -> usize {
    let line_count = document.line_count();
    if line_count < 2 || document.metadata().file_size < PARALLEL_SEARCH_MIN_BYTES {
        return 1;
    }
    let thread_count = rayon::current_num_threads().max(1);
    if thread_count == 1 || line_count < thread_count.saturating_mul(2) {
        return 1;
    }

    thread_count
        .saturating_mul(SEARCH_TASKS_PER_THREAD)
        .min(line_count)
        .max(1)
}

struct SearchChunkResult {
    line_indices: RoaringTreemap,
    truncated: bool,
}

enum SearchChunkRun {
    Completed(SearchChunkResult),
    Cancelled,
}

fn scan_search_chunk(
    document: &LogDocument,
    matcher: &SearchMatcher,
    rows: Range<usize>,
    max_results: Option<usize>,
    cancellation: &SearchCancellation,
    progress: &SearchProgress,
) -> SearchChunkRun {
    let mut line_indices = RoaringTreemap::new();
    let mut truncated = false;
    let max_results_u64 = max_results.map(|limit| u64::try_from(limit).unwrap_or(u64::MAX));
    let mut pending_scanned_lines = 0;
    let mut pending_matched_lines = 0;

    for row_ix in rows {
        if pending_scanned_lines == SEARCH_PROGRESS_BATCH_LINES {
            progress.advance(pending_scanned_lines, pending_matched_lines, max_results);
            pending_scanned_lines = 0;
            pending_matched_lines = 0;
            if cancellation.is_cancelled() {
                return SearchChunkRun::Cancelled;
            }
        }
        pending_scanned_lines += 1;

        let Some(line) = document.search_bytes_at_local_row(row_ix) else {
            continue;
        };
        if !matcher.is_match(&line) {
            continue;
        }
        if max_results_u64.is_some_and(|limit| line_indices.len() >= limit) {
            truncated = true;
            break;
        }
        let Some(source_row) = document.source_row(row_ix) else {
            continue;
        };
        if let Ok(source_row) = u64::try_from(source_row) {
            line_indices.insert(source_row);
            pending_matched_lines += 1;
        }
    }

    progress.advance(pending_scanned_lines, pending_matched_lines, max_results);
    if cancellation.is_cancelled() {
        SearchChunkRun::Cancelled
    } else {
        SearchChunkRun::Completed(SearchChunkResult {
            line_indices,
            truncated,
        })
    }
}

#[derive(Clone)]
pub struct SearchMatcher {
    inner: Matcher,
    whole_word: bool,
}

#[derive(Clone)]
enum Matcher {
    Literal(AhoCorasick),
    Regex(Regex),
}

impl SearchMatcher {
    /// Compile the same matcher used by the scanner for visible-line highlighting.
    pub fn new(query: &SearchQuery) -> Result<Option<Self>> {
        let text = query.text.as_str();
        if text.is_empty() {
            return Ok(None);
        }

        if query.regex {
            let regex = RegexBuilder::new(text)
                .case_insensitive(!query.case_sensitive)
                .build()
                .with_context(|| format!("无效的正则表达式：{text}"))?;
            return Ok(Some(Self {
                inner: Matcher::Regex(regex),
                whole_word: false,
            }));
        }

        let patterns = text
            .split('|')
            .filter(|pattern| !pattern.is_empty())
            .collect::<Vec<_>>();
        if patterns.is_empty() {
            return Ok(None);
        }

        let matcher = AhoCorasickBuilder::new()
            .ascii_case_insensitive(!query.case_sensitive)
            .build(patterns)
            .context("无法创建文本搜索器")?;
        Ok(Some(Self {
            inner: Matcher::Literal(matcher),
            whole_word: false,
        }))
    }

    /// Compile one literal, case-insensitive phrase for in-view quick find.
    /// Unlike the main literal search, `|` and every regex metacharacter remain literal.
    pub fn literal_phrase(text: &str) -> Result<Option<Self>> {
        if text.is_empty() {
            return Ok(None);
        }
        let matcher = RegexBuilder::new(&regex::escape(text))
            .case_insensitive(true)
            .build()
            .context("无法创建页内查找器")?;
        Ok(Some(Self {
            inner: Matcher::Regex(matcher),
            whole_word: false,
        }))
    }

    /// Compile one phrase for in-view quick find with independently selectable options.
    /// Literal mode keeps every metacharacter, including `|`, as ordinary text.
    pub fn quick_find(
        text: &str,
        case_sensitive: bool,
        whole_word: bool,
        regex: bool,
    ) -> Result<Option<Self>> {
        if text.is_empty() {
            return Ok(None);
        }
        let pattern = if regex {
            text.to_string()
        } else {
            regex::escape(text)
        };
        let matcher = RegexBuilder::new(&pattern)
            .case_insensitive(!case_sensitive)
            .build()
            .with_context(|| format!("无效的正则表达式：{text}"))?;
        Ok(Some(Self {
            inner: Matcher::Regex(matcher),
            whole_word,
        }))
    }

    /// Compile one literal phrase for persistent color-label rules.
    /// Unlike the main query syntax, `|` and regex metacharacters remain literal.
    pub fn literal(text: &str, case_sensitive: bool) -> Result<Option<Self>> {
        if text.is_empty() {
            return Ok(None);
        }
        let matcher = RegexBuilder::new(&regex::escape(text))
            .case_insensitive(!case_sensitive)
            .build()
            .context("无法创建颜色标签匹配器")?;
        Ok(Some(Self {
            inner: Matcher::Regex(matcher),
            whole_word: false,
        }))
    }

    fn is_match(&self, line: &[u8]) -> bool {
        if self.whole_word {
            return std::str::from_utf8(line)
                .is_ok_and(|text| !self.matching_ranges(text).is_empty());
        }
        match &self.inner {
            Matcher::Literal(matcher) => matcher.is_match(line),
            Matcher::Regex(matcher) => matcher.is_match(line),
        }
    }

    /// Return UTF-8 byte ranges for every non-empty match in rendered text.
    pub fn matching_ranges(&self, text: &str) -> Vec<Range<usize>> {
        let mut ranges: Vec<Range<usize>> = match &self.inner {
            Matcher::Literal(matcher) => matcher
                .find_iter(text.as_bytes())
                .filter(|matched| {
                    text.is_char_boundary(matched.start()) && text.is_char_boundary(matched.end())
                })
                .map(|matched| matched.start()..matched.end())
                .collect(),
            Matcher::Regex(matcher) => matcher
                .find_iter(text.as_bytes())
                .filter(|matched| {
                    matched.start() < matched.end()
                        && text.is_char_boundary(matched.start())
                        && text.is_char_boundary(matched.end())
                })
                .map(|matched| matched.start()..matched.end())
                .collect(),
        };
        if self.whole_word {
            ranges.retain(|range| is_whole_word_match(text, range));
        }
        ranges
    }
}

fn is_whole_word_match(text: &str, range: &Range<usize>) -> bool {
    let is_word_character = |character: char| character.is_alphanumeric() || character == '_';
    let before_is_word = text[..range.start]
        .chars()
        .next_back()
        .is_some_and(is_word_character);
    let after_is_word = text[range.end..]
        .chars()
        .next()
        .is_some_and(is_word_character);
    !before_is_word && !after_is_word
}

#[cfg(test)]
mod performance_tests {
    use std::{hint::black_box, sync::Arc, time::Instant};

    use super::{CompressedRows, SearchMatcher, SearchProgress, SearchQuery};

    #[test]
    fn union_reuses_storage_when_no_rows_are_added() {
        let rows = [2, 5, 9].into_iter().collect::<CompressedRows>();

        let empty_union = rows.union(std::iter::empty());
        let contained_union = rows.union([5, 2]);

        assert!(Arc::ptr_eq(&rows.rows, &empty_union.rows));
        assert!(Arc::ptr_eq(&rows.rows, &contained_union.rows));
    }

    #[test]
    fn union_clones_only_when_a_new_row_is_added() {
        let rows = [2, 5, 9].into_iter().collect::<CompressedRows>();

        let union = rows.union([5, 7, 11]);

        assert!(!Arc::ptr_eq(&rows.rows, &union.rows));
        assert_eq!(union.iter().collect::<Vec<_>>(), [2, 5, 7, 9, 11]);
    }

    #[test]
    fn progress_snapshot_is_bounded_by_total_lines() {
        let progress = SearchProgress::new(10);

        progress.update(20, 3);

        assert_eq!(progress.snapshot().scanned_lines, 10);
        assert_eq!(progress.snapshot().matched_lines, 3);
    }

    #[test]
    #[ignore = "手动性能基准：cargo test -p vclogg-core --release benchmark_matcher_reuse -- --ignored --nocapture"]
    fn benchmark_matcher_reuse() {
        const DOCUMENT_COUNT: usize = 64;
        let query = SearchQuery {
            text: (0..200)
                .map(|index| format!("service-{index:03}-[a-z]{{8}}"))
                .collect::<Vec<_>>()
                .join("|"),
            case_sensitive: false,
            regex: true,
            max_results: None,
        };

        let repeated_started = Instant::now();
        for _ in 0..DOCUMENT_COUNT {
            black_box(SearchMatcher::new(black_box(&query)).expect("正则应有效"));
        }
        let repeated = repeated_started.elapsed();

        let shared_started = Instant::now();
        let matcher = SearchMatcher::new(black_box(&query)).expect("正则应有效");
        for _ in 0..DOCUMENT_COUNT {
            black_box(matcher.clone());
        }
        let shared = shared_started.elapsed();

        eprintln!("{DOCUMENT_COUNT} 个文档重复编译：{repeated:?}；一次编译后共享：{shared:?}");
    }

    #[test]
    #[ignore = "手动性能基准：cargo test -p vclogg-core --release benchmark_noop_union -- --ignored --nocapture"]
    fn benchmark_noop_union() {
        let rows = (0..5_000_000_usize).step_by(3).collect::<CompressedRows>();
        let started = Instant::now();
        let mut observed = 0;
        for _ in 0..100 {
            observed += black_box(rows.union(std::iter::empty())).len();
        }
        let elapsed = started.elapsed();

        assert_eq!(observed, rows.len() * 100);
        eprintln!("空集合合并 100 次：{elapsed:?}，结果行数：{}", rows.len());
    }
}
