use std::{
    fs::{self, File},
    io::{BufWriter, Write as _},
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use rayon::{ThreadPoolBuilder, prelude::*};
use vclogg_core::{
    LogDocument, SearchCancellation, SearchMatcher, SearchProgress, SearchQuery, SearchRun, search,
    search_with_compiled_matcher, search_with_progress,
};

const LINE_COUNT: usize = 240_000;
const MATCH_INTERVAL: usize = 137;
static TEMPORARY_DIRECTORY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TemporaryDirectory(PathBuf);

impl TemporaryDirectory {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("系统时间应晚于 Unix 纪元")
            .as_nanos();
        let sequence = TEMPORARY_DIRECTORY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "vclogg2-{label}-{}-{nonce}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("应能创建临时搜索测试目录");
        Self(path)
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        _ = fs::remove_dir_all(&self.0);
    }
}

fn large_document() -> (TemporaryDirectory, LogDocument) {
    let temporary = TemporaryDirectory::new("parallel-search");
    let path = temporary.0.join("large.log");
    let mut writer = BufWriter::new(File::create(&path).expect("应能创建搜索测试日志"));
    for row in 0..LINE_COUNT {
        let message = if row % MATCH_INTERVAL == 0 {
            "target-token"
        } else {
            "ordinary-message"
        };
        writeln!(
            writer,
            "2026-08-27 INFO request={row:08} message={message} completed"
        )
        .expect("应能写入搜索测试日志");
    }
    writer.flush().expect("应能刷新搜索测试日志");
    let document = LogDocument::open(&path).expect("应能打开搜索测试日志");
    assert!(document.metadata().file_size > 3 * 4 * 1024 * 1024);
    (temporary, document)
}

fn run_search(document: &LogDocument, max_results: Option<usize>) -> (SearchRun, SearchProgress) {
    let query = SearchQuery {
        text: "target-token".into(),
        case_sensitive: true,
        regex: false,
        max_results,
    };
    let progress = SearchProgress::new(document.line_count());
    let pool = ThreadPoolBuilder::new()
        .num_threads(4)
        .build()
        .expect("应能创建并行搜索测试线程池");
    let run = pool
        .install(|| {
            search_with_progress(document, &query, &SearchCancellation::default(), &progress)
        })
        .expect("并行搜索应成功");
    (run, progress)
}

#[test]
fn parallel_search_returns_every_match_in_source_order() {
    let (_temporary, document) = large_document();

    let (run, progress) = run_search(&document, None);

    let SearchRun::Completed(result) = run else {
        panic!("并行搜索不应取消");
    };
    let expected = (0..LINE_COUNT).step_by(MATCH_INTERVAL).collect::<Vec<_>>();
    assert_eq!(result.line_indices.iter().collect::<Vec<_>>(), expected);
    assert!(!result.truncated);
    assert_eq!(progress.snapshot().scanned_lines, document.line_count());
    assert_eq!(progress.snapshot().matched_lines, result.len());
}

#[test]
fn parallel_search_limit_keeps_the_earliest_matches() {
    const LIMIT: usize = 17;
    let (_temporary, document) = large_document();

    let (run, progress) = run_search(&document, Some(LIMIT));

    let SearchRun::Completed(result) = run else {
        panic!("有结果上限的并行搜索不应取消");
    };
    let expected = (0..LINE_COUNT)
        .step_by(MATCH_INTERVAL)
        .take(LIMIT)
        .collect::<Vec<_>>();
    assert_eq!(result.line_indices.iter().collect::<Vec<_>>(), expected);
    assert!(result.truncated);
    assert_eq!(progress.snapshot().scanned_lines, document.line_count());
    assert_eq!(progress.snapshot().matched_lines, LIMIT);
}

#[test]
fn parallel_search_zero_limit_only_reports_truncation() {
    let (_temporary, document) = large_document();

    let (run, progress) = run_search(&document, Some(0));

    let SearchRun::Completed(result) = run else {
        panic!("零结果上限的并行搜索不应取消");
    };
    assert!(result.is_empty());
    assert!(result.truncated);
    assert_eq!(progress.snapshot().scanned_lines, document.line_count());
    assert_eq!(progress.snapshot().matched_lines, 0);
}

#[test]
fn pre_cancelled_parallel_search_does_not_scan() {
    let (_temporary, document) = large_document();
    let query = SearchQuery {
        text: "target-token".into(),
        case_sensitive: true,
        regex: false,
        max_results: None,
    };
    let progress = SearchProgress::new(document.line_count());
    let cancellation = SearchCancellation::default();
    cancellation.cancel();

    let run = search_with_progress(&document, &query, &cancellation, &progress)
        .expect("取消搜索不应产生错误");

    assert!(matches!(run, SearchRun::Cancelled));
    assert_eq!(progress.snapshot().scanned_lines, 0);
    assert_eq!(progress.snapshot().matched_lines, 0);
}

#[test]
fn source_changes_never_publish_partial_search_results() {
    let temporary = TemporaryDirectory::new("changed-source-search");
    let path = temporary.0.join("changed.log");
    fs::write(&path, b"target-token\nordinary\n").expect("应能写入原始搜索日志");
    let document = LogDocument::open(&path).expect("应能打开原始搜索日志");
    fs::write(&path, b"ordinary-tok\nordinary\n").expect("应能原地改写搜索日志");
    let query = SearchQuery {
        text: "target-token".into(),
        case_sensitive: true,
        regex: false,
        max_results: None,
    };
    let progress = SearchProgress::new(document.line_count());

    let run = search_with_progress(&document, &query, &SearchCancellation::default(), &progress)
        .expect("源文件变化应作为显式搜索结果返回");

    assert!(matches!(run, SearchRun::SourceChanged));
    assert!(search(&document, &query).is_err());
}

#[test]
fn parallel_regex_matches_serial_and_combined_index_search() {
    let (temporary, document) = large_document();
    let serial_pool = ThreadPoolBuilder::new().num_threads(1).build().unwrap();
    let parallel_pool = ThreadPoolBuilder::new().num_threads(4).build().unwrap();
    for (text, case_sensitive) in [
        (r"\bmessage=target-token\s+completed$", true),
        (r"\bREQUEST=\d+\s+MESSAGE=target-token", false),
    ] {
        for max_results in [None, Some(17)] {
            let query = SearchQuery {
                text: text.into(),
                case_sensitive,
                regex: true,
                max_results,
            };
            let matcher = SearchMatcher::new(&query).unwrap();
            let cancellation = SearchCancellation::default();
            let expected = serial_pool.install(|| search(&document, &query)).unwrap();
            let expected_rows = (0..LINE_COUNT)
                .step_by(MATCH_INTERVAL)
                .take(max_results.unwrap_or(usize::MAX))
                .collect::<Vec<_>>();
            assert_eq!(
                expected.line_indices.iter().collect::<Vec<_>>(),
                expected_rows
            );

            // The same compiled matcher is also shared by concurrent file searches.
            let results = parallel_pool.install(|| {
                (0..2)
                    .into_par_iter()
                    .map(|_| {
                        search_with_compiled_matcher(
                            &document,
                            matcher.as_ref(),
                            max_results,
                            &cancellation,
                        )
                    })
                    .collect::<Vec<_>>()
            });
            for result in results {
                let SearchRun::Completed(result) = result else {
                    panic!("search should complete")
                };
                assert_eq!(result.line_indices, expected.line_indices);
                assert_eq!(result.truncated, expected.truncated);
            }

            let (_, pending_cache, result) = parallel_pool
                .install(|| {
                    LogDocument::open_with_index_cache_and_search_cancellable(
                        document.path(),
                        temporary.0.join("regex-cache"),
                        matcher.as_ref().unwrap(),
                        max_results,
                        &cancellation,
                    )
                })
                .unwrap()
                .unwrap();
            assert!(
                pending_cache.is_some(),
                "exercise the combined uncached path"
            );
            let SearchRun::Completed(result) = result else {
                panic!("search should complete")
            };
            assert_eq!(result.line_indices, expected.line_indices);
            assert_eq!(result.truncated, expected.truncated);
        }
    }
}
