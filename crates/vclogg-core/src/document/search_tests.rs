use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use super::{APPEND_INTEGRITY_BLOCK_BYTES, DocumentBytes, LogDocument};

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
