use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fmt::Write as _,
    fs::{self, File},
    io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write},
    ops::{ControlFlow, Deref},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(windows)]
use std::{
    os::windows::ffi::OsStrExt as _, os::windows::fs::FileExt as _,
    os::windows::io::AsRawHandle as _,
};

#[cfg(unix)]
use std::os::unix::{ffi::OsStrExt as _, fs::FileExt as _, fs::MetadataExt as _};

use anyhow::{Context as _, Result};
use chardetng::EncodingDetector;
use encoding_rs::{Encoding, UTF_8, UTF_16BE, UTF_16LE};
use memchr::{memchr_iter, memchr2};
use memmap2::{Mmap, MmapOptions};
use sha2::{Digest as _, Sha256};

use crate::{CancellationToken, search::CompressedRows};

#[cfg(windows)]
use windows_sys::Win32::{
    Storage::FileSystem::{
        FILE_ID_INFO, FileIdInfo, GetFileInformationByHandleEx, MOVEFILE_REPLACE_EXISTING,
        MOVEFILE_WRITE_THROUGH, MoveFileExW,
    },
    System::{IO::DeviceIoControl, Ioctl::FSCTL_READ_FILE_USN_DATA},
};

const APPEND_SAMPLE_BYTES: usize = 64 * 1024;
const APPEND_INTEGRITY_BLOCK_BYTES: usize = 4 * 1024 * 1024;
const VERIFIED_BLOCK_CACHE_BYTES: usize = 8 * 1024 * 1024;
const INDEX_CACHE_MAGIC: &[u8; 8] = b"VCLOGG05";
const INDEX_CACHE_VERSION: u32 = 3;
const INDEX_CACHE_HEADER_BYTES: u64 = 8 + 4 + 4 + 2 + 8 + 8 + 1 + 8 + 16 + 8 + 8 * 7;
const MAX_CACHE_PATH_BYTES: usize = 32 * 1024;
const MAX_CACHE_ENCODING_BYTES: usize = 64;
const ENCODING_DETECTION_BYTES: usize = 1024 * 1024;
const BINARY_BYTES_PER_LINE: usize = 16;
const PARALLEL_INDEX_HASH_MIN_BYTES: usize = 8 * 1024 * 1024;
const INDEX_CANCELLATION_BATCH_LINES: usize = 4 * 1024;
const CACHE_CANCELLATION_BATCH_LINES: usize = 4 * 1024;

/// Stable metadata captured from the source file when its index is built.
#[derive(Clone, Debug)]
pub struct DocumentMetadata {
    pub path: PathBuf,
    pub file_size: u64,
    pub modified: Option<SystemTime>,
    pub line_count: usize,
    pub longest_line_bytes: usize,
    pub longest_line_columns: usize,
    pub encoding_name: String,
}

#[derive(Clone, Copy)]
enum FileEncoding {
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
    Legacy(&'static Encoding),
    Binary,
}

impl FileEncoding {
    fn name(self) -> String {
        match self {
            Self::Utf8 => "UTF-8".to_owned(),
            Self::Utf8Bom => "UTF-8 BOM".to_owned(),
            Self::Utf16Le => "UTF-16LE BOM".to_owned(),
            Self::Utf16Be => "UTF-16BE BOM".to_owned(),
            Self::Legacy(encoding) => encoding.name().to_owned(),
            Self::Binary => "Binary".to_owned(),
        }
    }

    fn bom_len(self) -> usize {
        match self {
            Self::Utf8Bom => 3,
            Self::Utf16Le | Self::Utf16Be => 2,
            Self::Utf8 | Self::Legacy(_) | Self::Binary => 0,
        }
    }

    fn from_cache_name(name: &[u8]) -> Option<Self> {
        match name {
            b"UTF-8" => Some(Self::Utf8),
            b"UTF-8 BOM" => Some(Self::Utf8Bom),
            b"UTF-16LE BOM" => Some(Self::Utf16Le),
            b"UTF-16BE BOM" => Some(Self::Utf16Be),
            b"Binary" => Some(Self::Binary),
            name => Encoding::for_label(name).map(Self::Legacy),
        }
    }

    fn decode(self, bytes: &[u8]) -> String {
        match self {
            Self::Utf8 | Self::Utf8Bom => String::from_utf8_lossy(bytes).into_owned(),
            Self::Utf16Le => UTF_16LE.decode_without_bom_handling(bytes).0.into_owned(),
            Self::Utf16Be => UTF_16BE.decode_without_bom_handling(bytes).0.into_owned(),
            Self::Legacy(encoding) => encoding.decode_without_bom_handling(bytes).0.into_owned(),
            Self::Binary => {
                let mut decoded = String::with_capacity(bytes.len().saturating_mul(3));
                for (ix, byte) in bytes.iter().enumerate() {
                    if ix > 0 {
                        decoded.push(' ');
                    }
                    write!(&mut decoded, "{byte:02x}").expect("writing to String cannot fail");
                }
                decoded
            }
        }
    }

    fn decode_preview(self, bytes: &[u8], max_bytes: usize) -> LinePreview {
        let end = match self {
            Self::Utf8 | Self::Utf8Bom => utf8_preview_end(bytes, max_bytes),
            Self::Utf16Le => utf16_preview_end(bytes, max_bytes, true),
            Self::Utf16Be => utf16_preview_end(bytes, max_bytes, false),
            Self::Legacy(_) | Self::Binary => bytes.len().min(max_bytes),
        };
        LinePreview {
            text: self.decode(&bytes[..end]),
            truncated: end < bytes.len(),
        }
    }

    fn decode_for_search(self, bytes: &[u8]) -> std::borrow::Cow<'_, [u8]> {
        match self {
            Self::Utf8 | Self::Utf8Bom => std::borrow::Cow::Borrowed(bytes),
            Self::Utf16Le | Self::Utf16Be | Self::Legacy(_) | Self::Binary => {
                std::borrow::Cow::Owned(self.decode(bytes).into_bytes())
            }
        }
    }

    fn trim_line_bytes(self, bytes: &[u8]) -> &[u8] {
        match self {
            Self::Utf16Le => bytes
                .strip_suffix(&[b'\n', 0])
                .unwrap_or(bytes)
                .strip_suffix(&[b'\r', 0])
                .unwrap_or_else(|| bytes.strip_suffix(&[b'\n', 0]).unwrap_or(bytes)),
            Self::Utf16Be => bytes
                .strip_suffix(&[0, b'\n'])
                .unwrap_or(bytes)
                .strip_suffix(&[0, b'\r'])
                .unwrap_or_else(|| bytes.strip_suffix(&[0, b'\n']).unwrap_or(bytes)),
            Self::Utf8 | Self::Utf8Bom | Self::Legacy(_) => bytes
                .strip_suffix(b"\n")
                .unwrap_or(bytes)
                .strip_suffix(b"\r")
                .unwrap_or_else(|| bytes.strip_suffix(b"\n").unwrap_or(bytes)),
            Self::Binary => bytes,
        }
    }
}

fn utf8_preview_end(bytes: &[u8], max_bytes: usize) -> usize {
    let mut end = bytes.len().min(max_bytes);
    if end == bytes.len() {
        return end;
    }
    while end > 0 && bytes[end] & 0b1100_0000 == 0b1000_0000 {
        end -= 1;
    }
    end
}

fn utf16_preview_end(bytes: &[u8], max_bytes: usize, little_endian: bool) -> usize {
    let mut end = bytes.len().min(max_bytes);
    end -= end % 2;
    if end < bytes.len() && end >= 2 {
        let pair = [bytes[end - 2], bytes[end - 1]];
        let last = if little_endian {
            u16::from_le_bytes(pair)
        } else {
            u16::from_be_bytes(pair)
        };
        if (0xD800..=0xDBFF).contains(&last) {
            end -= 2;
        }
    }
    end
}

/// How a changed source file produced its replacement snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DocumentRefreshKind {
    /// The old line index was retained and only newly appended bytes were scanned.
    Appended,
    /// The source could not be verified as an append, so the full index was rebuilt.
    Rebuilt,
}

enum DocumentBytes {
    Empty,
    Owned(Box<[u8]>),
    Verified(Box<VerifiedFileBytes>),
}

struct VerifiedFileBytes {
    len: usize,
    integrity_blocks: Arc<[[u8; 32]]>,
    path: PathBuf,
    identity: Option<FileIdentity>,
    transient_source_handles: AtomicBool,
    file: RwLock<Option<Arc<File>>>,
    state: Mutex<VerifiedFileState>,
}

struct VerifiedFileState {
    blocks: BTreeMap<usize, Arc<[u8]>>,
    invalid_blocks: BTreeSet<usize>,
    block_order: VecDeque<usize>,
    cached_bytes: usize,
}

enum DocumentByteStorage<'a> {
    Borrowed(&'a [u8]),
    Shared(Arc<[u8]>),
    Owned(Box<[u8]>),
}

pub(crate) struct DocumentLineBytes<'a> {
    storage: DocumentByteStorage<'a>,
    range: std::ops::Range<usize>,
}

impl Deref for DocumentLineBytes<'_> {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        let bytes = match &self.storage {
            DocumentByteStorage::Borrowed(bytes) => *bytes,
            DocumentByteStorage::Shared(bytes) => bytes,
            DocumentByteStorage::Owned(bytes) => bytes,
        };
        &bytes[self.range.clone()]
    }
}

impl DocumentLineBytes<'_> {
    fn truncate(&mut self, len: usize) {
        self.range.end = self.range.start.saturating_add(len).min(self.range.end);
    }
}

impl DocumentBytes {
    fn verified(file: File, path: PathBuf, len: usize, integrity_blocks: Arc<[[u8; 32]]>) -> Self {
        if len == 0 {
            Self::Empty
        } else {
            let identity = read_file_identity(&file);
            Self::Verified(Box::new(VerifiedFileBytes {
                len,
                integrity_blocks,
                path,
                identity,
                transient_source_handles: AtomicBool::new(false),
                file: RwLock::new(Some(Arc::new(file))),
                state: Mutex::new(VerifiedFileState {
                    blocks: BTreeMap::new(),
                    invalid_blocks: BTreeSet::new(),
                    block_order: VecDeque::new(),
                    cached_bytes: 0,
                }),
            }))
        }
    }

    fn len(&self) -> usize {
        match self {
            Self::Empty => 0,
            Self::Owned(bytes) => bytes.len(),
            Self::Verified(bytes) => bytes.len,
        }
    }

    fn read_range(&self, range: std::ops::Range<usize>) -> Option<DocumentLineBytes<'_>> {
        match self {
            Self::Empty if range.is_empty() => Some(DocumentLineBytes {
                storage: DocumentByteStorage::Borrowed(&[]),
                range: 0..0,
            }),
            Self::Empty => None,
            Self::Owned(bytes) => {
                bytes.get(range.clone())?;
                Some(DocumentLineBytes {
                    storage: DocumentByteStorage::Borrowed(bytes),
                    range,
                })
            }
            Self::Verified(bytes) => bytes.read_range(range),
        }
    }

    fn read_unverified_range(&self, range: std::ops::Range<usize>) -> Option<Vec<u8>> {
        match self {
            Self::Empty if range.is_empty() => Some(Vec::new()),
            Self::Empty => None,
            Self::Owned(bytes) => Some(bytes.get(range)?.to_vec()),
            Self::Verified(bytes) => bytes.read_unverified_range(range),
        }
    }

    fn resident_slice(&self) -> Option<&[u8]> {
        match self {
            Self::Empty => Some(&[]),
            Self::Owned(bytes) => Some(bytes),
            Self::Verified(_) => None,
        }
    }

    fn release_source_handle(&self) {
        if let Self::Verified(bytes) = self {
            bytes.release_source_handle();
        }
    }
}

impl VerifiedFileBytes {
    fn release_source_handle(&self) {
        self.transient_source_handles.store(true, Ordering::Release);
        if let Ok(mut file) = self.file.write() {
            *file = None;
        }
    }

    fn source_file(&self) -> Option<Arc<File>> {
        if !self.transient_source_handles.load(Ordering::Acquire)
            && let Some(file) = self.file.read().ok()?.clone()
        {
            return Some(file);
        }

        let file = Arc::new(File::open(&self.path).ok()?);
        let metadata = file.metadata().ok()?;
        if metadata.len() != u64::try_from(self.len).ok()? {
            return None;
        }
        if let Some(expected) = &self.identity
            && read_file_identity(&file).as_ref() != Some(expected)
        {
            return None;
        }
        if self.transient_source_handles.load(Ordering::Acquire) {
            return Some(file);
        }

        let mut retained = self.file.write().ok()?;
        if self.transient_source_handles.load(Ordering::Acquire) {
            return Some(file);
        }
        Some(retained.get_or_insert(file).clone())
    }

    fn read_range(&self, range: std::ops::Range<usize>) -> Option<DocumentLineBytes<'_>> {
        if range.start > range.end || range.end > self.len {
            return None;
        }
        if range.is_empty() {
            return Some(DocumentLineBytes {
                storage: DocumentByteStorage::Borrowed(&[]),
                range: 0..0,
            });
        }
        let first_block = range.start / APPEND_INTEGRITY_BLOCK_BYTES;
        let last_block = (range.end - 1) / APPEND_INTEGRITY_BLOCK_BYTES;
        if first_block == last_block {
            let block = self.load_verified_block(first_block, true)?;
            let block_start = first_block.saturating_mul(APPEND_INTEGRITY_BLOCK_BYTES);
            return Some(DocumentLineBytes {
                storage: DocumentByteStorage::Shared(block),
                range: range.start - block_start..range.end - block_start,
            });
        }

        let mut joined = Vec::with_capacity(range.len());
        for block_ix in first_block..=last_block {
            let block = self.load_verified_block(block_ix, true)?;
            let block_start = block_ix.saturating_mul(APPEND_INTEGRITY_BLOCK_BYTES);
            let start = range.start.saturating_sub(block_start).min(block.len());
            let end = range.end.saturating_sub(block_start).min(block.len());
            joined.extend_from_slice(block.get(start..end)?);
        }
        let len = joined.len();
        Some(DocumentLineBytes {
            storage: DocumentByteStorage::Owned(joined.into_boxed_slice()),
            range: 0..len,
        })
    }

    fn read_unverified_range(&self, range: std::ops::Range<usize>) -> Option<Vec<u8>> {
        if range.start > range.end || range.end > self.len {
            return None;
        }
        let mut bytes = vec![0_u8; range.len()];
        if bytes.is_empty() {
            return Some(bytes);
        }
        let file = self.source_file()?;
        read_file_exact_at(&file, &mut bytes, range.start as u64).ok()?;
        Some(bytes)
    }

    fn load_verified_block(&self, block_ix: usize, retain: bool) -> Option<Arc<[u8]>> {
        let retain = retain && !self.transient_source_handles.load(Ordering::Acquire);
        {
            let mut state = self.state.lock().ok()?;
            if let Some(block) = state.blocks.get(&block_ix).cloned() {
                if let Some(position) = state
                    .block_order
                    .iter()
                    .position(|entry| *entry == block_ix)
                {
                    state.block_order.remove(position);
                }
                state.block_order.push_back(block_ix);
                return Some(block);
            }
            if state.invalid_blocks.contains(&block_ix) {
                return None;
            }
        }

        let block = (|| {
            let block_start = block_ix.checked_mul(APPEND_INTEGRITY_BLOCK_BYTES)?;
            let block_end = block_start
                .checked_add(APPEND_INTEGRITY_BLOCK_BYTES)?
                .min(self.len);
            let mut block = vec![0_u8; block_end.checked_sub(block_start)?];
            let file = self.source_file()?;
            read_file_exact_at(&file, &mut block, block_start as u64).ok()?;
            let expected = self.integrity_blocks.get(block_ix)?;
            (<[u8; 32]>::from(Sha256::digest(&block)) == *expected).then_some(block)
        })();
        let Some(block) = block else {
            let mut state = self.state.lock().ok()?;
            if let Some(block) = state.blocks.get(&block_ix).cloned() {
                return Some(block);
            }
            state.invalid_blocks.insert(block_ix);
            return None;
        };

        let block: Arc<[u8]> = block.into();
        if !retain {
            return Some(block);
        }
        let mut state = self.state.lock().ok()?;
        if let Some(block) = state.blocks.get(&block_ix).cloned() {
            return Some(block);
        }
        if state.invalid_blocks.contains(&block_ix) {
            return None;
        }
        while state.cached_bytes.saturating_add(block.len()) > VERIFIED_BLOCK_CACHE_BYTES {
            let evicted_ix = state.block_order.pop_front()?;
            if let Some(evicted) = state.blocks.remove(&evicted_ix) {
                state.cached_bytes = state.cached_bytes.saturating_sub(evicted.len());
            }
        }
        state.cached_bytes = state.cached_bytes.saturating_add(block.len());
        state.blocks.insert(block_ix, block.clone());
        state.block_order.push_back(block_ix);
        Some(block)
    }
}

#[cfg(unix)]
fn read_file_exact_at(file: &File, bytes: &mut [u8], offset: u64) -> std::io::Result<()> {
    file.read_exact_at(bytes, offset)
}

#[cfg(windows)]
fn read_file_exact_at(file: &File, mut bytes: &mut [u8], mut offset: u64) -> std::io::Result<()> {
    while !bytes.is_empty() {
        let read = file.seek_read(bytes, offset)?;
        if read == 0 {
            return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof));
        }
        offset = offset.saturating_add(read as u64);
        bytes = &mut bytes[read..];
    }
    Ok(())
}

#[derive(Clone)]
struct AppendFingerprint {
    head: Arc<[u8]>,
    tail: Arc<[u8]>,
    integrity_blocks: Arc<[[u8; 32]]>,
}

struct IndexedLines {
    starts: MutableLineStarts,
    longest_line_bytes: usize,
    longest_completed_line_bytes: usize,
    longest_line_columns: usize,
    longest_completed_line_columns: usize,
}

/// Immutable line offsets compacted to four bytes whenever the snapshot fits in `u32`.
///
/// Index construction and append refreshes use the matching mutable width, so common files never
/// need a transient eight-byte offset table. A compact table promotes only if an append crosses the
/// `u32` byte boundary, while readers keep the same constant-time lookup semantics.
#[derive(Clone)]
enum LineStarts {
    Compact(Arc<[u32]>),
    Wide(Arc<[usize]>),
}

impl LineStarts {
    #[cfg(test)]
    fn from_native(starts: Vec<usize>) -> Self {
        MutableLineStarts::from_native(starts).into_immutable()
    }

    fn len(&self) -> usize {
        match self {
            Self::Compact(starts) => starts.len(),
            Self::Wide(starts) => starts.len(),
        }
    }

    fn get(&self, row_ix: usize) -> Option<usize> {
        match self {
            Self::Compact(starts) => starts.get(row_ix).map(|offset| *offset as usize),
            Self::Wide(starts) => starts.get(row_ix).copied(),
        }
    }

    fn iter(&self) -> LineStartsIter<'_> {
        match self {
            Self::Compact(starts) => LineStartsIter::Compact(starts.iter()),
            Self::Wide(starts) => LineStartsIter::Wide(starts.iter()),
        }
    }
}

enum MutableLineStarts {
    Compact(Vec<u32>),
    Wide(Vec<usize>),
}

impl MutableLineStarts {
    fn with_capacity(max_offset: u64, capacity: usize) -> Self {
        if max_offset <= u64::from(u32::MAX) {
            Self::Compact(Vec::with_capacity(capacity))
        } else {
            Self::Wide(Vec::with_capacity(capacity))
        }
    }

    fn from_native(starts: Vec<usize>) -> Self {
        let compact_limit = usize::try_from(u32::MAX).unwrap_or(usize::MAX);
        if starts.last().copied().unwrap_or_default() <= compact_limit {
            Self::Compact(
                starts
                    .into_iter()
                    .map(|offset| {
                        u32::try_from(offset)
                            .expect("line offsets were checked against the compact limit")
                    })
                    .collect(),
            )
        } else {
            Self::Wide(starts)
        }
    }

    fn from_immutable(starts: &LineStarts) -> Self {
        match starts {
            LineStarts::Compact(starts) => Self::Compact(starts.to_vec()),
            LineStarts::Wide(starts) => Self::Wide(starts.to_vec()),
        }
    }

    fn len(&self) -> usize {
        match self {
            Self::Compact(starts) => starts.len(),
            Self::Wide(starts) => starts.len(),
        }
    }

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn get(&self, row_ix: usize) -> Option<usize> {
        match self {
            Self::Compact(starts) => starts.get(row_ix).map(|offset| *offset as usize),
            Self::Wide(starts) => starts.get(row_ix).copied(),
        }
    }

    fn last(&self) -> Option<usize> {
        self.len().checked_sub(1).and_then(|index| self.get(index))
    }

    fn push(&mut self, offset: usize) {
        match self {
            Self::Compact(starts) => match u32::try_from(offset) {
                Ok(offset) => starts.push(offset),
                Err(_) => {
                    let mut wide = Vec::with_capacity(starts.len().saturating_add(1));
                    wide.extend(starts.iter().map(|offset| *offset as usize));
                    wide.push(offset);
                    *self = Self::Wide(wide);
                }
            },
            Self::Wide(starts) => starts.push(offset),
        }
    }

    fn truncate(&mut self, len: usize) {
        match self {
            Self::Compact(starts) => starts.truncate(len),
            Self::Wide(starts) => starts.truncate(len),
        }
    }

    fn into_immutable(self) -> LineStarts {
        match self {
            Self::Compact(starts) => LineStarts::Compact(starts.into()),
            Self::Wide(starts) => LineStarts::Wide(starts.into()),
        }
    }
}

enum LineStartsIter<'a> {
    Compact(std::slice::Iter<'a, u32>),
    Wide(std::slice::Iter<'a, usize>),
}

impl Iterator for LineStartsIter<'_> {
    type Item = usize;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Compact(starts) => starts.next().map(|offset| *offset as usize),
            Self::Wide(starts) => starts.next().copied(),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.len();
        (len, Some(len))
    }
}

impl ExactSizeIterator for LineStartsIter<'_> {
    fn len(&self) -> usize {
        match self {
            Self::Compact(starts) => starts.len(),
            Self::Wide(starts) => starts.len(),
        }
    }
}

struct CachedIndex {
    indexed_lines: IndexedLines,
    encoding: FileEncoding,
    integrity_blocks: Arc<[[u8; 32]]>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FileIdentity {
    volume_serial: u64,
    file_id: [u8; 16],
    usn: i64,
}

/// A validated index snapshot that can be persisted after its document has
/// already become usable in the UI.
pub struct PendingIndexCacheWrite {
    cache_path: PathBuf,
    source_path: PathBuf,
    file_size: u64,
    modified_millis: u64,
    identity: Option<FileIdentity>,
    encoding: FileEncoding,
    line_starts: LineStarts,
    integrity_blocks: Arc<[[u8; 32]]>,
    longest_line_bytes: usize,
    longest_completed_line_bytes: usize,
    longest_line_columns: usize,
    longest_completed_line_columns: usize,
}

/// A bounded decoded prefix for interactive display.
///
/// Search, export, and explicit copy operations continue to use [`LogDocument::line`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LinePreview {
    text: String,
    truncated: bool,
}

impl LinePreview {
    pub fn new(text: impl Into<String>, truncated: bool) -> Self {
        Self {
            text: text.into(),
            truncated,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn is_truncated(&self) -> bool {
        self.truncated
    }

    pub fn into_parts(self) -> (String, bool) {
        (self.text, self.truncated)
    }
}

impl PendingIndexCacheWrite {
    /// Atomically publish the cache. Failure never affects the open document.
    pub fn persist(self) -> Result<()> {
        write_index_cache(&self)
    }
}

/// A read-only log snapshot backed by verified positional reads and a line-start index.
///
/// The document owns no UI state. Reopening a changed source creates a new snapshot,
/// which lets the application swap generations atomically after background work ends.
pub struct LogDocument {
    bytes: Arc<DocumentBytes>,
    line_starts: LineStarts,
    line_ends: Option<LineStarts>,
    source_rows: Option<CompressedRows>,
    segment_start_row: usize,
    metadata: DocumentMetadata,
    longest_completed_line_bytes: usize,
    longest_completed_line_columns: usize,
    append_fingerprint: AppendFingerprint,
    content_digest: [u8; 32],
    encoding: FileEncoding,
}

pub(crate) struct DocumentSearchLines<'a> {
    document: &'a LogDocument,
    verified_block: Option<(usize, Option<Arc<[u8]>>)>,
}

impl DocumentSearchLines<'_> {
    pub(crate) fn bytes_at_local_row(
        &mut self,
        row_ix: usize,
    ) -> Option<std::borrow::Cow<'_, [u8]>> {
        let document = self.document;
        let range = document.line_byte_range_at_local_row(row_ix)?;
        match document.bytes.as_ref() {
            DocumentBytes::Empty => range.is_empty().then_some(std::borrow::Cow::Borrowed(&[])),
            DocumentBytes::Owned(bytes) => {
                let bytes = bytes.get(range)?;
                Some(
                    document
                        .encoding
                        .decode_for_search(document.encoding.trim_line_bytes(bytes)),
                )
            }
            DocumentBytes::Verified(bytes) => {
                if range.is_empty() {
                    return Some(std::borrow::Cow::Borrowed(&[]));
                }
                let first_block = range.start / APPEND_INTEGRITY_BLOCK_BYTES;
                let last_block = (range.end - 1) / APPEND_INTEGRITY_BLOCK_BYTES;
                if first_block != last_block {
                    let mut joined = Vec::with_capacity(range.len());
                    for block_ix in first_block..=last_block {
                        let block = bytes.load_verified_block(block_ix, false)?;
                        let block_start = block_ix.saturating_mul(APPEND_INTEGRITY_BLOCK_BYTES);
                        let start = range.start.saturating_sub(block_start).min(block.len());
                        let end = range.end.saturating_sub(block_start).min(block.len());
                        joined.extend_from_slice(block.get(start..end)?);
                    }
                    let trimmed_len = document.encoding.trim_line_bytes(&joined).len();
                    joined.truncate(trimmed_len);
                    return Some(std::borrow::Cow::Owned(
                        document.encoding.decode_for_search(&joined).into_owned(),
                    ));
                }
                if self
                    .verified_block
                    .as_ref()
                    .is_none_or(|(block_ix, _)| *block_ix != first_block)
                {
                    self.verified_block =
                        Some((first_block, bytes.load_verified_block(first_block, false)));
                }
                let block = self.verified_block.as_ref()?.1.as_ref()?;
                let block_start = first_block.saturating_mul(APPEND_INTEGRITY_BLOCK_BYTES);
                let line = block.get(range.start - block_start..range.end - block_start)?;
                Some(
                    document
                        .encoding
                        .decode_for_search(document.encoding.trim_line_bytes(line)),
                )
            }
        }
    }
}

impl LogDocument {
    /// Create an I/O-free shell used to register a stable opening document
    /// before its bounded preview starts.
    pub fn placeholder(path: impl AsRef<Path>) -> Self {
        Self::from_parts(
            path.as_ref().to_path_buf(),
            DocumentBytes::Empty,
            IndexedLines {
                starts: MutableLineStarts::Compact(Vec::new()),
                longest_line_bytes: 0,
                longest_completed_line_bytes: 0,
                longest_line_columns: 0,
                longest_completed_line_columns: 0,
            },
            None,
            0,
            FileEncoding::Utf8,
            None,
        )
    }

    /// Open a file and build its line index in one sequential pass.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let cancellation = CancellationToken::default();
        Ok(Self::open_cancellable(path, &cancellation)?
            .expect("a fresh cancellation token cannot cancel document opening"))
    }

    /// Open a file while allowing a caller to stop line-index construction.
    ///
    /// Cancellation is expected control flow and returns `Ok(None)`.
    pub fn open_cancellable(
        path: impl AsRef<Path>,
        cancellation: &CancellationToken,
    ) -> Result<Option<Self>> {
        if cancellation.is_cancelled() {
            return Ok(None);
        }
        let path = path.as_ref().to_path_buf();
        let mut file =
            File::open(&path).with_context(|| format!("无法打开日志文件：{}", path.display()))?;
        let file_metadata = file
            .metadata()
            .with_context(|| format!("无法读取文件信息：{}", path.display()))?;

        let encoding = detect_file_encoding(&mut file, file_metadata.len())
            .with_context(|| format!("无法检测日志编码：{}", path.display()))?;
        if cancellation.is_cancelled() {
            return Ok(None);
        }
        let mapped = map_snapshot(&file, file_metadata.len(), &path)?;
        let scan_bytes = mapped.as_deref().unwrap_or_default();
        let Some((indexed_lines, integrity_blocks)) =
            build_line_index_with_integrity_while(scan_bytes, encoding, &|| {
                cancellation.is_cancelled()
            })
        else {
            return Ok(None);
        };
        drop(mapped);
        let bytes = DocumentBytes::verified(
            file,
            path.clone(),
            usize::try_from(file_metadata.len())
                .with_context(|| format!("日志文件过大，无法读取：{}", path.display()))?,
            integrity_blocks.clone(),
        );

        Ok(Some(Self::from_parts(
            path,
            bytes,
            indexed_lines,
            file_metadata.modified().ok(),
            file_metadata.len(),
            encoding,
            Some(integrity_blocks),
        )))
    }

    /// Open a complete document, reusing a line index only when the cached
    /// platform file identity still exactly matches the source.
    pub fn open_with_index_cache(
        path: impl AsRef<Path>,
        cache_dir: impl AsRef<Path>,
    ) -> Result<(Self, Option<PendingIndexCacheWrite>)> {
        let cancellation = CancellationToken::default();
        Ok(
            Self::open_with_index_cache_cancellable(path, cache_dir, &cancellation)?
                .expect("a fresh cancellation token cannot cancel document opening"),
        )
    }

    /// Open a complete document with a validated index cache while observing cancellation.
    ///
    /// Cancellation is expected control flow and returns `Ok(None)`.
    pub fn open_with_index_cache_cancellable(
        path: impl AsRef<Path>,
        cache_dir: impl AsRef<Path>,
        cancellation: &CancellationToken,
    ) -> Result<Option<(Self, Option<PendingIndexCacheWrite>)>> {
        if cancellation.is_cancelled() {
            return Ok(None);
        }
        let path = path.as_ref().to_path_buf();
        let mut file =
            File::open(&path).with_context(|| format!("无法打开日志文件：{}", path.display()))?;
        let file_metadata = file
            .metadata()
            .with_context(|| format!("无法读取文件信息：{}", path.display()))?;
        let file_size = file_metadata.len();
        let modified = file_metadata.modified().ok();
        let modified_millis = system_time_millis(modified);
        let identity = read_file_identity(&file);
        let cache_path = index_cache_path(cache_dir.as_ref(), &path);
        let cached = read_index_cache_while(
            &cache_path,
            &path,
            file_size,
            modified_millis,
            identity.as_ref(),
            &|| cancellation.is_cancelled(),
        );
        if cancellation.is_cancelled() {
            return Ok(None);
        }

        let cache_missed = cached.is_none();
        let encoding = match cached.as_ref() {
            Some(cached) => cached.encoding,
            None => detect_file_encoding(&mut file, file_size)
                .with_context(|| format!("无法检测日志编码：{}", path.display()))?,
        };
        if cancellation.is_cancelled() {
            return Ok(None);
        }
        let (indexed_lines, integrity_blocks) = match cached {
            Some(cached) => (cached.indexed_lines, Some(cached.integrity_blocks)),
            None => {
                let mapped = map_snapshot(&file, file_size, &path)?;
                let scan_bytes = mapped.as_deref().unwrap_or_default();
                let Some((indexed_lines, integrity_blocks)) =
                    build_line_index_with_integrity_while(scan_bytes, encoding, &|| {
                        cancellation.is_cancelled()
                    })
                else {
                    return Ok(None);
                };
                (indexed_lines, Some(integrity_blocks))
            }
        };
        if cancellation.is_cancelled() {
            return Ok(None);
        }
        let integrity_blocks = integrity_blocks
            .expect("complete documents always carry integrity blocks from cache or indexing");
        let bytes = DocumentBytes::verified(
            file,
            path.clone(),
            usize::try_from(file_size)
                .with_context(|| format!("日志文件过大，无法读取：{}", path.display()))?,
            integrity_blocks.clone(),
        );
        let document = Self::from_parts(
            path.clone(),
            bytes,
            indexed_lines,
            modified,
            file_size,
            encoding,
            Some(integrity_blocks),
        );
        let pending_cache_write = cache_missed.then(|| PendingIndexCacheWrite {
            cache_path,
            source_path: path,
            file_size,
            modified_millis,
            identity,
            encoding,
            line_starts: document.line_starts.clone(),
            integrity_blocks: document.append_fingerprint.integrity_blocks.clone(),
            longest_line_bytes: document.metadata.longest_line_bytes,
            longest_completed_line_bytes: document.longest_completed_line_bytes,
            longest_line_columns: document.metadata.longest_line_columns,
            longest_completed_line_columns: document.longest_completed_line_columns,
        });
        if cancellation.is_cancelled() {
            Ok(None)
        } else {
            Ok(Some((document, pending_cache_write)))
        }
    }

    /// Open a bounded head preview without scanning the remainder of the file.
    /// Returns whether the preview already contains the complete source file.
    pub fn open_preview(
        path: impl AsRef<Path>,
        byte_limit: usize,
        line_limit: usize,
    ) -> Result<(Self, bool)> {
        let path = path.as_ref().to_path_buf();
        let mut file =
            File::open(&path).with_context(|| format!("无法打开日志文件：{}", path.display()))?;
        let file_metadata = file
            .metadata()
            .with_context(|| format!("无法读取文件信息：{}", path.display()))?;
        let source_size = file_metadata.len();
        let (encoding, mut preview) = detect_file_encoding_with_sample(&mut file, source_size)
            .with_context(|| format!("无法检测日志编码：{}", path.display()))?;
        let read_len = usize::try_from(source_size.min(byte_limit as u64))
            .with_context(|| format!("日志预览范围过大：{}", path.display()))?;
        if preview.len() < read_len {
            let sample_len = preview.len();
            preview.resize(read_len, 0);
            file.read_exact(&mut preview[sample_len..])
                .with_context(|| format!("无法读取日志预览：{}", path.display()))?;
        } else {
            preview.truncate(read_len);
        }

        let visible_len = preview_visible_len(&preview, encoding, line_limit);
        preview.truncate(visible_len);
        let bytes = DocumentBytes::Owned(preview.into_boxed_slice());
        let mut indexed_lines = build_line_index(
            bytes
                .resident_slice()
                .expect("bounded previews always keep resident bytes"),
            encoding,
        );
        let within_line_limit = indexed_lines.starts.len() <= line_limit;
        if !matches!(encoding, FileEncoding::Binary) && indexed_lines.starts.len() > line_limit {
            indexed_lines.starts.truncate(line_limit);
        }
        let complete = visible_len as u64 == source_size && within_line_limit;
        Ok((
            Self::from_parts(
                path,
                bytes,
                indexed_lines,
                file_metadata.modified().ok(),
                source_size,
                encoding,
                None,
            ),
            complete,
        ))
    }

    /// Read a bounded window around a saved source row from a validated index
    /// cache. Stale or unverifiable caches return `None` and must fall back to
    /// the ordinary head preview.
    pub fn open_cached_preview(
        path: impl AsRef<Path>,
        cache_dir: impl AsRef<Path>,
        preferred_row: usize,
        line_limit: usize,
        byte_limit: usize,
    ) -> Result<Option<Self>> {
        if line_limit == 0 || byte_limit == 0 {
            return Ok(None);
        }
        let path = path.as_ref().to_path_buf();
        let mut file =
            File::open(&path).with_context(|| format!("无法打开日志文件：{}", path.display()))?;
        let file_metadata = file
            .metadata()
            .with_context(|| format!("无法读取文件信息：{}", path.display()))?;
        let source_size = file_metadata.len();
        let modified = file_metadata.modified().ok();
        let modified_millis = system_time_millis(modified);
        let identity = read_file_identity(&file);
        let cache_path = index_cache_path(cache_dir.as_ref(), &path);
        let Some(cached) = read_index_cache(
            &cache_path,
            &path,
            source_size,
            modified_millis,
            identity.as_ref(),
        ) else {
            return Ok(None);
        };
        let CachedIndex {
            indexed_lines: cached,
            encoding,
            ..
        } = cached;
        let source_line_count = cached.starts.len();
        if source_line_count == 0 {
            return Ok(None);
        }

        let window_count = line_limit.min(source_line_count);
        let anchor = preferred_row.min(source_line_count - 1);
        let mut start_row = anchor
            .saturating_sub(window_count / 2)
            .min(source_line_count - window_count);
        let mut end_row = start_row + window_count;
        let source_size_usize = usize::try_from(source_size)
            .with_context(|| format!("日志文件过大，无法读取缓存预览：{}", path.display()))?;
        let byte_end_for = |end_row: usize| cached.starts.get(end_row).unwrap_or(source_size_usize);
        while end_row - start_row > 1
            && byte_end_for(end_row)
                .saturating_sub(cached.starts.get(start_row).unwrap_or_default())
                > byte_limit
        {
            let rows_before_anchor = anchor - start_row;
            let rows_after_anchor = end_row - anchor - 1;
            if rows_after_anchor >= rows_before_anchor && end_row - 1 > anchor {
                end_row -= 1;
            } else if start_row < anchor {
                start_row += 1;
            } else {
                end_row -= 1;
            }
        }
        let byte_start = cached.starts.get(start_row).unwrap_or_default();
        let byte_end = byte_end_for(end_row);
        let byte_count = byte_end
            .checked_sub(byte_start)
            .with_context(|| format!("索引缓存中的预览范围无效：{}", cache_path.display()))?;
        if byte_count > byte_limit {
            return Ok(None);
        }
        file.seek(SeekFrom::Start(byte_start as u64))?;
        let mut preview = vec![0_u8; byte_count];
        file.read_exact(&mut preview)
            .with_context(|| format!("无法读取缓存日志预览：{}", path.display()))?;

        let starts = MutableLineStarts::from_native(
            (start_row..end_row)
                .map(|row_ix| {
                    cached
                        .starts
                        .get(row_ix)
                        .expect("the preview row range was bounded by the cached line count")
                })
                .map(|offset| offset - byte_start)
                .collect(),
        );
        let indexed_lines = IndexedLines {
            starts,
            longest_line_bytes: cached.longest_line_bytes,
            longest_completed_line_bytes: cached.longest_completed_line_bytes,
            longest_line_columns: cached.longest_line_columns,
            longest_completed_line_columns: cached.longest_completed_line_columns,
        };
        Ok(Some(Self::from_segment_parts(
            path,
            DocumentBytes::Owned(preview.into_boxed_slice()),
            indexed_lines,
            modified,
            source_size,
            encoding,
            start_row..source_line_count,
            None,
        )))
    }

    /// Refresh this snapshot, extending its line index only when every block
    /// of the existing source prefix still matches the digests captured during open.
    /// All other changes fall back to a complete rebuild.
    pub fn refresh(&self) -> Result<(Self, DocumentRefreshKind)> {
        if let Some(document) = self.try_refresh_appended()? {
            return Ok((document, DocumentRefreshKind::Appended));
        }

        Self::open(self.path()).map(|document| (document, DocumentRefreshKind::Rebuilt))
    }

    pub fn metadata(&self) -> &DocumentMetadata {
        &self.metadata
    }

    pub fn path(&self) -> &Path {
        &self.metadata.path
    }

    pub fn file_name(&self) -> String {
        self.path()
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path().display().to_string())
    }

    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    /// Number of logical lines in the complete source snapshot. During a
    /// cached anchor preview this can be larger than [`Self::line_count`].
    pub fn source_line_count(&self) -> usize {
        self.metadata.line_count
    }

    /// Zero-based source row represented by the first visible preview row.
    pub fn segment_start_row(&self) -> usize {
        self.source_rows
            .as_ref()
            .and_then(CompressedRows::first)
            .unwrap_or(self.segment_start_row)
    }

    pub fn source_row(&self, local_row: usize) -> Option<usize> {
        match &self.source_rows {
            Some(source_rows) => source_rows.get(local_row),
            None => (local_row < self.line_count())
                .then(|| self.segment_start_row.saturating_add(local_row)),
        }
    }

    pub fn local_row(&self, source_row: usize) -> Option<usize> {
        match &self.source_rows {
            Some(source_rows) => source_rows.position(source_row),
            None => source_row
                .checked_sub(self.segment_start_row)
                .filter(|local_row| *local_row < self.line_count()),
        }
    }

    pub fn contains_source_row(&self, source_row: usize) -> bool {
        self.local_row(source_row).is_some()
    }

    /// Whether this snapshot can resolve every logical source row.
    ///
    /// Complete open documents satisfy this. Cached previews and sparse directory-result
    /// projections do not, even when they share the complete byte identity of the source.
    pub fn has_complete_line_index(&self) -> bool {
        self.source_rows.is_none()
            && self.segment_start_row == 0
            && self.line_count() == self.source_line_count()
    }

    /// Whether both values represent the same immutable complete byte snapshot.
    ///
    /// This deliberately does not establish path identity; callers associating
    /// per-file state must match paths separately. Independently constructed
    /// partial previews are not considered equivalent because their unseen bytes
    /// were never fingerprinted. References to the exact same snapshot remain
    /// equivalent even while it is only a preview.
    pub fn same_source_snapshot(&self, other: &Self) -> bool {
        if std::ptr::eq(self, other) {
            return true;
        }
        if !self.contains_complete_source() || !other.contains_complete_source() {
            return false;
        }
        self.metadata.file_size == other.metadata.file_size
            && self.metadata.encoding_name == other.metadata.encoding_name
            && self.content_digest == other.content_digest
    }

    /// Release the retained source handle and reopen it by strong identity for future cache misses.
    ///
    /// Directory-search result snapshots use this after scanning so a large result set does not
    /// retain one operating-system file descriptor per matching file. Already verified blocks
    /// remain available from the bounded snapshot cache.
    pub fn release_source_handle(&self) {
        self.bytes.release_source_handle();
    }

    /// Create a byte-snapshot view that retains offsets only for requested source rows.
    ///
    /// Directory-search results use this so files with millions of lines and a handful of
    /// matches do not retain complete line-offset tables. The source bytes and content identity
    /// remain shared, and retained lines keep their original source row coordinates.
    pub fn project_source_rows(&self, rows: &CompressedRows) -> Self {
        let capacity = rows.len().min(self.line_count());
        let max_offset = u64::try_from(self.bytes.len()).unwrap_or(u64::MAX);
        let mut selected_rows = Vec::with_capacity(capacity);
        let mut starts = MutableLineStarts::with_capacity(max_offset, capacity);
        let mut ends = MutableLineStarts::with_capacity(max_offset, capacity);
        for source_row in rows.iter() {
            let Some(local_row) = self.local_row(source_row) else {
                continue;
            };
            let Some(mut range) = self.line_byte_range_at_local_row(local_row) else {
                continue;
            };
            if source_row == 0 {
                range.start = range.start.saturating_sub(self.encoding.bom_len());
            }
            selected_rows.push(source_row);
            starts.push(range.start);
            ends.push(range.end);
        }

        Self {
            bytes: self.bytes.clone(),
            line_starts: starts.into_immutable(),
            line_ends: Some(ends.into_immutable()),
            source_rows: Some(selected_rows.into_iter().collect()),
            segment_start_row: 0,
            metadata: self.metadata.clone(),
            longest_completed_line_bytes: self.longest_completed_line_bytes,
            longest_completed_line_columns: self.longest_completed_line_columns,
            append_fingerprint: self.append_fingerprint.clone(),
            content_digest: self.content_digest,
            encoding: self.encoding,
        }
    }

    fn contains_complete_source(&self) -> bool {
        self.segment_start_row == 0 && self.bytes.len() as u64 == self.metadata.file_size
    }

    /// Decode one logical line without retaining its text in memory.
    pub fn line(&self, source_row: usize) -> Option<String> {
        self.line_bytes(source_row)
            .map(|bytes| self.encoding.decode(&bytes))
    }

    /// Decode a bounded prefix of one logical line for interactive display.
    pub fn line_preview(&self, source_row: usize, max_bytes: usize) -> Option<LinePreview> {
        let row_ix = self.local_row(source_row)?;
        let range = self.line_byte_range_at_local_row(row_ix)?;
        let read_end = range
            .end
            .min(range.start.saturating_add(max_bytes).saturating_add(4));
        let mut bytes = self.bytes.read_range(range.start..read_end)?;
        if read_end == range.end {
            let trimmed_len = self.encoding.trim_line_bytes(&bytes).len();
            bytes.truncate(trimmed_len);
        }
        let mut preview = self.encoding.decode_preview(&bytes, max_bytes);
        preview.truncated |= read_end < range.end;
        Some(preview)
    }

    pub(crate) fn line_bytes(&self, source_row: usize) -> Option<DocumentLineBytes<'_>> {
        let row_ix = self.local_row(source_row)?;
        self.line_bytes_at_local_row(row_ix)
    }

    fn line_bytes_at_local_row(&self, row_ix: usize) -> Option<DocumentLineBytes<'_>> {
        let range = self.line_byte_range_at_local_row(row_ix)?;
        let mut bytes = self.bytes.read_range(range)?;
        let trimmed_len = self.encoding.trim_line_bytes(&bytes).len();
        bytes.truncate(trimmed_len);
        Some(bytes)
    }

    fn line_byte_range_at_local_row(&self, row_ix: usize) -> Option<std::ops::Range<usize>> {
        let mut start = self.line_starts.get(row_ix)?;
        let end = self
            .line_ends
            .as_ref()
            .and_then(|ends| ends.get(row_ix))
            .or_else(|| self.line_starts.get(row_ix + 1))
            .unwrap_or(self.bytes.len());
        if self.source_row(row_ix) == Some(0) {
            start = start.saturating_add(self.encoding.bom_len()).min(end);
        }
        Some(start..end)
    }

    pub(crate) fn search_lines(&self) -> DocumentSearchLines<'_> {
        DocumentSearchLines {
            document: self,
            verified_block: None,
        }
    }

    fn try_refresh_appended(&self) -> Result<Option<Self>> {
        if self.source_rows.is_some() {
            return Ok(None);
        }
        if !matches!(self.encoding, FileEncoding::Utf8 | FileEncoding::Utf8Bom) {
            return Ok(None);
        }
        let path = self.path();
        let file =
            File::open(path).with_context(|| format!("无法打开日志文件：{}", path.display()))?;
        let file_metadata = file
            .metadata()
            .with_context(|| format!("无法读取文件信息：{}", path.display()))?;
        let new_size = file_metadata.len();
        let old_size = self.metadata.file_size;
        if new_size <= old_size {
            return Ok(None);
        }

        let mapped = map_snapshot(&file, new_size, path)?;
        let scan_bytes = mapped.as_deref().unwrap_or_default();
        if !self.append_prefix_matches(scan_bytes) {
            return Ok(None);
        }

        let old_size = usize::try_from(old_size)
            .with_context(|| format!("日志文件过大，无法建立索引：{}", path.display()))?;
        let mut line_starts = MutableLineStarts::from_immutable(&self.line_starts);
        let (
            longest_line_bytes,
            longest_completed_line_bytes,
            longest_line_columns,
            longest_completed_line_columns,
        ) = extend_line_index(
            scan_bytes,
            old_size,
            &mut line_starts,
            self.longest_completed_line_bytes,
            self.longest_completed_line_columns,
        );
        let indexed_lines = IndexedLines {
            starts: line_starts,
            longest_line_bytes,
            longest_completed_line_bytes,
            longest_line_columns,
            longest_completed_line_columns,
        };
        let integrity_blocks: Arc<[[u8; 32]]> = calculate_integrity_blocks(scan_bytes).into();
        drop(mapped);
        let bytes = DocumentBytes::verified(
            file,
            path.to_path_buf(),
            usize::try_from(new_size)
                .with_context(|| format!("日志文件过大，无法读取：{}", path.display()))?,
            integrity_blocks.clone(),
        );

        Ok(Some(Self::from_parts(
            path.to_path_buf(),
            bytes,
            indexed_lines,
            file_metadata.modified().ok(),
            new_size,
            self.encoding,
            Some(integrity_blocks),
        )))
    }

    fn append_prefix_matches(&self, bytes: &[u8]) -> bool {
        let old_size = self.bytes.len();
        if old_size != usize::try_from(self.metadata.file_size).unwrap_or(usize::MAX)
            || bytes.len() < old_size
        {
            return false;
        }
        let tail_start = old_size.saturating_sub(self.append_fingerprint.tail.len());

        bytes.get(..self.append_fingerprint.head.len())
            == Some(self.append_fingerprint.head.as_ref())
            && bytes.get(tail_start..old_size) == Some(self.append_fingerprint.tail.as_ref())
            && integrity_blocks_match(
                &bytes[..old_size],
                self.append_fingerprint.integrity_blocks.as_ref(),
            )
    }

    fn from_parts(
        path: PathBuf,
        bytes: DocumentBytes,
        indexed_lines: IndexedLines,
        modified: Option<SystemTime>,
        source_size: u64,
        encoding: FileEncoding,
        integrity_blocks: Option<Arc<[[u8; 32]]>>,
    ) -> Self {
        let source_line_count = indexed_lines.starts.len();
        Self::from_segment_parts(
            path,
            bytes,
            indexed_lines,
            modified,
            source_size,
            encoding,
            0..source_line_count,
            integrity_blocks,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn from_segment_parts(
        path: PathBuf,
        bytes: DocumentBytes,
        indexed_lines: IndexedLines,
        modified: Option<SystemTime>,
        source_size: u64,
        encoding: FileEncoding,
        source_rows: std::ops::Range<usize>,
        integrity_blocks: Option<Arc<[[u8; 32]]>>,
    ) -> Self {
        let file_size = source_size;
        let sample_len = bytes.len().min(APPEND_SAMPLE_BYTES);
        let tail_start = bytes.len().saturating_sub(sample_len);
        let integrity_blocks = integrity_blocks.unwrap_or_else(|| {
            calculate_integrity_blocks(
                bytes
                    .resident_slice()
                    .expect("only resident previews may omit integrity blocks"),
            )
            .into()
        });
        let content_digest = digest_integrity_blocks(&integrity_blocks);
        let append_fingerprint = AppendFingerprint {
            head: bytes
                .read_unverified_range(0..sample_len)
                .unwrap_or_default()
                .into(),
            tail: bytes
                .read_unverified_range(tail_start..bytes.len())
                .unwrap_or_default()
                .into(),
            integrity_blocks,
        };
        let metadata = DocumentMetadata {
            path,
            file_size,
            modified,
            line_count: source_rows.end,
            longest_line_bytes: indexed_lines.longest_line_bytes,
            longest_line_columns: indexed_lines.longest_line_columns,
            encoding_name: encoding.name(),
        };

        Self {
            bytes: Arc::new(bytes),
            line_starts: indexed_lines.starts.into_immutable(),
            line_ends: None,
            source_rows: None,
            segment_start_row: source_rows.start,
            metadata,
            longest_completed_line_bytes: indexed_lines.longest_completed_line_bytes,
            longest_completed_line_columns: indexed_lines.longest_completed_line_columns,
            append_fingerprint,
            content_digest,
            encoding,
        }
    }
}

fn calculate_integrity_blocks(bytes: &[u8]) -> Vec<[u8; 32]> {
    calculate_integrity_blocks_while(bytes, &|| false)
        .expect("a non-cancelling integrity scan must complete")
}

fn calculate_integrity_blocks_while(
    bytes: &[u8],
    is_cancelled: &(dyn Fn() -> bool + Sync),
) -> Option<Vec<[u8; 32]>> {
    let mut blocks = Vec::with_capacity(bytes.len().div_ceil(APPEND_INTEGRITY_BLOCK_BYTES));
    for block in bytes.chunks(APPEND_INTEGRITY_BLOCK_BYTES) {
        if is_cancelled() {
            return None;
        }
        blocks.push(Sha256::digest(block).into());
    }
    (!is_cancelled()).then_some(blocks)
}

fn digest_integrity_blocks(blocks: &[[u8; 32]]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"vclogg-document-content-v1");
    digest.update((blocks.len() as u64).to_le_bytes());
    for block in blocks {
        digest.update(block);
    }
    digest.finalize().into()
}

fn build_line_index_with_integrity_while(
    bytes: &[u8],
    encoding: FileEncoding,
    is_cancelled: &(dyn Fn() -> bool + Sync),
) -> Option<(IndexedLines, Arc<[[u8; 32]]>)> {
    if is_cancelled() {
        return None;
    }
    let parallel = bytes.len() >= PARALLEL_INDEX_HASH_MIN_BYTES
        && std::thread::available_parallelism().is_ok_and(|parallelism| parallelism.get() > 1);
    if !parallel {
        let indexed_lines = build_line_index_while(bytes, encoding, is_cancelled)?;
        let integrity_blocks = calculate_integrity_blocks_while(bytes, is_cancelled)?.into();
        return Some((indexed_lines, integrity_blocks));
    }

    let (indexed_lines, integrity_blocks) = std::thread::scope(|scope| {
        let integrity_task = scope.spawn(|| calculate_integrity_blocks_while(bytes, is_cancelled));
        let indexed_lines = build_line_index_while(bytes, encoding, is_cancelled);
        let integrity_blocks = integrity_task
            .join()
            .expect("日志完整性摘要线程不应异常终止");
        (indexed_lines, integrity_blocks)
    });
    Some((indexed_lines?, integrity_blocks?.into()))
}

fn integrity_blocks_match(bytes: &[u8], expected: &[[u8; 32]]) -> bool {
    bytes.len().div_ceil(APPEND_INTEGRITY_BLOCK_BYTES) == expected.len()
        && bytes
            .chunks(APPEND_INTEGRITY_BLOCK_BYTES)
            .zip(expected)
            .all(|(block, expected)| <[u8; 32]>::from(Sha256::digest(block)) == *expected)
}

fn map_snapshot(file: &File, file_size: u64, path: &Path) -> Result<Option<Mmap>> {
    if file_size == 0 {
        return Ok(None);
    }
    let mapped_len = usize::try_from(file_size)
        .with_context(|| format!("日志文件过大，无法映射：{}", path.display()))?;

    // SAFETY: The mapping is read-only, has the metadata length captured above,
    // and remains local to background index construction. Installed documents
    // use verified positional reads instead of retaining this mapping.
    unsafe { MmapOptions::new().len(mapped_len).map(file) }
        .map(Some)
        .with_context(|| format!("无法映射日志文件：{}", path.display()))
}

fn index_cache_path(cache_dir: &Path, source_path: &Path) -> PathBuf {
    let source_identity = index_cache_source_identity(source_path);
    let digest = Sha256::digest(&source_identity);
    let mut hash = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(&mut hash, "{byte:02x}").expect("writing to String cannot fail");
    }
    cache_dir.join(format!("{hash}.vclog-index"))
}

fn index_cache_source_identity(source_path: &Path) -> Vec<u8> {
    #[cfg(unix)]
    {
        let path = source_path.as_os_str().as_bytes();
        let mut identity = Vec::with_capacity(path.len() + 1);
        identity.push(b'U');
        identity.extend_from_slice(path);
        identity
    }
    #[cfg(windows)]
    {
        let mut identity = Vec::new();
        identity.push(b'W');
        identity.extend(
            source_path
                .as_os_str()
                .encode_wide()
                .flat_map(u16::to_le_bytes),
        );
        identity
    }
    #[cfg(not(any(unix, windows)))]
    {
        let path = source_path.to_string_lossy();
        let mut identity = Vec::with_capacity(path.len() + 1);
        identity.push(b'O');
        identity.extend_from_slice(path.as_bytes());
        identity
    }
}

fn system_time_millis(time: Option<SystemTime>) -> u64 {
    time.and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

fn read_index_cache(
    cache_path: &Path,
    source_path: &Path,
    file_size: u64,
    modified_millis: u64,
    identity: Option<&FileIdentity>,
) -> Option<CachedIndex> {
    read_index_cache_while(
        cache_path,
        source_path,
        file_size,
        modified_millis,
        identity,
        &|| false,
    )
}

fn read_index_cache_while(
    cache_path: &Path,
    source_path: &Path,
    file_size: u64,
    modified_millis: u64,
    identity: Option<&FileIdentity>,
    is_cancelled: &dyn Fn() -> bool,
) -> Option<CachedIndex> {
    if is_cancelled() {
        return None;
    }
    let identity = identity?;
    let file = File::open(cache_path).ok()?;
    let cache_size = file.metadata().ok()?.len();
    let mut reader = BufReader::new(file);
    let mut magic = [0_u8; 8];
    reader.read_exact(&mut magic).ok()?;
    if &magic != INDEX_CACHE_MAGIC || read_u32(&mut reader)? != INDEX_CACHE_VERSION {
        return None;
    }
    let path_len = usize::try_from(read_u32(&mut reader)?).ok()?;
    if path_len > MAX_CACHE_PATH_BYTES {
        return None;
    }
    let encoding_len = usize::from(read_u16(&mut reader)?);
    if encoding_len == 0 || encoding_len > MAX_CACHE_ENCODING_BYTES {
        return None;
    }
    let cached_file_size = read_u64(&mut reader)?;
    let cached_modified_millis = read_u64(&mut reader)?;
    let has_identity = read_byte(&mut reader)? == 1;
    let cached_volume_serial = read_u64(&mut reader)?;
    let mut cached_file_id = [0_u8; 16];
    reader.read_exact(&mut cached_file_id).ok()?;
    let cached_usn = read_i64(&mut reader)?;
    let line_count = usize::try_from(read_u64(&mut reader)?).ok()?;
    let encoded_offsets_len = usize::try_from(read_u64(&mut reader)?).ok()?;
    let integrity_block_count = usize::try_from(read_u64(&mut reader)?).ok()?;
    let longest_line_bytes = usize::try_from(read_u64(&mut reader)?).ok()?;
    let longest_completed_line_bytes = usize::try_from(read_u64(&mut reader)?).ok()?;
    let longest_line_columns = usize::try_from(read_u64(&mut reader)?).ok()?;
    let longest_completed_line_columns = usize::try_from(read_u64(&mut reader)?).ok()?;
    let expected_size = INDEX_CACHE_HEADER_BYTES
        .checked_add(u64::try_from(path_len).ok()?)?
        .checked_add(u64::try_from(encoding_len).ok()?)?
        .checked_add(u64::try_from(encoded_offsets_len).ok()?)?
        .checked_add(u64::try_from(integrity_block_count).ok()?.checked_mul(32)?)?;
    let expected_integrity_block_count =
        usize::try_from(file_size.div_ceil(u64::try_from(APPEND_INTEGRITY_BLOCK_BYTES).ok()?))
            .ok()?;
    if cache_size != expected_size
        || cached_file_size != file_size
        || cached_modified_millis != modified_millis
        || !has_identity
        || cached_volume_serial != identity.volume_serial
        || cached_file_id != identity.file_id
        || cached_usn != identity.usn
        || integrity_block_count != expected_integrity_block_count
    {
        return None;
    }
    if (file_size == 0 && line_count != 0)
        || (file_size > 0 && (line_count == 0 || u64::try_from(line_count).ok()? > file_size + 1))
        || encoded_offsets_len > line_count.saturating_mul(10)
        || (line_count == 0 && encoded_offsets_len != 0)
        || u64::try_from(longest_line_bytes).ok()? > file_size
        || longest_completed_line_bytes > longest_line_bytes
        || u64::try_from(longest_line_columns).ok()? > file_size.saturating_mul(8)
        || longest_completed_line_columns > longest_line_columns
    {
        return None;
    }
    let mut cached_path = vec![0_u8; path_len];
    reader.read_exact(&mut cached_path).ok()?;
    if cached_path != index_cache_source_identity(source_path) {
        return None;
    }
    let mut cached_encoding = vec![0_u8; encoding_len];
    reader.read_exact(&mut cached_encoding).ok()?;
    let encoding = FileEncoding::from_cache_name(&cached_encoding)?;

    let mut starts = MutableLineStarts::with_capacity(file_size, line_count);
    let mut previous = 0_u64;
    let mut consumed = 0_usize;
    for line_ix in 0..line_count {
        if line_ix % CACHE_CANCELLATION_BATCH_LINES == 0 && is_cancelled() {
            return None;
        }
        let delta = read_varint(&mut reader, &mut consumed)?;
        if (line_ix == 0 && delta != 0) || (line_ix > 0 && delta == 0) {
            return None;
        }
        previous = previous.checked_add(delta)?;
        starts.push(usize::try_from(previous).ok()?);
    }
    if consumed != encoded_offsets_len {
        return None;
    }
    if (!starts.is_empty() && starts.get(0) != Some(0))
        || starts
            .last()
            .is_some_and(|offset| u64::try_from(offset).map_or(true, |offset| offset > file_size))
    {
        return None;
    }

    let mut integrity_blocks = Vec::with_capacity(integrity_block_count);
    for _ in 0..integrity_block_count {
        if is_cancelled() {
            return None;
        }
        let mut digest = [0_u8; 32];
        reader.read_exact(&mut digest).ok()?;
        integrity_blocks.push(digest);
    }

    (!is_cancelled()).then_some(CachedIndex {
        indexed_lines: IndexedLines {
            starts,
            longest_line_bytes,
            longest_completed_line_bytes,
            longest_line_columns,
            longest_completed_line_columns,
        },
        encoding,
        integrity_blocks: integrity_blocks.into(),
    })
}

fn write_index_cache(pending: &PendingIndexCacheWrite) -> Result<()> {
    let source_path = index_cache_source_identity(&pending.source_path);
    let path_len = u32::try_from(source_path.len()).context("索引缓存路径过长")?;
    let encoding_name = pending.encoding.name();
    let encoding_name = encoding_name.as_bytes();
    let encoding_len = u16::try_from(encoding_name.len()).context("索引缓存编码名称过长")?;
    let encoded_offsets_len = encoded_offsets_len(&pending.line_starts);
    let Some(parent) = pending.cache_path.parent() else {
        anyhow::bail!("索引缓存路径没有父目录")
    };
    fs::create_dir_all(parent)
        .with_context(|| format!("无法创建索引缓存目录：{}", parent.display()))?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let temporary = pending
        .cache_path
        .with_extension(format!("tmp-{}-{nonce}", std::process::id()));
    let result = (|| -> Result<()> {
        let file = File::create(&temporary)
            .with_context(|| format!("无法创建索引暂存文件：{}", temporary.display()))?;
        let mut writer = BufWriter::new(file);
        writer.write_all(INDEX_CACHE_MAGIC)?;
        writer.write_all(&INDEX_CACHE_VERSION.to_le_bytes())?;
        writer.write_all(&path_len.to_le_bytes())?;
        writer.write_all(&encoding_len.to_le_bytes())?;
        writer.write_all(&pending.file_size.to_le_bytes())?;
        writer.write_all(&pending.modified_millis.to_le_bytes())?;
        writer.write_all(&[u8::from(pending.identity.is_some())])?;
        if let Some(identity) = &pending.identity {
            writer.write_all(&identity.volume_serial.to_le_bytes())?;
            writer.write_all(&identity.file_id)?;
            writer.write_all(&identity.usn.to_le_bytes())?;
        } else {
            writer.write_all(&0_u64.to_le_bytes())?;
            writer.write_all(&[0_u8; 16])?;
            writer.write_all(&0_i64.to_le_bytes())?;
        }
        writer.write_all(&(pending.line_starts.len() as u64).to_le_bytes())?;
        writer.write_all(&(encoded_offsets_len as u64).to_le_bytes())?;
        writer.write_all(&(pending.integrity_blocks.len() as u64).to_le_bytes())?;
        writer.write_all(&(pending.longest_line_bytes as u64).to_le_bytes())?;
        writer.write_all(&(pending.longest_completed_line_bytes as u64).to_le_bytes())?;
        writer.write_all(&(pending.longest_line_columns as u64).to_le_bytes())?;
        writer.write_all(&(pending.longest_completed_line_columns as u64).to_le_bytes())?;
        writer.write_all(&source_path)?;
        writer.write_all(encoding_name)?;
        let mut previous = 0_u64;
        for offset in pending.line_starts.iter() {
            let offset = offset as u64;
            write_varint(&mut writer, offset.saturating_sub(previous))?;
            previous = offset;
        }
        for digest in pending.integrity_blocks.iter() {
            writer.write_all(digest)?;
        }
        writer.flush()?;
        writer.get_ref().sync_all()?;
        drop(writer);
        replace_file_atomically(&temporary, &pending.cache_path)?;
        Ok(())
    })();
    if result.is_err() {
        _ = fs::remove_file(&temporary);
    }
    result
}

fn read_byte(reader: &mut impl Read) -> Option<u8> {
    let mut bytes = [0_u8; 1];
    reader.read_exact(&mut bytes).ok()?;
    Some(bytes[0])
}

fn read_u32(reader: &mut impl Read) -> Option<u32> {
    let mut bytes = [0_u8; 4];
    reader.read_exact(&mut bytes).ok()?;
    Some(u32::from_le_bytes(bytes))
}

fn read_u16(reader: &mut impl Read) -> Option<u16> {
    let mut bytes = [0_u8; 2];
    reader.read_exact(&mut bytes).ok()?;
    Some(u16::from_le_bytes(bytes))
}

fn read_u64(reader: &mut impl Read) -> Option<u64> {
    let mut bytes = [0_u8; 8];
    reader.read_exact(&mut bytes).ok()?;
    Some(u64::from_le_bytes(bytes))
}

fn read_i64(reader: &mut impl Read) -> Option<i64> {
    let mut bytes = [0_u8; 8];
    reader.read_exact(&mut bytes).ok()?;
    Some(i64::from_le_bytes(bytes))
}

fn encoded_offsets_len(offsets: &LineStarts) -> usize {
    let mut previous = 0_u64;
    offsets.iter().fold(0, |total, offset| {
        let offset = offset as u64;
        let delta = offset.saturating_sub(previous);
        previous = offset;
        total + varint_len(delta)
    })
}

fn varint_len(mut value: u64) -> usize {
    let mut len = 1;
    while value >= 0x80 {
        value >>= 7;
        len += 1;
    }
    len
}

fn write_varint(writer: &mut impl Write, mut value: u64) -> std::io::Result<()> {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        writer.write_all(&[byte])?;
        if value == 0 {
            return Ok(());
        }
    }
}

fn read_varint(reader: &mut impl Read, consumed: &mut usize) -> Option<u64> {
    let mut value = 0_u64;
    let mut shift = 0;
    loop {
        let byte = read_byte(reader)?;
        *consumed = consumed.checked_add(1)?;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some(value);
        }
        shift += 7;
        if shift >= 64 {
            return None;
        }
    }
}

#[cfg(windows)]
fn replace_file_atomically(source: &Path, destination: &Path) -> std::io::Result<()> {
    let mut source_wide = source.as_os_str().encode_wide().collect::<Vec<_>>();
    source_wide.push(0);
    let mut destination_wide = destination.as_os_str().encode_wide().collect::<Vec<_>>();
    destination_wide.push(0);
    let replaced = unsafe {
        MoveFileExW(
            source_wide.as_ptr(),
            destination_wide.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if replaced == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn replace_file_atomically(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(windows)]
fn read_file_identity(file: &File) -> Option<FileIdentity> {
    #[repr(C)]
    struct ReadFileUsnData {
        min_major_version: u16,
        max_major_version: u16,
    }

    let handle = file.as_raw_handle();
    let mut file_id = FILE_ID_INFO::default();
    let file_id_ok = unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileIdInfo,
            (&raw mut file_id).cast(),
            std::mem::size_of::<FILE_ID_INFO>() as u32,
        )
    } != 0;
    if !file_id_ok {
        return None;
    }

    let versions = ReadFileUsnData {
        min_major_version: 2,
        max_major_version: 3,
    };
    let mut output = [0_u8; 512];
    let mut returned = 0_u32;
    let usn_ok = unsafe {
        DeviceIoControl(
            handle,
            FSCTL_READ_FILE_USN_DATA,
            (&raw const versions).cast(),
            std::mem::size_of::<ReadFileUsnData>() as u32,
            output.as_mut_ptr().cast(),
            output.len() as u32,
            &raw mut returned,
            std::ptr::null_mut(),
        )
    } != 0;
    if !usn_ok || returned < 32 {
        return None;
    }
    let major = u16::from_le_bytes([output[4], output[5]]);
    let usn_offset = match major {
        2 => 24,
        3 => 40,
        _ => return None,
    };
    if (returned as usize) < usn_offset + 8 {
        return None;
    }
    let usn = i64::from_le_bytes(output[usn_offset..usn_offset + 8].try_into().ok()?);
    Some(FileIdentity {
        volume_serial: file_id.VolumeSerialNumber,
        file_id: file_id.FileId.Identifier,
        usn,
    })
}

#[cfg(unix)]
fn read_file_identity(file: &File) -> Option<FileIdentity> {
    let metadata = file.metadata().ok()?;
    let mut file_id = [0_u8; 16];
    file_id[..8].copy_from_slice(&metadata.ino().to_le_bytes());
    file_id[8..].copy_from_slice(&metadata.ctime_nsec().to_le_bytes());
    Some(FileIdentity {
        volume_serial: metadata.dev(),
        file_id,
        usn: metadata.ctime(),
    })
}

fn looks_like_binary(sample: &[u8]) -> bool {
    if sample.contains(&0) {
        return true;
    }
    let suspicious_controls = sample
        .iter()
        .filter(|byte| **byte < 0x20 && !matches!(**byte, b'\t' | b'\n' | b'\r'))
        .count();
    suspicious_controls.saturating_mul(20) > sample.len()
}

fn has_known_binary_signature(sample: &[u8]) -> bool {
    sample.starts_with(b"\x89PNG\r\n\x1a\n")
        || sample.starts_with(b"\xff\xd8\xff")
        || sample.starts_with(b"GIF87a")
        || sample.starts_with(b"GIF89a")
        || sample.starts_with(b"BM")
        || sample.starts_with(b"\x00\x00\x01\x00")
        || sample.starts_with(b"\x00\x00\x02\x00")
        || sample.starts_with(b"PK\x03\x04")
        || sample.starts_with(b"%PDF-")
        || sample.starts_with(b"\x1a\x45\xdf\xa3")
        || (sample.len() >= 12 && &sample[4..8] == b"ftyp")
        || (sample.len() >= 12 && sample.starts_with(b"RIFF") && &sample[8..12] == b"WEBP")
}

fn detect_file_encoding(file: &mut File, byte_size: u64) -> std::io::Result<FileEncoding> {
    let sample = read_encoding_sample(file, byte_size)?;
    let encoding = detect_file_encoding_from_sample(&sample, byte_size);
    file.seek(SeekFrom::Start(0))?;
    Ok(encoding)
}

fn detect_file_encoding_with_sample(
    file: &mut File,
    byte_size: u64,
) -> std::io::Result<(FileEncoding, Vec<u8>)> {
    let sample = read_encoding_sample(file, byte_size)?;
    let encoding = detect_file_encoding_from_sample(&sample, byte_size);
    Ok((encoding, sample))
}

fn read_encoding_sample(file: &mut File, byte_size: u64) -> std::io::Result<Vec<u8>> {
    file.seek(SeekFrom::Start(0))?;
    let sample_len = usize::try_from(byte_size.min(ENCODING_DETECTION_BYTES as u64))
        .unwrap_or(ENCODING_DETECTION_BYTES);
    let mut sample = vec![0_u8; sample_len];
    file.read_exact(&mut sample)?;
    Ok(sample)
}

fn detect_file_encoding_from_sample(sample: &[u8], byte_size: u64) -> FileEncoding {
    if sample.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return FileEncoding::Utf8Bom;
    }
    if sample.starts_with(&[0xFF, 0xFE]) {
        return FileEncoding::Utf16Le;
    }
    if sample.starts_with(&[0xFE, 0xFF]) {
        return FileEncoding::Utf16Be;
    }
    if has_known_binary_signature(sample) {
        return FileEncoding::Binary;
    }

    let pairs = sample.len() / 2;
    if pairs >= 4 {
        let even_zeroes = sample
            .iter()
            .step_by(2)
            .take(pairs)
            .filter(|byte| **byte == 0)
            .count();
        let odd_zeroes = sample
            .iter()
            .skip(1)
            .step_by(2)
            .take(pairs)
            .filter(|byte| **byte == 0)
            .count();
        let le_newlines = sample
            .as_chunks::<2>()
            .0
            .iter()
            .filter(|pair| **pair == [b'\n', 0])
            .count();
        let be_newlines = sample
            .as_chunks::<2>()
            .0
            .iter()
            .filter(|pair| **pair == [0, b'\n'])
            .count();
        let le_signal = le_newlines > be_newlines && le_newlines.saturating_mul(200) > pairs;
        let be_signal = be_newlines > le_newlines && be_newlines.saturating_mul(200) > pairs;
        if le_signal || (odd_zeroes * 3 > pairs && even_zeroes * 20 < pairs) {
            return FileEncoding::Utf16Le;
        }
        if be_signal || (even_zeroes * 3 > pairs && odd_zeroes * 20 < pairs) {
            return FileEncoding::Utf16Be;
        }
    }
    if looks_like_binary(sample) {
        return FileEncoding::Binary;
    }
    match std::str::from_utf8(sample) {
        Ok(_) => return FileEncoding::Utf8,
        Err(error) if error.error_len().is_none() => return FileEncoding::Utf8,
        Err(_) => {}
    }
    let mut detector = EncodingDetector::new();
    detector.feed(sample, sample.len() as u64 == byte_size);
    let detected = detector.guess(None, true);
    if detected == UTF_8 {
        FileEncoding::Utf8
    } else {
        FileEncoding::Legacy(detected)
    }
}

fn for_each_line_break(
    encoding: FileEncoding,
    bytes: &[u8],
    mut operation: impl FnMut(usize, usize) -> ControlFlow<()>,
) -> ControlFlow<()> {
    match encoding {
        FileEncoding::Binary => {}
        FileEncoding::Utf16Le | FileEncoding::Utf16Be => {
            let mut position = 0;
            while position + 1 < bytes.len() {
                let code_unit = match encoding {
                    FileEncoding::Utf16Le => {
                        u16::from_le_bytes([bytes[position], bytes[position + 1]])
                    }
                    FileEncoding::Utf16Be => {
                        u16::from_be_bytes([bytes[position], bytes[position + 1]])
                    }
                    _ => unreachable!(),
                };
                if code_unit == u16::from(b'\r') {
                    let followed_by_lf = position + 3 < bytes.len()
                        && match encoding {
                            FileEncoding::Utf16Le => {
                                bytes[position + 2] == b'\n' && bytes[position + 3] == 0
                            }
                            FileEncoding::Utf16Be => {
                                bytes[position + 2] == 0 && bytes[position + 3] == b'\n'
                            }
                            _ => false,
                        };
                    let width = if followed_by_lf { 4 } else { 2 };
                    operation(position, width)?;
                    position += width;
                } else if code_unit == u16::from(b'\n') {
                    operation(position, 2)?;
                    position += 2;
                } else {
                    position += 2;
                }
            }
        }
        FileEncoding::Utf8 | FileEncoding::Utf8Bom | FileEncoding::Legacy(_) => {
            let mut search_start = 0;
            while let Some(relative) = memchr2(b'\r', b'\n', &bytes[search_start..]) {
                let position = search_start + relative;
                let width = usize::from(
                    bytes[position] == b'\r' && bytes.get(position + 1) == Some(&b'\n'),
                ) + 1;
                operation(position, width)?;
                search_start = position + width;
            }
        }
    }
    ControlFlow::Continue(())
}

fn preview_visible_len(bytes: &[u8], encoding: FileEncoding, line_limit: usize) -> usize {
    if line_limit == 0 {
        return 0;
    }
    if matches!(encoding, FileEncoding::Binary) {
        return bytes
            .len()
            .min(line_limit.saturating_mul(BINARY_BYTES_PER_LINE));
    }
    let mut breaks = 0;
    let mut visible_len = bytes.len();
    _ = for_each_line_break(encoding, bytes, |position, width| {
        breaks += 1;
        if breaks == line_limit {
            visible_len = position + width;
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    });
    visible_len
}

fn build_line_index(bytes: &[u8], encoding: FileEncoding) -> IndexedLines {
    build_line_index_while(bytes, encoding, &|| false)
        .expect("a non-cancelling line-index scan must complete")
}

fn build_line_index_while(
    bytes: &[u8],
    encoding: FileEncoding,
    is_cancelled: &dyn Fn() -> bool,
) -> Option<IndexedLines> {
    if is_cancelled() {
        return None;
    }
    if bytes.is_empty() {
        return Some(IndexedLines {
            starts: MutableLineStarts::Compact(Vec::new()),
            longest_line_bytes: 0,
            longest_completed_line_bytes: 0,
            longest_line_columns: 0,
            longest_completed_line_columns: 0,
        });
    }

    if matches!(encoding, FileEncoding::Binary) {
        let line_count = bytes.len().div_ceil(BINARY_BYTES_PER_LINE);
        let mut starts = MutableLineStarts::with_capacity(bytes.len() as u64, line_count);
        for line_ix in 0..line_count {
            if line_ix % INDEX_CANCELLATION_BATCH_LINES == 0 && is_cancelled() {
                return None;
            }
            starts.push(line_ix.saturating_mul(BINARY_BYTES_PER_LINE));
        }
        let longest_line_bytes = bytes.len().min(BINARY_BYTES_PER_LINE);
        let longest_line_columns = longest_line_bytes.saturating_mul(3).saturating_sub(1);
        return Some(IndexedLines {
            starts,
            longest_line_bytes,
            longest_completed_line_bytes: longest_line_bytes,
            longest_line_columns,
            longest_completed_line_columns: longest_line_columns,
        });
    }

    let mut line_starts = MutableLineStarts::with_capacity(bytes.len() as u64, 0);
    line_starts.push(0);
    let mut longest_completed_line_bytes = 0;
    let mut longest_completed_line_columns = 0;
    let mut current_start = 0;
    let mut indexed_lines = 0;
    let mut cancelled = false;

    _ = for_each_line_break(encoding, bytes, |line_end, width| {
        if indexed_lines % INDEX_CANCELLATION_BATCH_LINES == 0 && is_cancelled() {
            cancelled = true;
            return ControlFlow::Break(());
        }
        indexed_lines += 1;
        longest_completed_line_bytes =
            longest_completed_line_bytes.max(line_end.saturating_sub(current_start));
        longest_completed_line_columns =
            longest_completed_line_columns.max(display_columns_encoded(
                encoding,
                &bytes[current_start..line_end],
                current_start == 0,
            ));

        current_start = line_end + width;
        if current_start <= bytes.len() && line_starts.last() != Some(current_start) {
            line_starts.push(current_start);
        }
        ControlFlow::Continue(())
    });
    if cancelled || is_cancelled() {
        return None;
    }

    let trailing_line_bytes = if current_start < bytes.len() {
        bytes.len() - current_start
    } else {
        0
    };
    let longest_line_bytes = longest_completed_line_bytes.max(trailing_line_bytes);
    let trailing_line_columns = if current_start < bytes.len() {
        display_columns_encoded(encoding, &bytes[current_start..], current_start == 0)
    } else {
        0
    };
    let longest_line_columns = longest_completed_line_columns.max(trailing_line_columns);

    Some(IndexedLines {
        starts: line_starts,
        longest_line_bytes,
        longest_completed_line_bytes,
        longest_line_columns,
        longest_completed_line_columns,
    })
}

fn display_columns_encoded(encoding: FileEncoding, bytes: &[u8], first_line: bool) -> usize {
    let bytes = if first_line {
        &bytes[encoding.bom_len().min(bytes.len())..]
    } else {
        bytes
    };
    match encoding {
        FileEncoding::Utf8 | FileEncoding::Utf8Bom => display_columns(bytes, false),
        FileEncoding::Utf16Le | FileEncoding::Utf16Be | FileEncoding::Legacy(_) => encoding
            .decode(bytes)
            .chars()
            .fold(0usize, |columns, character| match character {
                '\t' => columns.saturating_add(8 - columns % 8),
                '\r' => columns,
                _ => columns.saturating_add(1),
            }),
        FileEncoding::Binary => bytes.len().saturating_mul(3).saturating_sub(1),
    }
}

fn extend_line_index(
    bytes: &[u8],
    old_size: usize,
    line_starts: &mut MutableLineStarts,
    mut longest_completed_line_bytes: usize,
    mut longest_completed_line_columns: usize,
) -> (usize, usize, usize, usize) {
    let mut current_start = if old_size == 0 {
        line_starts.push(0);
        0
    } else if bytes.get(old_size - 1) == Some(&b'\n') {
        if line_starts.last() != Some(old_size) {
            line_starts.push(old_size);
        }
        old_size
    } else {
        line_starts.last().unwrap_or_default()
    };

    for relative_newline_ix in memchr_iter(b'\n', &bytes[old_size..]) {
        let newline_ix = old_size + relative_newline_ix;
        let line_end = if newline_ix > current_start && bytes[newline_ix - 1] == b'\r' {
            newline_ix - 1
        } else {
            newline_ix
        };
        longest_completed_line_bytes =
            longest_completed_line_bytes.max(line_end.saturating_sub(current_start));
        longest_completed_line_columns = longest_completed_line_columns.max(display_columns(
            &bytes[current_start..line_end],
            current_start == 0,
        ));

        current_start = newline_ix + 1;
        if current_start <= bytes.len() && line_starts.last() != Some(current_start) {
            line_starts.push(current_start);
        }
    }

    let trailing_line_bytes = if current_start < bytes.len() {
        bytes.len() - current_start
    } else {
        0
    };
    let longest_line_bytes = longest_completed_line_bytes.max(trailing_line_bytes);
    let trailing_line_columns = if current_start < bytes.len() {
        display_columns(&bytes[current_start..], current_start == 0)
    } else {
        0
    };
    let longest_line_columns = longest_completed_line_columns.max(trailing_line_columns);

    (
        longest_line_bytes,
        longest_completed_line_bytes,
        longest_line_columns,
        longest_completed_line_columns,
    )
}

fn display_columns(bytes: &[u8], first_line: bool) -> usize {
    let bytes = if first_line && bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        &bytes[3..]
    } else {
        bytes
    };
    bytes.iter().fold(0usize, |columns, byte| match byte {
        b'\t' => columns.saturating_add(8 - columns % 8),
        b'\r' => columns,
        _ => columns.saturating_add(1),
    })
}

#[cfg(test)]
mod source_snapshot_tests {
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicUsize, Ordering},
        time::SystemTime,
    };

    use crate::CancellationToken;

    use super::{DocumentBytes, FileEncoding, LogDocument, build_line_index_while};

    fn test_directory(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("系统时间应晚于 Unix 纪元")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "vclogg2-source-snapshot-{label}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&directory).expect("应能创建文档快照测试目录");
        directory
    }

    #[test]
    fn pre_cancelled_open_avoids_source_io() {
        let cancellation = CancellationToken::default();
        cancellation.cancel();
        let missing = PathBuf::from("this-source-does-not-need-to-exist.log");

        let document = LogDocument::open_cancellable(&missing, &cancellation)
            .expect("pre-cancelled opening should not fail");
        let cached = LogDocument::open_with_index_cache_cancellable(
            &missing,
            PathBuf::from("unused-cache"),
            &cancellation,
        )
        .expect("pre-cancelled cached opening should not fail");

        assert!(document.is_none());
        assert!(cached.is_none());
    }

    #[test]
    fn line_index_construction_polls_cancellation() {
        let bytes = b"line\n".repeat(10_000);
        let checks = AtomicUsize::new(0);

        let indexed = build_line_index_while(&bytes, FileEncoding::Utf8, &|| {
            checks.fetch_add(1, Ordering::Relaxed) >= 1
        });

        assert!(indexed.is_none());
        assert!(checks.load(Ordering::Relaxed) >= 2);
    }

    #[test]
    fn complete_documents_opened_independently_share_snapshot_identity() {
        let directory = test_directory("same");
        let path = directory.join("source.log");
        fs::write(&path, b"alpha\nbeta\n").expect("应能写入测试日志");

        let first = LogDocument::open(&path).expect("应能首次打开测试日志");
        let second = LogDocument::open(&path).expect("应能再次打开测试日志");

        assert!(first.same_source_snapshot(&second));
        _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn same_sized_but_different_contents_have_distinct_snapshot_identity() {
        let directory = test_directory("different");
        let first_path = directory.join("first.log");
        let second_path = directory.join("second.log");
        fs::write(&first_path, b"alpha\n").expect("应能写入第一份测试日志");
        fs::write(&second_path, b"omega\n").expect("应能写入第二份测试日志");

        let first = LogDocument::open(&first_path).expect("应能打开第一份测试日志");
        let second = LogDocument::open(&second_path).expect("应能打开第二份测试日志");

        assert_eq!(first.metadata().file_size, second.metadata().file_size);
        assert!(!first.same_source_snapshot(&second));
        _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn unread_blocks_reject_external_overwrites() {
        let directory = test_directory("external-overwrite");
        let path = directory.join("source.log");
        fs::write(&path, b"alpha\nbeta\n").expect("应能写入原始测试日志");
        let document = LogDocument::open(&path).expect("应能打开原始测试日志");

        fs::write(&path, b"omega\nbeta\n").expect("应能原地覆盖测试日志");

        assert_eq!(document.line(0), None);
        _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn verified_blocks_preserve_bytes_after_external_overwrite() {
        let directory = test_directory("cached-external-overwrite");
        let path = directory.join("source.log");
        fs::write(&path, b"alpha\nbeta\n").expect("应能写入原始测试日志");
        let document = LogDocument::open(&path).expect("应能打开原始测试日志");
        assert_eq!(document.line(0).as_deref(), Some("alpha"));

        fs::write(&path, b"omega\nbeta\n").expect("应能原地覆盖测试日志");

        assert_eq!(document.line(0).as_deref(), Some("alpha"));
        assert_eq!(document.line(1).as_deref(), Some("beta"));
        _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn released_source_handles_reopen_only_the_original_file_identity() {
        let directory = test_directory("released-source-handle");
        let path = directory.join("source.log");
        fs::write(&path, b"alpha\nbeta\n").expect("应能写入原始测试日志");
        let document = LogDocument::open(&path).expect("应能打开原始测试日志");

        document.release_source_handle();
        assert_eq!(document.line(0).as_deref(), Some("alpha"));
        let DocumentBytes::Verified(bytes) = document.bytes.as_ref() else {
            panic!("完整文档应使用校验块存储");
        };
        assert_eq!(
            bytes
                .state
                .lock()
                .expect("校验块缓存锁不应中毒")
                .cached_bytes,
            0,
            "瞬时句柄文档不应长期保留原始文件块"
        );

        let replaced_path = directory.join("replacement.log");
        fs::write(&replaced_path, b"first\nsecond\n").expect("应能写入待替换测试日志");
        let replaced = LogDocument::open(&replaced_path).expect("应能打开待替换测试日志");
        replaced.release_source_handle();
        fs::rename(&replaced_path, directory.join("original.log"))
            .expect("释放句柄后应能移走原始日志");
        fs::write(&replaced_path, b"first\nsecond\n").expect("应能写入同内容替换日志");

        assert_eq!(replaced.line(0), None);
        _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn sparse_source_row_views_keep_only_requested_line_offsets() {
        let directory = test_directory("sparse-source-rows");
        let path = directory.join("source.log");
        fs::write(&path, b"\xef\xbb\xbfalpha\nbeta\ngamma\n").expect("应能写入稀疏源行测试日志");
        let document = LogDocument::open(&path).expect("应能打开稀疏源行测试日志");
        let rows = [0, 2, usize::MAX].into_iter().collect();

        let projected = document.project_source_rows(&rows);

        assert_eq!(projected.line_count(), 2);
        assert_eq!(projected.source_line_count(), document.source_line_count());
        assert_eq!(projected.source_row(0), Some(0));
        assert_eq!(projected.source_row(1), Some(2));
        assert_eq!(projected.local_row(2), Some(1));
        assert_eq!(projected.line(0).as_deref(), Some("alpha"));
        assert_eq!(projected.line(1), None);
        assert_eq!(projected.line(2).as_deref(), Some("gamma"));
        assert!(projected.same_source_snapshot(&document));
        assert_eq!(projected.line_ends.as_ref().map(|ends| ends.len()), Some(2));
        _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn unread_blocks_handle_external_truncation_without_mapping_access() {
        let directory = test_directory("external-truncate");
        let path = directory.join("source.log");
        fs::write(&path, b"alpha\nbeta\n").expect("应能写入原始测试日志");
        let document = LogDocument::open(&path).expect("应能打开原始测试日志");

        fs::File::create(&path).expect("应能截断测试日志");

        assert_eq!(document.line(0), None);
        _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn independently_opened_partial_previews_are_not_assumed_equivalent() {
        let directory = test_directory("preview");
        let path = directory.join("source.log");
        fs::write(&path, b"alpha\nbeta\n").expect("应能写入测试日志");

        let (first, first_complete) =
            LogDocument::open_preview(&path, 6, 1).expect("应能打开第一份日志预览");
        let (second, second_complete) =
            LogDocument::open_preview(&path, 6, 1).expect("应能打开第二份日志预览");

        assert!(!first_complete);
        assert!(!second_complete);
        assert!(first.same_source_snapshot(&first));
        assert!(!first.same_source_snapshot(&second));
        _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn cached_preview_maps_source_rows_to_local_virtual_rows() {
        let directory = test_directory("row-coordinates");
        let path = directory.join("source.log");
        let cache_directory = directory.join("cache");
        fs::write(&path, b"zero\none\ntwo\nthree").expect("应能写入测试日志");
        let (_, pending_cache) = LogDocument::open_with_index_cache(&path, &cache_directory)
            .expect("应能为测试日志建立索引缓存");
        pending_cache
            .expect("首次打开应生成待写入的索引缓存")
            .persist()
            .expect("应能写入测试索引缓存");

        let preview = LogDocument::open_cached_preview(&path, &cache_directory, 2, 1, 1024)
            .expect("应能读取缓存预览")
            .expect("缓存预览应存在");

        assert_eq!(preview.segment_start_row(), 2);
        assert_eq!(preview.source_row(0), Some(2));
        assert_eq!(preview.local_row(2), Some(0));
        assert_eq!(preview.local_row(1), None);
        assert_eq!(preview.local_row(3), None);
        _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn cached_preview_never_reads_an_anchor_line_beyond_the_byte_budget() {
        let directory = test_directory("bounded-cached-preview");
        let path = directory.join("source.log");
        let cache_directory = directory.join("cache");
        fs::write(&path, b"short\nthis line is much too large\ntail").expect("应能写入测试日志");
        let (_, pending_cache) = LogDocument::open_with_index_cache(&path, &cache_directory)
            .expect("应能为测试日志建立索引缓存");
        pending_cache
            .expect("首次打开应生成待写入的索引缓存")
            .persist()
            .expect("应能写入测试索引缓存");

        let preview = LogDocument::open_cached_preview(&path, &cache_directory, 1, 3, 8)
            .expect("缓存预览检查不应失败");

        assert!(preview.is_none());
        _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn cached_preview_shrinks_to_complete_lines_around_the_anchor() {
        let directory = test_directory("shrunk-cached-preview");
        let path = directory.join("source.log");
        let cache_directory = directory.join("cache");
        fs::write(&path, b"zero\none\ntwo\nthree\nfour").expect("应能写入测试日志");
        let (_, pending_cache) = LogDocument::open_with_index_cache(&path, &cache_directory)
            .expect("应能为测试日志建立索引缓存");
        pending_cache
            .expect("首次打开应生成待写入的索引缓存")
            .persist()
            .expect("应能写入测试索引缓存");

        let preview = LogDocument::open_cached_preview(&path, &cache_directory, 2, 5, 9)
            .expect("应能读取缩小后的缓存预览")
            .expect("锚点行在预算内时缓存预览应存在");

        assert!(preview.contains_source_row(2));
        assert!(preview.bytes.len() <= 9);
        assert_eq!(preview.line(2).as_deref(), Some("two"));
        _ = fs::remove_dir_all(directory);
    }
}

#[cfg(test)]
mod line_preview_tests {
    use super::FileEncoding;

    #[test]
    fn utf8_preview_never_splits_a_codepoint() {
        let bytes = "a你b".as_bytes();

        let preview = FileEncoding::Utf8.decode_preview(bytes, 2);
        assert_eq!(preview.text(), "a");
        assert!(preview.is_truncated());

        let preview = FileEncoding::Utf8.decode_preview(bytes, 4);
        assert_eq!(preview.text(), "a你");
        assert!(preview.is_truncated());
    }

    #[test]
    fn utf16_preview_never_splits_a_surrogate_pair() {
        let units = "A😀B".encode_utf16().collect::<Vec<_>>();
        let bytes = units
            .iter()
            .flat_map(|unit| unit.to_le_bytes())
            .collect::<Vec<_>>();

        let preview = FileEncoding::Utf16Le.decode_preview(&bytes, 4);
        assert_eq!(preview.text(), "A");
        assert!(preview.is_truncated());

        let preview = FileEncoding::Utf16Le.decode_preview(&bytes, 6);
        assert_eq!(preview.text(), "A😀");
        assert!(preview.is_truncated());
    }

    #[test]
    fn binary_preview_decodes_only_the_bounded_prefix() {
        let preview = FileEncoding::Binary.decode_preview(&[0x00, 0xff, 0x42], 2);
        assert_eq!(preview.text(), "00 ff");
        assert!(preview.is_truncated());

        let complete = FileEncoding::Binary.decode_preview(&[0x00, 0xff], usize::MAX);
        assert_eq!(complete.text(), "00 ff");
        assert!(!complete.is_truncated());
    }
}

#[cfg(test)]
mod index_cache_tests {
    use std::{fs, sync::Arc, time::SystemTime};

    #[cfg(unix)]
    use std::{ffi::OsString, os::unix::ffi::OsStringExt as _, path::PathBuf};

    use super::{
        APPEND_INTEGRITY_BLOCK_BYTES, FileEncoding, FileIdentity, LineStarts, MutableLineStarts,
        PendingIndexCacheWrite, index_cache_path, index_cache_source_identity, read_index_cache,
    };

    #[test]
    fn completed_line_offsets_use_compact_storage_when_possible() {
        let starts = LineStarts::from_native(vec![0, 11, u32::MAX as usize]);

        assert!(matches!(&starts, LineStarts::Compact(_)));
        assert_eq!(
            starts.iter().collect::<Vec<_>>(),
            [0, 11, u32::MAX as usize]
        );
    }

    #[test]
    fn line_offset_builder_stays_compact_for_common_files() {
        let mut starts = MutableLineStarts::with_capacity(u64::from(u32::MAX), 2);
        starts.push(0);
        starts.push(11);

        assert!(matches!(&starts, MutableLineStarts::Compact(_)));
        assert!(matches!(starts.into_immutable(), LineStarts::Compact(_)));
    }

    #[cfg(target_pointer_width = "64")]
    #[test]
    fn compact_append_builder_promotes_only_when_required() {
        let mut starts = MutableLineStarts::from_immutable(&LineStarts::from_native(vec![0, 11]));

        starts.push(u32::MAX as usize + 1);

        assert!(matches!(&starts, MutableLineStarts::Wide(_)));
        assert_eq!(starts.get(0), Some(0));
        assert_eq!(starts.get(1), Some(11));
        assert_eq!(starts.get(2), Some(u32::MAX as usize + 1));
    }

    #[cfg(target_pointer_width = "64")]
    #[test]
    fn completed_line_offsets_fall_back_for_files_larger_than_u32() {
        let wide_offset = u32::MAX as usize + 1;
        let starts = LineStarts::from_native(vec![0, wide_offset]);

        assert!(matches!(&starts, LineStarts::Wide(_)));
        assert_eq!(starts.get(1), Some(wide_offset));
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_paths_keep_distinct_cache_identities() {
        let first = PathBuf::from(OsString::from_vec(b"source-\x80.log".to_vec()));
        let second = PathBuf::from(OsString::from_vec(b"source-\x81.log".to_vec()));

        assert_eq!(first.to_string_lossy(), second.to_string_lossy());
        assert_ne!(
            index_cache_source_identity(&first),
            index_cache_source_identity(&second)
        );
        assert_ne!(
            index_cache_path(PathBuf::from("cache").as_path(), &first),
            index_cache_path(PathBuf::from("cache").as_path(), &second)
        );
    }

    #[test]
    fn cache_round_trip_preserves_encoding_and_integrity_blocks() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("系统时间应晚于 Unix 纪元")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "vclogg2-index-cache-test-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&directory).expect("应能创建缓存测试目录");
        let cache_path = directory.join("round-trip.vclog-index");
        let source_path = directory.join("source.log");
        let file_size = (APPEND_INTEGRITY_BLOCK_BYTES + 1) as u64;
        let modified_millis = 123_456;
        let identity = FileIdentity {
            volume_serial: 42,
            file_id: [7; 16],
            usn: 99,
        };
        let pending = PendingIndexCacheWrite {
            cache_path: cache_path.clone(),
            source_path: source_path.clone(),
            file_size,
            modified_millis,
            identity: Some(identity.clone()),
            encoding: FileEncoding::Utf8Bom,
            line_starts: LineStarts::from_native(vec![0, 11, 23]),
            integrity_blocks: Arc::from([[3; 32], [5; 32]]),
            longest_line_bytes: 12,
            longest_completed_line_bytes: 12,
            longest_line_columns: 12,
            longest_completed_line_columns: 12,
        };

        pending.persist().expect("索引缓存应能写入");
        let cached = read_index_cache(
            &cache_path,
            &source_path,
            file_size,
            modified_millis,
            Some(&identity),
        )
        .expect("索引缓存应能读回");

        assert_eq!(cached.encoding.name(), "UTF-8 BOM");
        assert_eq!(
            (0..cached.indexed_lines.starts.len())
                .map(|index| cached.indexed_lines.starts.get(index).unwrap())
                .collect::<Vec<_>>(),
            [0, 11, 23]
        );
        assert_eq!(cached.integrity_blocks.as_ref(), &[[3; 32], [5; 32]]);
        _ = fs::remove_dir_all(directory);
    }
}
