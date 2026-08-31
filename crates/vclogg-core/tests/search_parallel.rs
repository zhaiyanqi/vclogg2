use std::{
    fs::{self, File},
    io::{BufWriter, Write as _},
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use rayon::ThreadPoolBuilder;
use vclogg_core::{
    LogDocument, SearchCancellation, SearchProgress, SearchQuery, SearchRun, search_with_progress,
};

const LINE_COUNT: usize = 60_000;
const MATCH_INTERVAL: usize = 137;

struct TemporaryDirectory(PathBuf);

impl TemporaryDirectory {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("系统时间应晚于 Unix 纪元")
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("vclogg2-{label}-{}-{nonce}", std::process::id()));
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
    assert!(document.metadata().file_size > 1024 * 1024);
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
