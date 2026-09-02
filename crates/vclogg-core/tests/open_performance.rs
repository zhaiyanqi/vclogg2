use std::{
    fs::{self, File, OpenOptions},
    hint::black_box,
    io::{BufWriter, Write as _},
    path::PathBuf,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use vclogg_core::{DocumentRefreshKind, LogDocument};

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
