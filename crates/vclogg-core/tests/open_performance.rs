use std::{
    fs::{self, File, OpenOptions},
    hint::black_box,
    io::{BufWriter, Write as _},
    path::PathBuf,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use vclogg_core::{
    DocumentRefreshKind, LogDocument, SearchCancellation, SearchMatcher, SearchQuery, SearchRun,
    search_with_compiled_matcher,
};

struct TemporaryDirectory(PathBuf);

impl TemporaryDirectory {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("系统时间应晚于 Unix 纪元")
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("vclogg2-{label}-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&path).expect("应能创建临时性能测试目录");
        Self(path)
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn bounded_preview_keeps_utf8_line_contents() {
    let temporary = TemporaryDirectory::new("open-preview-correctness");
    let path = temporary.0.join("utf8.log");
    fs::write(&path, b"first\nsecond\nthird\n").expect("应能写入 UTF-8 测试日志");

    let (document, complete) =
        LogDocument::open_preview(&path, 1024, 2).expect("应能打开 UTF-8 日志预览");

    assert!(!complete);
    assert_eq!(document.line_count(), 2);
    assert_eq!(document.line(0).as_deref(), Some("first"));
    assert_eq!(document.line(1).as_deref(), Some("second"));
}

#[test]
fn parallel_full_open_preserves_append_verification() {
    const SOURCE_BYTES: usize = 9 * 1024 * 1024;

    let temporary = TemporaryDirectory::new("parallel-open-correctness");
    let path = temporary.0.join("large.log");
    let line = b"2026-08-27 INFO original line\n";
    let mut writer = BufWriter::new(File::create(&path).expect("应能创建并发打开测试日志"));
    for _ in 0..SOURCE_BYTES.div_ceil(line.len()) {
        writer.write_all(line).expect("应能写入并发打开测试日志");
    }
    writer.flush().expect("应能刷新并发打开测试日志");

    let document = LogDocument::open(&path).expect("大文件并发索引应成功");
    let previous_lines = document.line_count();
    let mut appender = OpenOptions::new()
        .append(true)
        .open(&path)
        .expect("应能追加测试日志");
    appender
        .write_all(b"2026-08-27 INFO appended line\n")
        .expect("应能追加测试行");
    appender.flush().expect("应能刷新追加测试行");

    let (refreshed, kind) = document.refresh().expect("追加刷新应成功");

    assert_eq!(kind, DocumentRefreshKind::Appended);
    assert!(refreshed.line_count() > previous_lines);
}

#[test]
fn combined_uncached_open_and_search_matches_verified_second_pass_across_blocks() {
    const BLOCK_BYTES: usize = 4 * 1024 * 1024;
    let temporary = TemporaryDirectory::new("combined-open-search-correctness");
    let path = temporary.0.join("large.log");
    let cache = temporary.0.join("index");
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.resize(BLOCK_BYTES - 3, b'a');
    bytes.extend_from_slice(b"needle\n");
    bytes.resize(BLOCK_BYTES * 2 - 1, b'b');
    bytes.extend_from_slice(b"\r\nneedle\n");
    bytes.resize(BLOCK_BYTES * 2 + 1024, b'c');
    fs::write(&path, bytes).expect("应能写入跨块搜索测试日志");

    let query = SearchQuery {
        text: "needle".to_owned(),
        ..SearchQuery::default()
    };
    let matcher = SearchMatcher::new(&query)
        .expect("搜索器应能编译")
        .expect("非空查询应有搜索器");
    let cancellation = SearchCancellation::default();
    let (combined_document, pending, combined_run) =
        LogDocument::open_with_index_cache_and_search_cancellable(
            &path,
            &cache,
            &matcher,
            None,
            &cancellation,
        )
        .expect("合并打开与搜索应成功")
        .expect("搜索不应被取消");
    pending
        .expect("首次合并打开应产生索引缓存")
        .persist()
        .expect("合并打开的索引缓存应能持久化");
    let SearchRun::Completed(combined) = combined_run else {
        panic!("合并搜索应完成");
    };
    let verified =
        search_with_compiled_matcher(&combined_document, Some(&matcher), None, &cancellation);
    let SearchRun::Completed(verified) = verified else {
        panic!("验证搜索应完成");
    };

    assert_eq!(
        combined.line_indices.iter().collect::<Vec<_>>(),
        verified.line_indices.iter().collect::<Vec<_>>()
    );
    assert_eq!(combined.line_indices.iter().collect::<Vec<_>>(), [0, 2]);
    assert_eq!(combined.truncated, verified.truncated);

    let (_, pending, cached_run) = LogDocument::open_with_index_cache_and_search_cancellable(
        &path,
        &cache,
        &matcher,
        None,
        &cancellation,
    )
    .expect("缓存命中打开与搜索应成功")
    .expect("缓存命中搜索不应被取消");
    assert!(pending.is_none());
    let SearchRun::Completed(cached) = cached_run else {
        panic!("缓存命中搜索应完成");
    };
    assert_eq!(
        cached.line_indices.iter().collect::<Vec<_>>(),
        combined.line_indices.iter().collect::<Vec<_>>()
    );
}

#[test]
#[ignore = "手动性能基准：cargo test -p vclogg-core --release benchmark_open_preview -- --ignored --nocapture"]
fn benchmark_open_preview() {
    const SOURCE_BYTES: usize = 64 * 1024 * 1024;
    const RUNS: usize = 100;
    const PREVIEW_BYTES: usize = 1024 * 1024;
    const PREVIEW_LINES: usize = 200;

    let temporary = TemporaryDirectory::new("open-preview-performance");
    let path = temporary.0.join("large.log");
    let line = b"2026-08-27 INFO request completed\n";
    let mut writer = BufWriter::new(File::create(&path).expect("应能创建性能测试日志"));
    for _ in 0..SOURCE_BYTES.div_ceil(line.len()) {
        writer.write_all(line).expect("应能写入性能测试日志");
    }
    writer.flush().expect("应能刷新性能测试日志");

    let started = Instant::now();
    for _ in 0..RUNS {
        let (document, complete) = LogDocument::open_preview(
            black_box(&path),
            black_box(PREVIEW_BYTES),
            black_box(PREVIEW_LINES),
        )
        .expect("应能打开日志预览");
        assert!(!complete);
        assert_eq!(document.line_count(), PREVIEW_LINES);
        black_box(document);
    }
    let elapsed = started.elapsed();

    eprintln!(
        "打开 {RUNS} 次 1 MiB/200 行预览：{elapsed:?}，平均：{:?}",
        elapsed / RUNS as u32
    );
}

#[test]
#[cfg(windows)]
#[ignore = "手动性能基准：cargo test -p vclogg-core --release benchmark_cached_full_open -- --ignored --nocapture"]
fn benchmark_cached_full_open() {
    const SOURCE_BYTES: usize = 128 * 1024 * 1024;
    const RUNS: usize = 5;

    let temporary = TemporaryDirectory::new("cached-open-performance");
    let path = temporary.0.join("large.log");
    let cache = temporary.0.join("index");
    let line = b"2026-08-27 INFO cached file open performance line\n";
    let mut writer = BufWriter::new(File::create(&path).expect("应能创建性能测试日志"));
    for _ in 0..SOURCE_BYTES.div_ceil(line.len()) {
        writer.write_all(line).expect("应能写入性能测试日志");
    }
    writer.flush().expect("应能刷新性能测试日志");

    let (document, pending) =
        LogDocument::open_with_index_cache(&path, &cache).expect("首次打开应成功");
    pending
        .expect("首次打开应生成索引缓存")
        .persist()
        .expect("索引缓存应能持久化");
    drop(document);

    let started = Instant::now();
    for _ in 0..RUNS {
        let (document, pending) =
            LogDocument::open_with_index_cache(black_box(&path), black_box(&cache))
                .expect("缓存命中打开应成功");
        assert!(pending.is_none());
        black_box(document);
    }
    let elapsed = started.elapsed();

    eprintln!(
        "缓存命中打开 128 MiB 文件 {RUNS} 次：{elapsed:?}，平均：{:?}",
        elapsed / RUNS as u32
    );
}

#[test]
#[ignore = "手动性能基准：cargo test -p vclogg-core --release benchmark_uncached_full_open -- --ignored --nocapture"]
fn benchmark_uncached_full_open() {
    const SOURCE_BYTES: usize = 128 * 1024 * 1024;
    const RUNS: usize = 5;

    let temporary = TemporaryDirectory::new("uncached-open-performance");
    let path = temporary.0.join("large.log");
    let cache = temporary.0.join("index");
    let line = b"2026-08-27 INFO uncached file open performance line\n";
    let mut writer = BufWriter::new(File::create(&path).expect("应能创建性能测试日志"));
    for _ in 0..SOURCE_BYTES.div_ceil(line.len()) {
        writer.write_all(line).expect("应能写入性能测试日志");
    }
    writer.flush().expect("应能刷新性能测试日志");

    let started = Instant::now();
    for _ in 0..RUNS {
        let (document, pending) =
            LogDocument::open_with_index_cache(black_box(&path), black_box(&cache))
                .expect("无缓存完整打开应成功");
        assert!(pending.is_some());
        black_box((document, pending));
    }
    let elapsed = started.elapsed();

    eprintln!(
        "无缓存打开 128 MiB 文件 {RUNS} 次：{elapsed:?}，平均：{:?}",
        elapsed / RUNS as u32
    );
}

#[test]
#[ignore = "手动性能基准：cargo test -p vclogg-core --release benchmark_combined_uncached_open_search -- --ignored --nocapture"]
fn benchmark_combined_uncached_open_search() {
    const SOURCE_BYTES: usize = 128 * 1024 * 1024;
    const RUNS: usize = 5;
    let temporary = TemporaryDirectory::new("combined-open-search-performance");
    let path = temporary.0.join("large.log");
    let cache = temporary.0.join("index");
    let line = b"2026-08-27 INFO combined uncached open search needle line\n";
    let mut writer = BufWriter::new(File::create(&path).expect("应能创建性能测试日志"));
    for _ in 0..SOURCE_BYTES.div_ceil(line.len()) {
        writer.write_all(line).expect("应能写入性能测试日志");
    }
    writer.flush().expect("应能刷新性能测试日志");
    let query = SearchQuery {
        text: "needle".to_owned(),
        ..SearchQuery::default()
    };
    let matcher = SearchMatcher::new(&query)
        .expect("搜索器应能编译")
        .expect("非空查询应有搜索器");
    let cancellation = SearchCancellation::default();

    let separate_started = Instant::now();
    for _ in 0..RUNS {
        let (document, pending) =
            LogDocument::open_with_index_cache(&path, &cache).expect("分离打开应成功");
        assert!(pending.is_some());
        let run = search_with_compiled_matcher(&document, Some(&matcher), None, &cancellation);
        assert!(matches!(run, SearchRun::Completed(_)));
        black_box((document, run));
    }
    let separate_elapsed = separate_started.elapsed();

    let combined_started = Instant::now();
    for _ in 0..RUNS {
        let opened = LogDocument::open_with_index_cache_and_search_cancellable(
            &path,
            &cache,
            &matcher,
            None,
            &cancellation,
        )
        .expect("合并打开应成功")
        .expect("合并打开不应取消");
        assert!(opened.1.is_some());
        assert!(matches!(opened.2, SearchRun::Completed(_)));
        black_box(opened);
    }
    let combined_elapsed = combined_started.elapsed();

    eprintln!(
        "128 MiB 无缓存打开+搜索 {RUNS} 次：分离 {separate_elapsed:?}，平均 {:?}；合并 {combined_elapsed:?}，平均 {:?}",
        separate_elapsed / RUNS as u32,
        combined_elapsed / RUNS as u32,
    );
}

#[test]
#[ignore = "手动性能基准：cargo test -p vclogg-core --release benchmark_cached_preview_handoff -- --ignored --nocapture"]
fn benchmark_cached_preview_handoff() {
    const SOURCE_BYTES: usize = 128 * 1024 * 1024;
    const RUNS: usize = 5;
    const PREVIEW_BYTES: usize = 1024 * 1024;
    const PREVIEW_LINES: usize = 200;

    let temporary = TemporaryDirectory::new("cached-preview-handoff-performance");
    let path = temporary.0.join("large.log");
    let cache = temporary.0.join("index");
    let line = b"2026-08-27 INFO cached preview handoff performance line\n";
    let mut writer = BufWriter::new(File::create(&path).expect("应能创建性能测试日志"));
    for _ in 0..SOURCE_BYTES.div_ceil(line.len()) {
        writer.write_all(line).expect("应能写入性能测试日志");
    }
    writer.flush().expect("应能刷新性能测试日志");
    let (document, pending) =
        LogDocument::open_with_index_cache(&path, &cache).expect("首次缓存交接性能测试打开应成功");
    let preferred_row = document.line_count() / 2;
    pending
        .expect("首次打开应生成索引缓存")
        .persist()
        .expect("索引缓存应能持久化");
    drop(document);

    let separate_started = Instant::now();
    for _ in 0..RUNS {
        let preview = LogDocument::open_cached_preview(
            black_box(&path),
            black_box(&cache),
            black_box(preferred_row),
            PREVIEW_LINES,
            PREVIEW_BYTES,
        )
        .expect("独立缓存预览应成功")
        .expect("独立缓存预览应命中");
        let (complete, pending) =
            LogDocument::open_with_index_cache(black_box(&path), black_box(&cache))
                .expect("独立完整缓存打开应成功");
        assert!(pending.is_none());
        black_box((preview, complete));
    }
    let separate_elapsed = separate_started.elapsed();

    let handoff_started = Instant::now();
    for _ in 0..RUNS {
        let pair = LogDocument::open_cached_preview_with_complete_document(
            black_box(&path),
            black_box(&cache),
            black_box(preferred_row),
            PREVIEW_LINES,
            PREVIEW_BYTES,
        )
        .expect("缓存预览交接应成功")
        .expect("缓存预览交接应命中");
        black_box(pair);
    }
    let handoff_elapsed = handoff_started.elapsed();

    eprintln!(
        "缓存预览后完整打开 {RUNS} 次：重复解析 {separate_elapsed:?}；单次解析交接 {handoff_elapsed:?}"
    );
}
