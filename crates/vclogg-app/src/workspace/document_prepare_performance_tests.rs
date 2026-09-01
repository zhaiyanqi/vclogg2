use std::{
    fs::{self, File},
    hint::black_box,
    io::{BufWriter, Write as _},
    path::PathBuf,
    time::Instant,
};

use vclogg_core::LogDocument;

use super::{
    directory_search_scan_paths, path_match_key, prepare_paths_bounded, prepare_paths_bounded_while,
};

struct TemporaryDirectory(PathBuf);

impl TemporaryDirectory {
    fn new(label: &str) -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .expect("系统时间应晚于 Unix 纪元")
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("vclogg2-{label}-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&path).expect("应能创建临时文件准备目录");
        Self(path)
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn bounded_path_mapping_preserves_input_order() {
    let paths = (0..12)
        .map(|index| PathBuf::from(format!("file-{index:02}.log")))
        .collect::<Vec<_>>();

    let prepared = prepare_paths_bounded(paths.clone(), |path| path.to_path_buf());

    assert_eq!(
        prepared.into_iter().collect::<Vec<_>>(),
        paths
            .into_iter()
            .map(|path| (path.clone(), path))
            .collect::<Vec<_>>()
    );
}

#[test]
fn cancelled_path_mapping_does_not_start_pending_work() {
    let cancellation = vclogg_core::SearchCancellation::default();
    cancellation.cancel();
    let paths = (0..100)
        .map(|index| PathBuf::from(format!("file-{index:03}.log")))
        .collect::<Vec<_>>();

    let prepared = prepare_paths_bounded_while(
        paths,
        || !cancellation.is_cancelled(),
        |_| panic!("cancelled preparation must not start another file"),
    );

    assert!(prepared.is_empty());
}

#[test]
fn empty_directory_query_only_prepares_open_paths_for_marks() {
    let open_path = PathBuf::from("open.log");
    let other_path = PathBuf::from("other.log");
    let open_paths = [path_match_key(&open_path)].into_iter().collect();

    let scan_paths =
        directory_search_scan_paths(vec![other_path, open_path.clone()], false, &open_paths);

    assert_eq!(scan_paths, [open_path]);
}

#[test]
fn nonempty_directory_query_prepares_every_enumerated_path() {
    let paths = vec![PathBuf::from("first.log"), PathBuf::from("second.log")];

    let scan_paths = directory_search_scan_paths(paths.clone(), true, &Default::default());

    assert_eq!(scan_paths, paths);
}

#[test]
#[ignore = "手动性能基准：cargo test -p vclogg2 --release benchmark_parallel_document_prepare -- --ignored --nocapture"]
fn benchmark_parallel_document_prepare() {
    const FILE_COUNT: usize = 8;
    const FILE_BYTES: usize = 16 * 1024 * 1024;
    let temporary = TemporaryDirectory::new("parallel-document-prepare");
    let line = b"2026-08-27 INFO parallel document preparation line\n";
    let paths = (0..FILE_COUNT)
        .map(|index| {
            let path = temporary.0.join(format!("file-{index:02}.log"));
            let mut writer = BufWriter::new(File::create(&path).expect("应能创建性能测试日志"));
            for _ in 0..FILE_BYTES.div_ceil(line.len()) {
                writer.write_all(line).expect("应能写入性能测试日志");
            }
            writer.flush().expect("应能刷新性能测试日志");
            path
        })
        .collect::<Vec<_>>();

    let sequential_started = Instant::now();
    let sequential = paths
        .iter()
        .map(|path| LogDocument::open(black_box(path)).expect("串行文件准备应成功"))
        .collect::<Vec<_>>();
    let sequential_elapsed = sequential_started.elapsed();
    black_box(sequential);

    let parallel_started = Instant::now();
    let parallel = prepare_paths_bounded(paths, |path| {
        LogDocument::open(black_box(path)).expect("并行文件准备应成功")
    });
    let parallel_elapsed = parallel_started.elapsed();
    black_box(parallel);

    eprintln!(
        "准备 {FILE_COUNT} 个 16 MiB 文件：串行 {sequential_elapsed:?}；最多 4 路并行 {parallel_elapsed:?}"
    );
}
