use std::{
    fs::{self, File},
    hint::black_box,
    io::{BufWriter, Write as _},
    path::PathBuf,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use vclogg_core::{
    LogDocument, SearchCancellation, SearchProgress, SearchQuery, SearchRun, search_with_progress,
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
#[ignore = "手动性能基准：cargo test -p vclogg-core --release benchmark_literal_search -- --ignored --nocapture"]
fn benchmark_literal_search() {
    const LINE_COUNT: usize = 1_000_000;
    const RUNS: usize = 5;

    let temporary = TemporaryDirectory::new("search-performance");
    let path = temporary.0.join("large.log");
    let mut writer = BufWriter::new(File::create(&path).expect("应能创建性能测试日志"));
    for row in 0..LINE_COUNT {
        if row % 97 == 0 {
            writeln!(
                writer,
                "2026-08-26 INFO request={row} target-token completed"
            )
            .expect("应能写入性能测试日志");
        } else {
            writeln!(
                writer,
                "2026-08-26 INFO request={row} ordinary message completed"
            )
            .expect("应能写入性能测试日志");
        }
    }
    writer.flush().expect("应能刷新性能测试日志");

    let document = LogDocument::open(&path).expect("应能打开性能测试日志");
    let query = SearchQuery {
        text: "target-token".into(),
        case_sensitive: true,
        regex: false,
        max_results: None,
    };
    let started = Instant::now();
    let mut matched = 0;
    for _ in 0..RUNS {
        let progress = SearchProgress::new(document.line_count());
        let run = search_with_progress(
            black_box(&document),
            black_box(&query),
            &SearchCancellation::default(),
            &progress,
        )
        .expect("性能测试搜索应成功");
        let SearchRun::Completed(result) = run else {
            panic!("性能测试搜索不应取消");
        };
        matched += result.len();
    }
    let elapsed = started.elapsed();

    assert_eq!(matched, LINE_COUNT.div_ceil(97) * RUNS);
    eprintln!(
        "搜索 {LINE_COUNT} 行，执行 {RUNS} 次：{elapsed:?}，平均：{:?}",
        elapsed / RUNS as u32
    );
}
