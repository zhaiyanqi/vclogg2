use std::{
    fs::{self, File},
    hint::black_box,
    io::{BufWriter, Write as _},
    path::PathBuf,
    time::Instant,
};

use vclogg_core::LogDocument;

use crate::state_store::FileSessionState;

use super::{
    SearchPreparationOptions, directory_search_scan_paths, path_match_key, prepare_document,
    prepare_paths_bounded, prepare_paths_bounded_while,
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
fn cached_complete_document_handoff_does_not_reopen_the_source() {
    let temporary = TemporaryDirectory::new("cached-document-handoff");
    let path = temporary.0.join("source.log");
    fs::write(&path, b"alpha\nbeta\n").expect("应能写入缓存交接测试日志");
    let document = std::sync::Arc::new(LogDocument::open(&path).expect("应能打开缓存交接测试日志"));
    fs::remove_file(&path).expect("应能移除缓存交接测试日志");

    let prepared = prepare_document(
        &path,
        Some(document.clone()),
        None,
        None,
        SearchPreparationOptions {
            case_sensitive: false,
            regex: false,
            max_results: None,
        },
        &[],
    )
    .expect("缓存完整文档应在源路径暂时不可用时直接交接");

    assert!(std::sync::Arc::ptr_eq(&prepared.document, &document));
    assert!(prepared.pending_index_cache.is_none());
}

#[test]
fn saved_search_is_prepared_with_the_application_search_options() {
    let temporary = TemporaryDirectory::new("prepared-saved-search");
    let path = temporary.0.join("source.log");
    fs::write(&path, b"alpha\nAlpha\nalpha\n").expect("应能写入已保存搜索测试日志");
    let document =
        std::sync::Arc::new(LogDocument::open(&path).expect("应能打开已保存搜索测试日志"));
    let session = FileSessionState {
        query_text: "alpha".to_owned(),
        ..FileSessionState::default()
    };

    let prepared = prepare_document(
        &path,
        Some(document),
        None,
        Some(session),
        SearchPreparationOptions {
            case_sensitive: true,
            regex: false,
            max_results: Some(100),
        },
        &[],
    )
    .expect("文档准备应恢复已保存搜索");

    assert_eq!(prepared.load_state, super::DocumentLoadState::Ready);
    assert_eq!(
        prepared
            .search_result
            .line_indices
            .iter()
            .collect::<Vec<_>>(),
        vec![0, 2]
    );
    assert!(prepared.search_matcher.is_some());
    assert_eq!(prepared.session.as_ref().unwrap().query_text, "alpha");
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
