use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use super::{APPEND_INTEGRITY_BLOCK_BYTES, DocumentBytes, LogDocument};
use crate::{SearchQuery, search};

static SOURCE_SEQUENCE: AtomicUsize = AtomicUsize::new(0);

struct TestSource(PathBuf);

impl TestSource {
    fn new(bytes: &[u8]) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let sequence = SOURCE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "vclogg2-search-blocks-{}-{nonce}-{sequence}.log",
            std::process::id()
        ));
        fs::write(&path, bytes).unwrap();
        Self(path)
    }

    fn open(&self) -> LogDocument {
        LogDocument::open(&self.0).unwrap()
    }
}

impl Drop for TestSource {
    fn drop(&mut self) {
        _ = fs::remove_file(&self.0);
    }
}

fn block_reads(document: &LogDocument) -> usize {
    let DocumentBytes::Verified(bytes) = document.bytes.as_ref() else {
        panic!("file searches must use positional reads");
    };
    bytes.source_block_reads.load(Ordering::Relaxed)
}

#[test]
fn sequential_search_reads_each_block_once_across_crlf_and_utf8_boundaries() {
    let mut bytes = b"\xef\xbb\xbfhead\n".to_vec();
    bytes.resize(APPEND_INTEGRITY_BLOCK_BYTES - 1, b'x');
    bytes.extend_from_slice(b"\r\ntarget\n");
    bytes.resize(2 * APPEND_INTEGRITY_BLOCK_BYTES - 1, b'y');
    bytes.extend_from_slice("中\ntail\n".as_bytes());
    let source = TestSource::new(&bytes);

    for verify_integrity in [true, false] {
        let document = source.open();
        let mut reader = document.search_lines(verify_integrity);
        let expected = std::str::from_utf8(&bytes[3..]).unwrap();
        for (row, expected) in expected.lines().enumerate() {
            assert_eq!(
                reader.bytes_at_local_row(row).unwrap().as_ref(),
                expected.as_bytes()
            );
        }
        assert_eq!(
            block_reads(&document),
            3,
            "each source block should be read once"
        );
    }
}

#[test]
fn search_reuses_blocks_for_utf16_lines_spanning_multiple_blocks() {
    let text = format!(
        "head\n{}中\r\ntail",
        "x".repeat(APPEND_INTEGRITY_BLOCK_BYTES + 1)
    );
    let bytes = [0xff, 0xfe]
        .into_iter()
        .chain(text.encode_utf16().flat_map(u16::to_le_bytes))
        .collect::<Vec<_>>();
    let source = TestSource::new(&bytes);
    let document = source.open();
    let mut reader = document.search_lines(true);
    for (row, expected) in text.lines().enumerate() {
        assert_eq!(
            reader.bytes_at_local_row(row).unwrap().as_ref(),
            expected.as_bytes()
        );
    }
    assert_eq!(block_reads(&document), 3);
}

#[test]
fn cross_block_search_rejects_changes_in_the_next_block() {
    let mut bytes = b"head\n".to_vec();
    bytes.resize(APPEND_INTEGRITY_BLOCK_BYTES + 8, b'x');
    bytes.extend_from_slice(b"\ntail\n");
    let source = TestSource::new(&bytes);
    let document = source.open();
    let mut reader = document.search_lines(true);
    assert_eq!(reader.bytes_at_local_row(0).unwrap().as_ref(), b"head");

    bytes[APPEND_INTEGRITY_BLOCK_BYTES] = b'y';
    fs::write(&source.0, bytes).unwrap();
    assert!(reader.bytes_at_local_row(1).is_none());
}

#[test]
fn parallel_search_does_not_duplicate_reads_within_one_source_block() {
    let source = TestSource::new(&b"target message\n".repeat(150_000));
    let document = source.open();
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(8)
        .build()
        .unwrap();
    let result = pool
        .install(|| {
            search(
                &document,
                &SearchQuery {
                    text: "target".into(),
                    case_sensitive: true,
                    ..SearchQuery::default()
                },
            )
        })
        .unwrap();

    assert_eq!(result.len(), 150_000);
    assert_eq!(block_reads(&document), 1);
}

#[test]
fn parallel_search_balances_bytes_when_line_lengths_differ() {
    let short = format!("{:<31}\n", "target");
    let long = format!("{:<1023}\n", "target");
    let short_count = APPEND_INTEGRITY_BLOCK_BYTES / short.len();
    let long_count = 3 * APPEND_INTEGRITY_BLOCK_BYTES / long.len();
    let bytes = format!("{}{}", short.repeat(short_count), long.repeat(long_count));
    let source = TestSource::new(bytes.as_bytes());
    let document = source.open();
    let ranges = document.search_row_ranges(4);
    assert_eq!(ranges.len(), 4);
    assert!(ranges[0].len() > ranges[1].len() * 8);
    for rows in &ranges {
        let start = document
            .line_byte_range_at_local_row(rows.start)
            .unwrap()
            .start;
        let end = document
            .line_byte_range_at_local_row(rows.end - 1)
            .unwrap()
            .end;
        assert_eq!(end - start, APPEND_INTEGRITY_BLOCK_BYTES);
    }

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(4)
        .build()
        .unwrap();
    let result = pool
        .install(|| {
            search(
                &document,
                &SearchQuery {
                    text: "target".into(),
                    case_sensitive: true,
                    ..SearchQuery::default()
                },
            )
        })
        .unwrap();
    assert_eq!(result.len(), short_count + long_count);
    assert_eq!(result.line_indices.first(), Some(0));
    assert_eq!(
        result.line_indices.get(result.len() - 1),
        Some(short_count + long_count - 1)
    );
    assert_eq!(block_reads(&document), 4);
}

#[test]
fn byte_partitioned_search_preserves_long_lines_and_sparse_source_rows() {
    let mut bytes = b"head\n".to_vec();
    bytes.resize(3 * APPEND_INTEGRITY_BLOCK_BYTES + 1, b'x');
    bytes.extend_from_slice(b"target\r\ntail\n");
    let source = TestSource::new(&bytes);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(8)
        .build()
        .unwrap();
    for sparse in [false, true] {
        let document = source.open();
        let document = if sparse {
            document.project_source_rows(&[0, 2, 3].into_iter().collect())
        } else {
            document
        };
        for limit in [None, Some(1)] {
            let result = pool
                .install(|| {
                    search(
                        &document,
                        &SearchQuery {
                            text: "head|target|tail".into(),
                            case_sensitive: true,
                            max_results: limit,
                            ..SearchQuery::default()
                        },
                    )
                })
                .unwrap();
            let expected = if sparse { vec![0, 2] } else { vec![0, 1, 2] };
            assert_eq!(
                result.line_indices.iter().collect::<Vec<_>>(),
                expected
                    .into_iter()
                    .take(limit.unwrap_or(usize::MAX))
                    .collect::<Vec<_>>()
            );
            assert_eq!(result.truncated, limit.is_some());
        }
    }
}

#[test]
fn dense_projection_shares_offsets_and_compressed_rows_without_expansion() {
    let source = TestSource::new(&b"target\r\n".repeat(10_000));
    let document = source.open();
    let rows = crate::CompressedRows::from_inclusive_ranges([(0, 5999), (8000, 10_000)]);
    let projected = document.project_source_rows(&rows);
    assert!(projected.shared_line_index);
    assert!(projected.line_ends.is_none());
    let (super::LineStarts::Compact(original), super::LineStarts::Compact(selected)) =
        (&document.line_starts, &projected.line_starts)
    else {
        panic!("compact index expected")
    };
    assert!(std::sync::Arc::ptr_eq(original, selected));
    assert!(std::sync::Arc::ptr_eq(
        &rows.rows,
        &projected.source_rows.as_ref().unwrap().rows
    ));
    assert_eq!(projected.line_count(), 8001);
    assert_eq!(projected.source_row(6000), Some(8000));
    assert_eq!(projected.line(7999), None);
    assert_eq!(projected.line(8000).as_deref(), Some("target"));
    assert_eq!(projected.line(10_000).as_deref(), Some(""));
    let query = SearchQuery {
        text: "target".into(),
        ..SearchQuery::default()
    };
    let result = search(&projected, &query).unwrap();
    assert_eq!(result.len(), 8000);
    let nested = projected.project_source_rows(&[0, 7000, 8000, usize::MAX].into_iter().collect());
    assert_eq!(nested.line_count(), 2);
    assert_eq!(nested.line(8000).as_deref(), Some("target"));
}

#[test]
fn combined_search_only_extends_the_block_owning_a_long_line() {
    let block = APPEND_INTEGRITY_BLOCK_BYTES;
    let mut bytes = vec![b'x'; block * 5 + 13];
    bytes.extend_from_slice(b"target\r\ntail");
    let source = TestSource::new(&bytes);
    let file = fs::File::open(&source.0).unwrap();
    let query = SearchQuery {
        text: "target|tail".into(),
        ..SearchQuery::default()
    };
    let matcher = crate::SearchMatcher::new(&query).unwrap().unwrap();
    for index in 1..5 {
        let mut contents = bytes[index * block..(index + 1) * block].to_vec();
        let result = super::match_parallel_utf8_block_lines(
            &file,
            bytes.len(),
            index * block,
            block,
            &mut contents,
            super::FileEncoding::Utf8,
            &matcher,
            &source.0,
            &|| false,
        )
        .unwrap()
        .unwrap();
        assert!(result.is_empty());
        assert_eq!(
            contents.len(),
            block,
            "middle blocks must not copy the long suffix"
        );
    }
    let (_, combined) = super::build_parallel_utf8_file_index_with_integrity(
        &file,
        bytes.len(),
        super::FileEncoding::Utf8,
        &source.0,
        Some((&matcher, None)),
        &|| false,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        combined.unwrap().line_indices.iter().collect::<Vec<_>>(),
        [0, 1]
    );
}

#[test]
fn combined_search_handles_line_breaks_at_block_edges_without_tail_reads() {
    let block = APPEND_INTEGRITY_BLOCK_BYTES;
    for delimiter in [b"\n".as_slice(), b"\r", b"\r\n"] {
        let mut bytes = vec![b'x'; block - 1];
        bytes.extend_from_slice(delimiter);
        bytes.extend_from_slice(b"target\n");
        let source = TestSource::new(&bytes);
        let file = fs::File::open(&source.0).unwrap();
        let query = SearchQuery {
            text: "^x+$|target|^$".into(),
            regex: true,
            ..SearchQuery::default()
        };
        let matcher = crate::SearchMatcher::new(&query).unwrap().unwrap();
        let mut first = bytes[..block].to_vec();
        super::match_parallel_utf8_block_lines(
            &file,
            bytes.len(),
            0,
            block,
            &mut first,
            super::FileEncoding::Utf8,
            &matcher,
            &source.0,
            &|| false,
        )
        .unwrap();
        assert_eq!(first.len(), block);
        let (_, combined) = super::build_parallel_utf8_file_index_with_integrity(
            &file,
            bytes.len(),
            super::FileEncoding::Utf8,
            &source.0,
            Some((&matcher, None)),
            &|| false,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            combined.unwrap().line_indices,
            search(&source.open(), &query).unwrap().line_indices
        );
    }
}
