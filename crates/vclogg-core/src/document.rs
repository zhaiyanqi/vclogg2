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
use encoding_rs::{CoderResult, Decoder, Encoding, UTF_8, UTF_16BE, UTF_16LE};
use memchr::{memchr2, memchr3_iter};
use rayon::prelude::{IntoParallelIterator as _, ParallelIterator as _};
use sha2::{Digest as _, Sha256};

use crate::{CancellationToken, CompressedRows};

mod index_cache;

#[cfg(test)]
use index_cache::index_cache_source_identity;
use index_cache::{
    index_cache_path, read_index_cache, read_index_cache_while, system_time_millis,
    write_index_cache,
};

#[cfg(windows)]
use windows_sys::Win32::{
    Storage::FileSystem::{
        FILE_ID_INFO, FileIdInfo, GetFileInformationByHandleEx, MOVEFILE_REPLACE_EXISTING,
        MOVEFILE_WRITE_THROUGH, MoveFileExW,
    },
    System::{IO::DeviceIoControl, Ioctl::FSCTL_READ_FILE_USN_DATA},
};

const APPEND_INTEGRITY_BLOCK_BYTES: usize = 4 * 1024 * 1024;
const VERIFIED_BLOCK_CACHE_BYTES: usize = 8 * 1024 * 1024;
const INDEX_CACHE_MAGIC: &[u8; 8] = b"VCLOGG05";
const INDEX_CACHE_VERSION: u32 = 3;
const INDEX_CACHE_HEADER_BYTES: u64 = 8 + 4 + 4 + 2 + 8 + 8 + 1 + 8 + 16 + 8 + 8 * 7;
const MAX_CACHE_PATH_BYTES: usize = 32 * 1024;
const MAX_CACHE_ENCODING_BYTES: usize = 64;
const ENCODING_DETECTION_BYTES: usize = 1024 * 1024;
const BINARY_BYTES_PER_LINE: usize = 16;
const INDEX_CANCELLATION_BATCH_LINES: usize = 4 * 1024;
const CACHE_CANCELLATION_BATCH_LINES: usize = 4 * 1024;
const PARALLEL_INDEX_MIN_BYTES: usize = 8 * 1024 * 1024;

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum VerifiedSourceUnavailable {
    Transient,
    InvalidSnapshot,
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

    fn source_file(&self) -> Result<Arc<File>, VerifiedSourceUnavailable> {
        if !self.transient_source_handles.load(Ordering::Acquire)
            && let Some(file) = self
                .file
                .read()
                .map_err(|_| VerifiedSourceUnavailable::Transient)?
                .clone()
        {
            return Ok(file);
        }

        let file =
            Arc::new(File::open(&self.path).map_err(|_| VerifiedSourceUnavailable::Transient)?);
        let metadata = file
            .metadata()
            .map_err(|_| VerifiedSourceUnavailable::Transient)?;
        let expected_len =
            u64::try_from(self.len).map_err(|_| VerifiedSourceUnavailable::InvalidSnapshot)?;
        if metadata.len() != expected_len {
            return Err(VerifiedSourceUnavailable::InvalidSnapshot);
        }
        if let Some(expected) = &self.identity {
            match try_read_file_identity(&file) {
                Ok(current) if &current == expected => {}
                Ok(_) => return Err(VerifiedSourceUnavailable::InvalidSnapshot),
                Err(_) => return Err(VerifiedSourceUnavailable::Transient),
            }
        }
        if self.transient_source_handles.load(Ordering::Acquire) {
            return Ok(file);
        }

        let mut retained = self
            .file
            .write()
            .map_err(|_| VerifiedSourceUnavailable::Transient)?;
        if self.transient_source_handles.load(Ordering::Acquire) {
            return Ok(file);
        }
        Ok(retained.get_or_insert(file).clone())
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

    fn read_source_block(&self, block_ix: usize) -> Result<Vec<u8>, VerifiedSourceUnavailable> {
        let block_start = block_ix
            .checked_mul(APPEND_INTEGRITY_BLOCK_BYTES)
            .ok_or(VerifiedSourceUnavailable::InvalidSnapshot)?;
        let block_end = block_start
            .checked_add(APPEND_INTEGRITY_BLOCK_BYTES)
            .ok_or(VerifiedSourceUnavailable::InvalidSnapshot)?
            .min(self.len);
        let mut block = vec![
            0_u8;
            block_end
                .checked_sub(block_start)
                .ok_or(VerifiedSourceUnavailable::InvalidSnapshot)?
        ];
        let file = self.source_file()?;
        read_file_exact_at(&file, &mut block, block_start as u64).map_err(|error| {
            if error.kind() == std::io::ErrorKind::UnexpectedEof {
                VerifiedSourceUnavailable::InvalidSnapshot
            } else {
                VerifiedSourceUnavailable::Transient
            }
        })?;
        Ok(block)
    }

    fn load_source_block(&self, block_ix: usize) -> Option<Arc<[u8]>> {
        self.read_source_block(block_ix)
            .ok()
            .map(|block| block.into())
    }

    fn source_identity_matches(&self) -> bool {
        let Some(expected) = self.identity.as_ref() else {
            return false;
        };
        let Ok(file) = self.source_file() else {
            return false;
        };
        let Ok(metadata) = file.metadata() else {
            return false;
        };
        metadata.len() == self.len as u64
            && try_read_file_identity(&file).is_ok_and(|identity| &identity == expected)
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
            let block = self.read_source_block(block_ix)?;
            let expected = self
                .integrity_blocks
                .get(block_ix)
                .ok_or(VerifiedSourceUnavailable::InvalidSnapshot)?;
            if <[u8; 32]>::from(Sha256::digest(&block)) != *expected {
                return Err(VerifiedSourceUnavailable::InvalidSnapshot);
            }
            Ok(block)
        })();
        let block = match block {
            Ok(block) => block,
            Err(VerifiedSourceUnavailable::Transient) => return None,
            Err(VerifiedSourceUnavailable::InvalidSnapshot) => {
                let mut state = self.state.lock().ok()?;
                if let Some(block) = state.blocks.get(&block_ix).cloned() {
                    return Some(block);
                }
                state.invalid_blocks.insert(block_ix);
                return None;
            }
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
    integrity_blocks: Arc<[[u8; 32]]>,
}

struct IndexedLines {
    starts: MutableLineStarts,
    longest_line_bytes: usize,
    longest_completed_line_bytes: usize,
    longest_line_columns: usize,
    longest_completed_line_columns: usize,
}

type IndexedFileSnapshot = (IndexedLines, Arc<[[u8; 32]]>);

struct ParallelIndexedBlock {
    control_bytes: Vec<u32>,
    digest: [u8; 32],
}

const CONTROL_KIND_BITS: u32 = 2;
const CONTROL_CR: u32 = 0;
const CONTROL_LF: u32 = 1;
const CONTROL_TAB: u32 = 2;

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
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LinePreview {
    text: String,
    truncated: bool,
}

/// Task-local reader for a sequence of complete logical lines.
///
/// It retains at most the current verified source block, so search, export, and explicit copy
/// operations can share adjacent positional reads without growing document-owned caches.
#[derive(Default)]
pub struct LineReader {
    verified_blocks: TaskVerifiedBlockReader,
}

/// Task-local reader for a sequence of visible line previews.
///
/// It retains at most the current verified source block, so adjacent rows from transient
/// directory-search snapshots share one positional read without growing document-owned caches.
#[derive(Default)]
pub struct LinePreviewReader {
    verified_blocks: TaskVerifiedBlockReader,
}

#[derive(Default)]
struct TaskVerifiedBlockReader {
    content_digest: Option<[u8; 32]>,
    verified_block: Option<(usize, Option<Arc<[u8]>>)>,
}

enum TaskVerifiedBytes {
    Shared(Arc<[u8]>, std::ops::Range<usize>),
    Owned(Box<[u8]>),
}

impl Deref for TaskVerifiedBytes {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Shared(bytes, range) => &bytes[range.clone()],
            Self::Owned(bytes) => bytes,
        }
    }
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

impl LineReader {
    /// Decode one complete logical line in this reader's task-local I/O context.
    pub fn line(&mut self, document: &LogDocument, source_row: usize) -> Option<String> {
        self.verified_blocks.reset_for(document.content_digest);
        let row_ix = document.local_row(source_row)?;
        let range = document.line_byte_range_at_local_row(row_ix)?;
        let bytes = match document.bytes.as_ref() {
            DocumentBytes::Empty => range.is_empty().then_some(&[][..])?,
            DocumentBytes::Owned(bytes) => bytes.get(range)?,
            DocumentBytes::Verified(bytes) => {
                let bytes = self.verified_blocks.read_range(bytes, range)?;
                return Some(decode_complete_line(document, &bytes));
            }
        };
        Some(decode_complete_line(document, bytes))
    }
}

impl LinePreviewReader {
    pub fn line_preview(
        &mut self,
        document: &LogDocument,
        source_row: usize,
        max_bytes: usize,
    ) -> Option<LinePreview> {
        self.verified_blocks.reset_for(document.content_digest);
        let row_ix = document.local_row(source_row)?;
        let range = document.line_byte_range_at_local_row(row_ix)?;
        let read_end = range
            .end
            .min(range.start.saturating_add(max_bytes).saturating_add(4));
        let complete_line = read_end == range.end;
        match document.bytes.as_ref() {
            DocumentBytes::Empty => {
                (range.is_empty()).then(|| decode_line_preview(document, &[], max_bytes, true))
            }
            DocumentBytes::Owned(bytes) => Some(decode_line_preview(
                document,
                bytes.get(range.start..read_end)?,
                max_bytes,
                complete_line,
            )),
            DocumentBytes::Verified(bytes) => {
                let bytes = self
                    .verified_blocks
                    .read_range(bytes, range.start..read_end)?;
                Some(decode_line_preview(
                    document,
                    &bytes,
                    max_bytes,
                    complete_line,
                ))
            }
        }
    }
}

impl TaskVerifiedBlockReader {
    fn reset_for(&mut self, content_digest: [u8; 32]) {
        if self.content_digest != Some(content_digest) {
            self.content_digest = Some(content_digest);
            self.verified_block = None;
        }
    }

    fn read_range(
        &mut self,
        bytes: &VerifiedFileBytes,
        range: std::ops::Range<usize>,
    ) -> Option<TaskVerifiedBytes> {
        if range.start > range.end || range.end > bytes.len {
            return None;
        }
        if range.is_empty() {
            return Some(TaskVerifiedBytes::Owned(Box::new([])));
        }
        let first_block = range.start / APPEND_INTEGRITY_BLOCK_BYTES;
        let last_block = (range.end - 1) / APPEND_INTEGRITY_BLOCK_BYTES;
        if first_block == last_block {
            let block = self.load_verified_block(bytes, first_block)?;
            let block_start = first_block.saturating_mul(APPEND_INTEGRITY_BLOCK_BYTES);
            let local_range = range.start - block_start..range.end - block_start;
            block.get(local_range.clone())?;
            return Some(TaskVerifiedBytes::Shared(block, local_range));
        }

        let mut joined = Vec::with_capacity(range.len());
        for block_ix in first_block..=last_block {
            let block = self.load_verified_block(bytes, block_ix)?;
            let block_start = block_ix.saturating_mul(APPEND_INTEGRITY_BLOCK_BYTES);
            let start = range.start.saturating_sub(block_start).min(block.len());
            let end = range.end.saturating_sub(block_start).min(block.len());
            joined.extend_from_slice(block.get(start..end)?);
        }
        Some(TaskVerifiedBytes::Owned(joined.into_boxed_slice()))
    }

    fn load_verified_block(
        &mut self,
        bytes: &VerifiedFileBytes,
        block_ix: usize,
    ) -> Option<Arc<[u8]>> {
        if self
            .verified_block
            .as_ref()
            .is_none_or(|(cached_ix, _)| *cached_ix != block_ix)
        {
            self.verified_block = Some((block_ix, bytes.load_verified_block(block_ix, false)));
        }
        self.verified_block.as_ref()?.1.clone()
    }
}

fn decode_complete_line(document: &LogDocument, bytes: &[u8]) -> String {
    document
        .encoding
        .decode(document.encoding.trim_line_bytes(bytes))
}

fn decode_line_preview(
    document: &LogDocument,
    bytes: &[u8],
    max_bytes: usize,
    complete_line: bool,
) -> LinePreview {
    let bytes = if complete_line {
        document.encoding.trim_line_bytes(bytes)
    } else {
        bytes
    };
    let mut preview = document.encoding.decode_preview(bytes, max_bytes);
    preview.truncated |= !complete_line;
    preview
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
    verify_integrity: bool,
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
                        let block = if self.verify_integrity {
                            bytes.load_verified_block(block_ix, false)?
                        } else {
                            bytes.load_source_block(block_ix)?
                        };
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
                    let block = if self.verify_integrity {
                        bytes.load_verified_block(first_block, false)
                    } else {
                        bytes.load_source_block(first_block)
                    };
                    self.verified_block = Some((first_block, block));
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
        let Some((indexed_lines, integrity_blocks)) = build_file_index_with_integrity_while(
            &file,
            file_metadata.len(),
            file_metadata.modified().ok(),
            read_file_identity(&file),
            encoding,
            &path,
            &|| cancellation.is_cancelled(),
        )?
        else {
            return Ok(None);
        };
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
                let Some((indexed_lines, integrity_blocks)) =
                    build_file_index_with_integrity_while(
                        &file,
                        file_size,
                        modified,
                        identity.clone(),
                        encoding,
                        &path,
                        &|| cancellation.is_cancelled(),
                    )?
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
        LinePreviewReader::default().line_preview(self, source_row, max_bytes)
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

    pub(crate) fn search_lines(&self, verify_integrity: bool) -> DocumentSearchLines<'_> {
        DocumentSearchLines {
            document: self,
            verify_integrity,
            verified_block: None,
        }
    }

    pub(crate) fn has_strong_source_identity(&self) -> bool {
        matches!(self.bytes.as_ref(), DocumentBytes::Verified(bytes) if bytes.identity.is_some())
    }

    /// Recheck the platform change token around a search that bypasses per-block hashing.
    /// Callers without a strong identity must keep verifying every block instead.
    pub(crate) fn source_identity_matches(&self) -> bool {
        match self.bytes.as_ref() {
            DocumentBytes::Verified(bytes) => bytes.source_identity_matches(),
            DocumentBytes::Empty | DocumentBytes::Owned(_) => true,
        }
    }

    fn try_refresh_appended(&self) -> Result<Option<Self>> {
        if !self.has_complete_line_index() {
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
        if old_size == 0 || new_size <= old_size {
            return Ok(None);
        }

        let new_size_usize = usize::try_from(new_size)
            .with_context(|| format!("日志文件过大，无法建立索引：{}", path.display()))?;
        let old_size_usize = usize::try_from(old_size)
            .with_context(|| format!("日志文件过大，无法校验追加：{}", path.display()))?;
        let mut line_starts = MutableLineStarts::from_immutable(&self.line_starts);
        if line_starts.last() == Some(old_size_usize) {
            line_starts.truncate(line_starts.len().saturating_sub(1));
        }
        let indexer = StreamingLineIndexer::resume(
            new_size_usize,
            self.encoding,
            line_starts,
            self.longest_completed_line_bytes,
            self.longest_completed_line_columns,
        );
        let Some((indexed_lines, integrity_blocks)) = build_appended_file_index_with_integrity(
            &file,
            new_size_usize,
            file_metadata.modified().ok(),
            read_file_identity(&file),
            old_size_usize,
            self.append_fingerprint.integrity_blocks.as_ref(),
            indexer,
            path,
        )?
        else {
            return Ok(None);
        };
        let bytes = DocumentBytes::verified(
            file,
            path.to_path_buf(),
            new_size_usize,
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
        let integrity_blocks = integrity_blocks.unwrap_or_else(|| {
            calculate_integrity_blocks(
                bytes
                    .resident_slice()
                    .expect("only resident previews may omit integrity blocks"),
            )
            .into()
        });
        let content_digest = digest_integrity_blocks(&integrity_blocks);
        let append_fingerprint = AppendFingerprint { integrity_blocks };
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

enum StreamingLineColumns {
    Bytes(usize),
    Decoded {
        encoding: &'static Encoding,
        decoder: Decoder,
        columns: usize,
    },
}

impl StreamingLineColumns {
    fn new(encoding: FileEncoding) -> Self {
        match encoding {
            FileEncoding::Utf8 | FileEncoding::Utf8Bom => Self::Bytes(0),
            FileEncoding::Utf16Le => Self::decoded(UTF_16LE),
            FileEncoding::Utf16Be => Self::decoded(UTF_16BE),
            FileEncoding::Legacy(encoding) => Self::decoded(encoding),
            FileEncoding::Binary => Self::Bytes(0),
        }
    }

    fn decoded(encoding: &'static Encoding) -> Self {
        Self::Decoded {
            encoding,
            decoder: encoding.new_decoder_without_bom_handling(),
            columns: 0,
        }
    }

    fn feed(&mut self, bytes: &[u8]) {
        match self {
            Self::Bytes(columns) => update_byte_columns(columns, bytes),
            Self::Decoded {
                decoder, columns, ..
            } => update_decoded_columns(decoder, columns, bytes, false),
        }
    }

    fn finish_line(&mut self) -> usize {
        match self {
            Self::Bytes(columns) => std::mem::take(columns),
            Self::Decoded {
                encoding,
                decoder,
                columns,
            } => {
                update_decoded_columns(decoder, columns, &[], true);
                let completed = std::mem::take(columns);
                *decoder = encoding.new_decoder_without_bom_handling();
                completed
            }
        }
    }
}

fn update_byte_columns(columns: &mut usize, bytes: &[u8]) {
    for byte in bytes {
        *columns = match byte {
            b'\t' => columns.saturating_add(8 - *columns % 8),
            b'\r' => *columns,
            _ => columns.saturating_add(1),
        };
    }
}

fn update_decoded_columns(
    decoder: &mut Decoder,
    columns: &mut usize,
    mut bytes: &[u8],
    last: bool,
) {
    let mut output = [0_u8; 8 * 1024];
    loop {
        let (result, read, written, _) = decoder.decode_to_utf8(bytes, &mut output, last);
        let decoded =
            std::str::from_utf8(&output[..written]).expect("encoding_rs always emits valid UTF-8");
        for character in decoded.chars() {
            *columns = match character {
                '\t' => columns.saturating_add(8 - *columns % 8),
                '\r' => *columns,
                _ => columns.saturating_add(1),
            };
        }
        bytes = &bytes[read..];
        if result == CoderResult::InputEmpty {
            break;
        }
    }
}

struct StreamingLineIndexer {
    encoding: FileEncoding,
    starts: MutableLineStarts,
    columns: StreamingLineColumns,
    current_start: usize,
    longest_completed_line_bytes: usize,
    longest_completed_line_columns: usize,
    completed_lines: usize,
    pending_cr: Option<usize>,
    bom_remaining: usize,
}

impl StreamingLineIndexer {
    fn new(file_size: usize, encoding: FileEncoding) -> Self {
        let mut starts = MutableLineStarts::with_capacity(file_size as u64, 0);
        if file_size > 0 {
            starts.push(0);
        }
        Self {
            encoding,
            starts,
            columns: StreamingLineColumns::new(encoding),
            current_start: 0,
            longest_completed_line_bytes: 0,
            longest_completed_line_columns: 0,
            completed_lines: 0,
            pending_cr: None,
            bom_remaining: encoding.bom_len(),
        }
    }

    fn resume(
        file_size: usize,
        encoding: FileEncoding,
        mut starts: MutableLineStarts,
        longest_completed_line_bytes: usize,
        longest_completed_line_columns: usize,
    ) -> Self {
        if file_size > 0 && starts.is_empty() {
            starts.push(0);
        }
        let current_start = starts.last().unwrap_or_default();
        Self {
            encoding,
            starts,
            columns: StreamingLineColumns::new(encoding),
            current_start,
            longest_completed_line_bytes,
            longest_completed_line_columns,
            completed_lines: 0,
            pending_cr: None,
            bom_remaining: usize::from(current_start == 0).saturating_mul(encoding.bom_len()),
        }
    }

    fn feed(
        &mut self,
        block_start: usize,
        bytes: &[u8],
        final_block: bool,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Option<()> {
        match self.encoding {
            FileEncoding::Utf16Le | FileEncoding::Utf16Be => {
                self.feed_utf16(block_start, bytes, final_block, is_cancelled)
            }
            FileEncoding::Utf8 | FileEncoding::Utf8Bom | FileEncoding::Legacy(_) => {
                self.feed_single_byte(block_start, bytes, final_block, is_cancelled)
            }
            FileEncoding::Binary => Some(()),
        }
    }

    fn feed_content(&mut self, bytes: &[u8]) {
        let skip = self.bom_remaining.min(bytes.len());
        self.bom_remaining -= skip;
        self.columns.feed(&bytes[skip..]);
    }

    fn complete_line(
        &mut self,
        line_end: usize,
        delimiter_width: usize,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Option<()> {
        self.completed_lines = self.completed_lines.saturating_add(1);
        if self
            .completed_lines
            .is_multiple_of(INDEX_CANCELLATION_BATCH_LINES)
            && is_cancelled()
        {
            return None;
        }
        self.longest_completed_line_bytes = self
            .longest_completed_line_bytes
            .max(line_end.saturating_sub(self.current_start));
        self.longest_completed_line_columns = self
            .longest_completed_line_columns
            .max(self.columns.finish_line());
        self.current_start = line_end.saturating_add(delimiter_width);
        if self.starts.last() != Some(self.current_start) {
            self.starts.push(self.current_start);
        }
        Some(())
    }

    fn feed_single_byte(
        &mut self,
        block_start: usize,
        bytes: &[u8],
        final_block: bool,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Option<()> {
        let mut content_start = 0;
        if let Some(pending_cr) = self.pending_cr.take() {
            if bytes.first() == Some(&b'\n') {
                self.complete_line(pending_cr, 2, is_cancelled)?;
                content_start = 1;
            } else {
                self.complete_line(pending_cr, 1, is_cancelled)?;
            }
        }

        let mut search_start = content_start;
        while let Some(relative) = memchr2(b'\r', b'\n', &bytes[search_start..]) {
            let position = search_start + relative;
            self.feed_content(&bytes[content_start..position]);
            let absolute = block_start.saturating_add(position);
            if bytes[position] == b'\r' && position + 1 == bytes.len() && !final_block {
                self.pending_cr = Some(absolute);
                return Some(());
            }
            let width =
                usize::from(bytes[position] == b'\r' && bytes.get(position + 1) == Some(&b'\n'))
                    + 1;
            self.complete_line(absolute, width, is_cancelled)?;
            content_start = position + width;
            search_start = content_start;
        }
        self.feed_content(&bytes[content_start..]);
        Some(())
    }

    fn feed_utf16(
        &mut self,
        block_start: usize,
        bytes: &[u8],
        final_block: bool,
        is_cancelled: &dyn Fn() -> bool,
    ) -> Option<()> {
        let little_endian = matches!(self.encoding, FileEncoding::Utf16Le);
        let code_unit_at = |position: usize| {
            if little_endian {
                u16::from_le_bytes([bytes[position], bytes[position + 1]])
            } else {
                u16::from_be_bytes([bytes[position], bytes[position + 1]])
            }
        };
        let mut content_start = 0;
        if let Some(pending_cr) = self.pending_cr.take() {
            if bytes.len() >= 2 && code_unit_at(0) == u16::from(b'\n') {
                self.complete_line(pending_cr, 4, is_cancelled)?;
                content_start = 2;
            } else {
                self.complete_line(pending_cr, 2, is_cancelled)?;
            }
        }

        let mut position = content_start;
        while position + 1 < bytes.len() {
            let code_unit = code_unit_at(position);
            if code_unit != u16::from(b'\r') && code_unit != u16::from(b'\n') {
                position += 2;
                continue;
            }
            self.feed_content(&bytes[content_start..position]);
            let absolute = block_start.saturating_add(position);
            if code_unit == u16::from(b'\r') && position + 2 == bytes.len() && !final_block {
                self.pending_cr = Some(absolute);
                return Some(());
            }
            let followed_by_lf = code_unit == u16::from(b'\r')
                && position + 3 < bytes.len()
                && code_unit_at(position + 2) == u16::from(b'\n');
            let width = if followed_by_lf { 4 } else { 2 };
            self.complete_line(absolute, width, is_cancelled)?;
            content_start = position + width;
            position = content_start;
        }
        self.feed_content(&bytes[content_start..]);
        Some(())
    }

    fn finish(mut self, file_size: usize) -> IndexedLines {
        let trailing_line_bytes = file_size.saturating_sub(self.current_start);
        let trailing_line_columns = if self.current_start < file_size {
            self.columns.finish_line()
        } else {
            0
        };
        IndexedLines {
            starts: self.starts,
            longest_line_bytes: self.longest_completed_line_bytes.max(trailing_line_bytes),
            longest_completed_line_bytes: self.longest_completed_line_bytes,
            longest_line_columns: self
                .longest_completed_line_columns
                .max(trailing_line_columns),
            longest_completed_line_columns: self.longest_completed_line_columns,
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn build_file_index_with_integrity_while(
    file: &File,
    file_size: u64,
    expected_modified: Option<SystemTime>,
    expected_identity: Option<FileIdentity>,
    encoding: FileEncoding,
    path: &Path,
    is_cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<Option<IndexedFileSnapshot>> {
    if is_cancelled() {
        return Ok(None);
    }
    let file_size_usize = usize::try_from(file_size)
        .with_context(|| format!("日志文件过大，无法建立索引：{}", path.display()))?;
    if file_size_usize >= PARALLEL_INDEX_MIN_BYTES
        && matches!(encoding, FileEncoding::Utf8 | FileEncoding::Utf8Bom)
    {
        let Some(snapshot) = build_parallel_utf8_file_index_with_integrity(
            file,
            file_size_usize,
            encoding,
            path,
            is_cancelled,
        )?
        else {
            return Ok(None);
        };
        validate_indexed_file_snapshot(
            file,
            file_size,
            expected_modified,
            expected_identity.as_ref(),
            path,
        )?;
        return Ok(Some(snapshot));
    }
    let mut indexer = (!matches!(encoding, FileEncoding::Binary))
        .then(|| StreamingLineIndexer::new(file_size_usize, encoding));
    let mut integrity_blocks =
        Vec::with_capacity(file_size_usize.div_ceil(APPEND_INTEGRITY_BLOCK_BYTES));
    let mut block_start = 0usize;
    let mut block = vec![0_u8; file_size_usize.min(APPEND_INTEGRITY_BLOCK_BYTES)];
    while block_start < file_size_usize {
        if is_cancelled() {
            return Ok(None);
        }
        let block_len = (file_size_usize - block_start).min(APPEND_INTEGRITY_BLOCK_BYTES);
        block.resize(block_len, 0);
        read_file_exact_at(file, &mut block, block_start as u64)
            .with_context(|| format!("建立索引时无法读取日志文件：{}", path.display()))?;
        integrity_blocks.push(Sha256::digest(&block).into());
        if let Some(indexer) = indexer.as_mut() {
            let final_block = block_start.saturating_add(block_len) == file_size_usize;
            if indexer
                .feed(block_start, &block, final_block, is_cancelled)
                .is_none()
            {
                return Ok(None);
            }
        }
        block_start = block_start.saturating_add(block_len);
    }
    if is_cancelled() {
        return Ok(None);
    }
    validate_indexed_file_snapshot(
        file,
        file_size,
        expected_modified,
        expected_identity.as_ref(),
        path,
    )?;

    let indexed_lines = match indexer {
        Some(indexer) => indexer.finish(file_size_usize),
        None => {
            let Some(indexed_lines) = build_binary_line_index(file_size_usize, is_cancelled) else {
                return Ok(None);
            };
            indexed_lines
        }
    };
    Ok(Some((indexed_lines, integrity_blocks.into())))
}

fn build_parallel_utf8_file_index_with_integrity(
    file: &File,
    file_size: usize,
    encoding: FileEncoding,
    path: &Path,
    is_cancelled: &(dyn Fn() -> bool + Sync),
) -> Result<Option<IndexedFileSnapshot>> {
    debug_assert!(matches!(
        encoding,
        FileEncoding::Utf8 | FileEncoding::Utf8Bom
    ));
    let block_count = file_size.div_ceil(APPEND_INTEGRITY_BLOCK_BYTES);
    let blocks = (0..block_count)
        .into_par_iter()
        .map(|block_ix| -> Result<Option<ParallelIndexedBlock>> {
            if is_cancelled() {
                return Ok(None);
            }
            let block_start = block_ix.saturating_mul(APPEND_INTEGRITY_BLOCK_BYTES);
            let block_len = (file_size - block_start).min(APPEND_INTEGRITY_BLOCK_BYTES);
            let mut bytes = vec![0_u8; block_len];
            read_file_exact_at(file, &mut bytes, block_start as u64)
                .with_context(|| format!("建立索引时无法读取日志文件：{}", path.display()))?;
            if is_cancelled() {
                return Ok(None);
            }

            let digest = Sha256::digest(&bytes).into();
            let mut control_bytes = Vec::new();
            for offset in memchr3_iter(b'\r', b'\n', b'\t', &bytes) {
                let kind = match bytes[offset] {
                    b'\r' => CONTROL_CR,
                    b'\n' => CONTROL_LF,
                    b'\t' => CONTROL_TAB,
                    _ => unreachable!("memchr3 only returns requested control bytes"),
                };
                let offset =
                    u32::try_from(offset).expect("an integrity block offset always fits in u32");
                control_bytes.push((offset << CONTROL_KIND_BITS) | kind);
            }
            Ok((!is_cancelled()).then_some(ParallelIndexedBlock {
                control_bytes,
                digest,
            }))
        })
        .collect::<Result<Vec<_>>>()?;
    if blocks.iter().any(Option::is_none) || is_cancelled() {
        return Ok(None);
    }
    let blocks = blocks
        .into_iter()
        .map(|block| block.expect("cancelled blocks were handled before merging"))
        .collect::<Vec<_>>();

    let mut starts = MutableLineStarts::with_capacity(file_size as u64, 0);
    if file_size > 0 {
        starts.push(0);
    }
    let mut current_start = 0_usize;
    let mut content_cursor = encoding.bom_len().min(file_size);
    let mut current_columns = 0_usize;
    let mut longest_completed_line_bytes = 0_usize;
    let mut longest_completed_line_columns = 0_usize;
    let mut completed_lines = 0_usize;
    let mut controls = blocks
        .iter()
        .enumerate()
        .flat_map(|(block_ix, block)| {
            block.control_bytes.iter().map(move |encoded| {
                let offset = (encoded >> CONTROL_KIND_BITS) as usize;
                let kind = encoded & ((1 << CONTROL_KIND_BITS) - 1);
                (
                    block_ix
                        .saturating_mul(APPEND_INTEGRITY_BLOCK_BYTES)
                        .saturating_add(offset),
                    kind,
                )
            })
        })
        .peekable();

    while let Some((position, kind)) = controls.next() {
        if position < content_cursor {
            continue;
        }
        if kind == CONTROL_TAB {
            current_columns = current_columns.saturating_add(position - content_cursor);
            current_columns = current_columns.saturating_add(8 - current_columns % 8);
            content_cursor = position.saturating_add(1);
            continue;
        }

        current_columns = current_columns.saturating_add(position - content_cursor);
        let delimiter_width = if kind == CONTROL_CR
            && controls.peek().is_some_and(|(next_position, next_kind)| {
                *next_position == position.saturating_add(1) && *next_kind == CONTROL_LF
            }) {
            _ = controls.next();
            2
        } else {
            1
        };
        completed_lines = completed_lines.saturating_add(1);
        if completed_lines.is_multiple_of(INDEX_CANCELLATION_BATCH_LINES) && is_cancelled() {
            return Ok(None);
        }
        longest_completed_line_bytes =
            longest_completed_line_bytes.max(position.saturating_sub(current_start));
        longest_completed_line_columns = longest_completed_line_columns.max(current_columns);
        current_start = position.saturating_add(delimiter_width);
        if starts.last() != Some(current_start) {
            starts.push(current_start);
        }
        content_cursor = current_start;
        current_columns = 0;
    }

    current_columns = current_columns.saturating_add(file_size.saturating_sub(content_cursor));
    let trailing_line_bytes = file_size.saturating_sub(current_start);
    let indexed_lines = IndexedLines {
        starts,
        longest_line_bytes: longest_completed_line_bytes.max(trailing_line_bytes),
        longest_completed_line_bytes,
        longest_line_columns: longest_completed_line_columns.max(current_columns),
        longest_completed_line_columns,
    };
    let integrity_blocks = blocks
        .into_iter()
        .map(|block| block.digest)
        .collect::<Vec<_>>()
        .into();
    Ok((!is_cancelled()).then_some((indexed_lines, integrity_blocks)))
}

fn validate_indexed_file_snapshot(
    file: &File,
    file_size: u64,
    expected_modified: Option<SystemTime>,
    expected_identity: Option<&FileIdentity>,
    path: &Path,
) -> Result<()> {
    let current_metadata = file
        .metadata()
        .with_context(|| format!("建立索引后无法读取文件信息：{}", path.display()))?;
    if current_metadata.len() != file_size
        || current_metadata.modified().ok() != expected_modified
        || expected_identity
            .is_some_and(|expected| read_file_identity(file).as_ref() != Some(expected))
    {
        anyhow::bail!("日志文件在建立索引时发生了变化：{}", path.display());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn build_appended_file_index_with_integrity(
    file: &File,
    new_size: usize,
    expected_modified: Option<SystemTime>,
    expected_identity: Option<FileIdentity>,
    old_size: usize,
    old_integrity_blocks: &[[u8; 32]],
    mut indexer: StreamingLineIndexer,
    path: &Path,
) -> Result<Option<IndexedFileSnapshot>> {
    if old_size >= new_size
        || old_integrity_blocks.len() != old_size.div_ceil(APPEND_INTEGRITY_BLOCK_BYTES)
    {
        return Ok(None);
    }
    let rescan_start = indexer.current_start;
    let mut integrity_blocks = Vec::with_capacity(new_size.div_ceil(APPEND_INTEGRITY_BLOCK_BYTES));
    let mut block_start = 0usize;
    let mut block = vec![0_u8; new_size.min(APPEND_INTEGRITY_BLOCK_BYTES)];
    while block_start < new_size {
        let block_len = (new_size - block_start).min(APPEND_INTEGRITY_BLOCK_BYTES);
        block.resize(block_len, 0);
        read_file_exact_at(file, &mut block, block_start as u64)
            .with_context(|| format!("校验追加内容时无法读取日志文件：{}", path.display()))?;

        if block_start < old_size {
            let old_block_len = (old_size - block_start).min(APPEND_INTEGRITY_BLOCK_BYTES);
            let block_ix = block_start / APPEND_INTEGRITY_BLOCK_BYTES;
            let old_digest: [u8; 32] = Sha256::digest(&block[..old_block_len]).into();
            if old_integrity_blocks.get(block_ix) != Some(&old_digest) {
                return Ok(None);
            }
        }
        integrity_blocks.push(Sha256::digest(&block).into());

        let block_end = block_start.saturating_add(block_len);
        let scan_start = block_start.max(rescan_start);
        if scan_start < block_end {
            let slice_start = scan_start - block_start;
            indexer
                .feed(
                    scan_start,
                    &block[slice_start..],
                    block_end == new_size,
                    &|| false,
                )
                .expect("a non-cancelling append scan must complete");
        }
        block_start = block_end;
    }

    let current_metadata = file
        .metadata()
        .with_context(|| format!("校验追加后无法读取文件信息：{}", path.display()))?;
    if current_metadata.len() != new_size as u64
        || current_metadata.modified().ok() != expected_modified
        || expected_identity
            .as_ref()
            .is_some_and(|expected| read_file_identity(file).as_ref() != Some(expected))
    {
        anyhow::bail!("日志文件在校验追加时发生了变化：{}", path.display());
    }

    Ok(Some((indexer.finish(new_size), integrity_blocks.into())))
}

fn build_binary_line_index(
    file_size: usize,
    is_cancelled: &dyn Fn() -> bool,
) -> Option<IndexedLines> {
    if file_size == 0 {
        return Some(IndexedLines {
            starts: MutableLineStarts::Compact(Vec::new()),
            longest_line_bytes: 0,
            longest_completed_line_bytes: 0,
            longest_line_columns: 0,
            longest_completed_line_columns: 0,
        });
    }
    let line_count = file_size.div_ceil(BINARY_BYTES_PER_LINE);
    let mut starts = MutableLineStarts::with_capacity(file_size as u64, line_count);
    for line_ix in 0..line_count {
        if line_ix.is_multiple_of(INDEX_CANCELLATION_BATCH_LINES) && is_cancelled() {
            return None;
        }
        starts.push(line_ix.saturating_mul(BINARY_BYTES_PER_LINE));
    }
    let longest_line_bytes = file_size.min(BINARY_BYTES_PER_LINE);
    let longest_line_columns = longest_line_bytes.saturating_mul(3).saturating_sub(1);
    Some(IndexedLines {
        starts,
        longest_line_bytes,
        longest_completed_line_bytes: longest_line_bytes,
        longest_line_columns,
        longest_completed_line_columns: longest_line_columns,
    })
}

#[cfg(windows)]
fn read_file_identity(file: &File) -> Option<FileIdentity> {
    try_read_file_identity(file).ok()
}

#[cfg(windows)]
fn try_read_file_identity(file: &File) -> std::io::Result<FileIdentity> {
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
        return Err(std::io::Error::last_os_error());
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
    if !usn_ok {
        return Err(std::io::Error::last_os_error());
    }
    if returned < 32 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "file USN response is truncated",
        ));
    }
    let major = u16::from_le_bytes([output[4], output[5]]);
    let usn_offset = match major {
        2 => 24,
        3 => 40,
        _ => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "file USN response has an unsupported version",
            ));
        }
    };
    if (returned as usize) < usn_offset + 8 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "file USN response does not contain a complete sequence number",
        ));
    }
    let usn = i64::from_le_bytes(output[usn_offset..usn_offset + 8].try_into().map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "file USN response contains an invalid sequence number",
        )
    })?);
    Ok(FileIdentity {
        volume_serial: file_id.VolumeSerialNumber,
        file_id: file_id.FileId.Identifier,
        usn,
    })
}

#[cfg(unix)]
fn read_file_identity(file: &File) -> Option<FileIdentity> {
    try_read_file_identity(file).ok()
}

#[cfg(unix)]
fn try_read_file_identity(file: &File) -> std::io::Result<FileIdentity> {
    let metadata = file.metadata()?;
    let mut file_id = [0_u8; 16];
    file_id[..8].copy_from_slice(&metadata.ino().to_le_bytes());
    file_id[8..].copy_from_slice(&metadata.ctime_nsec().to_le_bytes());
    Ok(FileIdentity {
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
        fs::{self, File},
        path::PathBuf,
        sync::atomic::{AtomicUsize, Ordering},
        time::SystemTime,
    };

    use encoding_rs::SHIFT_JIS;

    use crate::CancellationToken;

    use super::{
        APPEND_INTEGRITY_BLOCK_BYTES, DocumentBytes, DocumentRefreshKind, FileEncoding,
        LinePreviewReader, LineReader, LogDocument, PARALLEL_INDEX_MIN_BYTES,
        build_file_index_with_integrity_while, build_line_index, build_line_index_while,
        calculate_integrity_blocks, read_file_identity,
    };

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

    fn assert_streaming_index_matches_slice(
        directory: &std::path::Path,
        name: &str,
        bytes: &[u8],
        encoding: FileEncoding,
    ) {
        let path = directory.join(name);
        fs::write(&path, bytes).expect("应能写入流式索引测试日志");
        let file = File::open(&path).expect("应能打开流式索引测试日志");
        let metadata = file.metadata().expect("应能读取流式索引测试元数据");
        let expected = build_line_index(bytes, encoding);

        let (actual, integrity_blocks) = build_file_index_with_integrity_while(
            &file,
            metadata.len(),
            metadata.modified().ok(),
            read_file_identity(&file),
            encoding,
            &path,
            &|| false,
        )
        .expect("流式索引不应失败")
        .expect("流式索引不应取消");

        assert_eq!(
            actual.starts.into_immutable().iter().collect::<Vec<_>>(),
            expected.starts.into_immutable().iter().collect::<Vec<_>>()
        );
        assert_eq!(actual.longest_line_bytes, expected.longest_line_bytes);
        assert_eq!(
            actual.longest_completed_line_bytes,
            expected.longest_completed_line_bytes
        );
        assert_eq!(actual.longest_line_columns, expected.longest_line_columns);
        assert_eq!(
            actual.longest_completed_line_columns,
            expected.longest_completed_line_columns
        );
        assert_eq!(integrity_blocks.as_ref(), calculate_integrity_blocks(bytes));
    }

    #[test]
    fn streaming_index_matches_slice_across_integrity_block_boundaries() {
        let directory = test_directory("streaming-index-boundaries");

        let mut utf8 = vec![b'a'; PARALLEL_INDEX_MIN_BYTES - 1];
        utf8.extend_from_slice(b"\r\ntail\tcolumn\rstandalone\n");
        assert_streaming_index_matches_slice(&directory, "utf8.log", &utf8, FileEncoding::Utf8);

        let mut utf16 = Vec::with_capacity(APPEND_INTEGRITY_BLOCK_BYTES + 16);
        for _ in 0..(APPEND_INTEGRITY_BLOCK_BYTES / 2 - 1) {
            utf16.extend_from_slice(&[b'a', 0]);
        }
        utf16.extend_from_slice(&[b'\r', 0, b'\n', 0, b'b', 0, b'\n', 0]);
        assert_streaming_index_matches_slice(
            &directory,
            "utf16.log",
            &utf16,
            FileEncoding::Utf16Le,
        );

        let mut shift_jis = vec![b'a'; APPEND_INTEGRITY_BLOCK_BYTES - 1];
        shift_jis.extend_from_slice(&[0x82, 0xa0, b'\n']);
        assert_streaming_index_matches_slice(
            &directory,
            "shift-jis.log",
            &shift_jis,
            FileEncoding::Legacy(SHIFT_JIS),
        );

        _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn positional_append_refresh_matches_a_complete_rebuild() {
        let directory = test_directory("positional-append");
        let path = directory.join("append.log");
        fs::write(&path, b"\xef\xbb\xbfalpha\nunfinished\r").expect("应能写入追加刷新测试日志");
        let original = LogDocument::open(&path).expect("应能打开追加刷新测试日志");
        fs::write(
            &path,
            b"\xef\xbb\xbfalpha\nunfinished\r\nnext\rstandalone\nlast",
        )
        .expect("应能追加刷新测试内容");

        let (refreshed, kind) = original.refresh().expect("位置读取追加刷新应成功");
        let rebuilt = LogDocument::open(&path).expect("完整重建应成功");

        assert_eq!(kind, DocumentRefreshKind::Appended);
        assert!(refreshed.same_source_snapshot(&rebuilt));
        assert_eq!(refreshed.line_count(), rebuilt.line_count());
        assert_eq!(
            refreshed.metadata().longest_line_bytes,
            rebuilt.metadata().longest_line_bytes
        );
        assert_eq!(
            refreshed.metadata().longest_line_columns,
            rebuilt.metadata().longest_line_columns
        );
        assert_eq!(
            (0..refreshed.line_count())
                .map(|row| refreshed.line(row))
                .collect::<Vec<_>>(),
            (0..rebuilt.line_count())
                .map(|row| rebuilt.line(row))
                .collect::<Vec<_>>()
        );
        _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn changed_prefix_with_a_larger_file_forces_a_rebuild() {
        let directory = test_directory("changed-append-prefix");
        let path = directory.join("append.log");
        fs::write(&path, b"old\n").expect("应能写入追加前缀测试日志");
        let original = LogDocument::open(&path).expect("应能打开追加前缀测试日志");
        fs::write(&path, b"NEW\nmore\n").expect("应能覆盖追加前缀测试日志");

        let (refreshed, kind) = original.refresh().expect("前缀变化后应能完整重建");

        assert_eq!(kind, DocumentRefreshKind::Rebuilt);
        assert_eq!(refreshed.line(0).as_deref(), Some("NEW"));
        _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn empty_source_growth_rebuilds_to_redetect_encoding() {
        let directory = test_directory("empty-growth-encoding");
        let path = directory.join("append.log");
        fs::write(&path, []).expect("应能创建空日志");
        let original = LogDocument::open(&path).expect("应能打开空日志");
        fs::write(&path, [0xff, 0xfe, b'a', 0, b'\n', 0]).expect("应能写入 UTF-16LE 日志");

        let (refreshed, kind) = original.refresh().expect("空日志增长后应能重建");

        assert_eq!(kind, DocumentRefreshKind::Rebuilt);
        assert_eq!(refreshed.metadata().encoding_name, "UTF-16LE BOM");
        assert_eq!(refreshed.line(0).as_deref(), Some("a"));
        _ = fs::remove_dir_all(directory);
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
    fn released_source_handles_retry_after_transient_path_unavailability() {
        let directory = test_directory("released-source-retry");
        let source_directory = directory.join("source");
        let parked_directory = directory.join("source-parked");
        fs::create_dir(&source_directory).expect("应能创建源目录");
        let path = source_directory.join("source.log");
        fs::write(&path, b"alpha\nbeta\n").expect("应能写入原始测试日志");
        let document = LogDocument::open(&path).expect("应能打开原始测试日志");
        document.release_source_handle();

        fs::rename(&source_directory, &parked_directory).expect("应能暂时移走源目录");
        assert_eq!(document.line(0), None);
        fs::rename(&parked_directory, &source_directory).expect("应能恢复同一个源目录");

        assert_eq!(document.line(0).as_deref(), Some("alpha"));
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
    fn task_local_line_reader_reuses_a_transient_verified_block() {
        let directory = test_directory("task-local-line-block");
        let path = directory.join("source.log");
        fs::write(&path, b"alpha\nbeta\n").expect("应能写入完整行读取器测试日志");
        let document = LogDocument::open(&path).expect("应能打开完整行读取器测试日志");
        document.release_source_handle();
        let mut reader = LineReader::default();

        assert_eq!(reader.line(&document, 0).as_deref(), Some("alpha"));
        fs::remove_file(&path).expect("首行读取后应已关闭瞬时源句柄");
        assert_eq!(reader.line(&document, 1).as_deref(), Some("beta"));
        assert!(document.line(1).is_none());
        _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn task_local_preview_reader_reuses_a_transient_verified_block() {
        let directory = test_directory("task-local-preview-block");
        let path = directory.join("source.log");
        fs::write(&path, b"alpha\nbeta\n").expect("应能写入预览读取器测试日志");
        let document = LogDocument::open(&path).expect("应能打开预览读取器测试日志");
        document.release_source_handle();
        let mut reader = LinePreviewReader::default();

        assert_eq!(
            reader
                .line_preview(&document, 0, 64)
                .map(|preview| preview.into_parts()),
            Some(("alpha".to_string(), false))
        );
        fs::remove_file(&path).expect("首行读取后应已关闭瞬时源句柄");
        assert_eq!(
            reader
                .line_preview(&document, 1, 64)
                .map(|preview| preview.into_parts()),
            Some(("beta".to_string(), false))
        );
        assert!(document.line_preview(1, 64).is_none());
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
