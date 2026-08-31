use std::{
    fmt::Write as _,
    fs::{self, File},
    io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write},
    ops::{ControlFlow, Deref},
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(windows)]
use std::{os::windows::ffi::OsStrExt as _, os::windows::io::AsRawHandle as _};

#[cfg(unix)]
use std::os::unix::fs::MetadataExt as _;

use anyhow::{Context as _, Result};
use chardetng::EncodingDetector;
use encoding_rs::{Encoding, UTF_8, UTF_16BE, UTF_16LE};
use memchr::{memchr_iter, memchr2};
use memmap2::{Mmap, MmapOptions};
use sha2::{Digest as _, Sha256};

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
const INDEX_CACHE_MAGIC: &[u8; 8] = b"VCLOGG05";
const INDEX_CACHE_VERSION: u32 = 2;
const INDEX_CACHE_HEADER_BYTES: u64 = 8 + 4 + 4 + 2 + 8 + 8 + 1 + 8 + 16 + 8 + 8 * 7;
const MAX_CACHE_PATH_BYTES: usize = 32 * 1024;
const MAX_CACHE_ENCODING_BYTES: usize = 64;
const ENCODING_DETECTION_BYTES: usize = 1024 * 1024;
const BINARY_BYTES_PER_LINE: usize = 16;
const PARALLEL_INDEX_HASH_MIN_BYTES: usize = 8 * 1024 * 1024;

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
    Mapped(Mmap),
    Owned(Box<[u8]>),
}

impl Deref for DocumentBytes {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Empty => &[],
            Self::Mapped(bytes) => bytes,
            Self::Owned(bytes) => bytes,
        }
    }
}

struct AppendFingerprint {
    head: Arc<[u8]>,
    tail: Arc<[u8]>,
    integrity_blocks: Arc<[[u8; 32]]>,
}

struct IndexedLines {
    starts: Vec<usize>,
    longest_line_bytes: usize,
    longest_completed_line_bytes: usize,
    longest_line_columns: usize,
    longest_completed_line_columns: usize,
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
    line_starts: Arc<[usize]>,
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

/// A read-only log snapshot backed by a memory map and a line-start index.
///
/// The document owns no UI state. Reopening a changed source creates a new snapshot,
/// which lets the application swap generations atomically after background work ends.
pub struct LogDocument {
    bytes: Arc<DocumentBytes>,
    line_starts: Arc<[usize]>,
    segment_start_row: usize,
    metadata: DocumentMetadata,
    longest_completed_line_bytes: usize,
    longest_completed_line_columns: usize,
    append_fingerprint: AppendFingerprint,
    content_digest: [u8; 32],
    encoding: FileEncoding,
}

impl LogDocument {
    /// Create an I/O-free shell used to register a stable opening document
    /// before its bounded preview starts.
    pub fn placeholder(path: impl AsRef<Path>) -> Self {
        Self::from_parts(
            path.as_ref().to_path_buf(),
            DocumentBytes::Empty,
            IndexedLines {
                starts: Vec::new(),
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
        let path = path.as_ref().to_path_buf();
        let mut file =
            File::open(&path).with_context(|| format!("无法打开日志文件：{}", path.display()))?;
        let file_metadata = file
            .metadata()
            .with_context(|| format!("无法读取文件信息：{}", path.display()))?;

        let encoding = detect_file_encoding(&mut file, file_metadata.len())
            .with_context(|| format!("无法检测日志编码：{}", path.display()))?;
        let bytes = map_snapshot(&file, file_metadata.len(), &path)?;
        let (indexed_lines, integrity_blocks) = build_line_index_with_integrity(&bytes, encoding);

        Ok(Self::from_parts(
            path,
            bytes,
            indexed_lines,
            file_metadata.modified().ok(),
            file_metadata.len(),
            encoding,
            Some(integrity_blocks),
        ))
    }

    /// Open a complete document, reusing a line index only when the cached
    /// platform file identity still exactly matches the source.
    pub fn open_with_index_cache(
        path: impl AsRef<Path>,
        cache_dir: impl AsRef<Path>,
    ) -> Result<(Self, Option<PendingIndexCacheWrite>)> {
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
        let cached = read_index_cache(
            &cache_path,
            &path,
            file_size,
            modified_millis,
            identity.as_ref(),
        );

        let cache_missed = cached.is_none();
        let encoding = match cached.as_ref() {
            Some(cached) => cached.encoding,
            None => detect_file_encoding(&mut file, file_size)
                .with_context(|| format!("无法检测日志编码：{}", path.display()))?,
        };
        let bytes = map_snapshot(&file, file_size, &path)?;
        let (indexed_lines, integrity_blocks) = match cached {
            Some(cached) => (cached.indexed_lines, Some(cached.integrity_blocks)),
            None => {
                let (indexed_lines, integrity_blocks) =
                    build_line_index_with_integrity(&bytes, encoding);
                (indexed_lines, Some(integrity_blocks))
            }
        };
        let document = Self::from_parts(
            path.clone(),
            bytes,
            indexed_lines,
            modified,
            file_size,
            encoding,
            integrity_blocks,
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
        Ok((document, pending_cache_write))
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
        let mut indexed_lines = build_line_index(&bytes, encoding);
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
    ) -> Result<Option<Self>> {
        if line_limit == 0 {
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
        let start_row = anchor
            .saturating_sub(window_count / 2)
            .min(source_line_count - window_count);
        let end_row = start_row + window_count;
        let byte_start = cached.starts[start_row];
        let byte_end = if end_row < source_line_count {
            cached.starts[end_row]
        } else {
            usize::try_from(source_size)
                .with_context(|| format!("日志文件过大，无法读取缓存预览：{}", path.display()))?
        };
        let byte_count = byte_end
            .checked_sub(byte_start)
            .with_context(|| format!("索引缓存中的预览范围无效：{}", cache_path.display()))?;
        file.seek(SeekFrom::Start(byte_start as u64))?;
        let mut preview = vec![0_u8; byte_count];
        file.read_exact(&mut preview)
            .with_context(|| format!("无法读取缓存日志预览：{}", path.display()))?;

        let starts = cached.starts[start_row..end_row]
            .iter()
            .map(|offset| offset - byte_start)
            .collect();
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
        self.segment_start_row
    }

    pub fn source_row(&self, local_row: usize) -> Option<usize> {
        (local_row < self.line_count()).then(|| self.segment_start_row.saturating_add(local_row))
    }

    pub fn local_row(&self, source_row: usize) -> Option<usize> {
        source_row
            .checked_sub(self.segment_start_row)
            .filter(|local_row| *local_row < self.line_count())
    }

    pub fn contains_source_row(&self, source_row: usize) -> bool {
        self.local_row(source_row).is_some()
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

    fn contains_complete_source(&self) -> bool {
        self.segment_start_row == 0 && self.bytes.len() as u64 == self.metadata.file_size
    }

    /// Decode one logical line without retaining its text in memory.
    pub fn line(&self, source_row: usize) -> Option<String> {
        self.line_bytes(source_row)
            .map(|bytes| self.encoding.decode(bytes))
    }

    /// Decode a bounded prefix of one logical line for interactive display.
    pub fn line_preview(&self, source_row: usize, max_bytes: usize) -> Option<LinePreview> {
        self.line_bytes(source_row)
            .map(|bytes| self.encoding.decode_preview(bytes, max_bytes))
    }

    pub(crate) fn line_bytes(&self, source_row: usize) -> Option<&[u8]> {
        let row_ix = self.local_row(source_row)?;
        self.line_bytes_at_local_row(row_ix)
    }

    fn line_bytes_at_local_row(&self, row_ix: usize) -> Option<&[u8]> {
        let mut start = *self.line_starts.get(row_ix)?;
        let end = self
            .line_starts
            .get(row_ix + 1)
            .copied()
            .unwrap_or(self.bytes.len());
        if self.segment_start_row == 0 && row_ix == 0 {
            start = start.saturating_add(self.encoding.bom_len()).min(end);
        }
        self.bytes
            .get(start..end)
            .map(|bytes| self.encoding.trim_line_bytes(bytes))
    }

    pub(crate) fn search_bytes_at_local_row(
        &self,
        row_ix: usize,
    ) -> Option<std::borrow::Cow<'_, [u8]>> {
        let bytes = self.line_bytes_at_local_row(row_ix)?;
        match self.encoding {
            FileEncoding::Utf8 | FileEncoding::Utf8Bom => Some(std::borrow::Cow::Borrowed(bytes)),
            FileEncoding::Utf16Le
            | FileEncoding::Utf16Be
            | FileEncoding::Legacy(_)
            | FileEncoding::Binary => Some(std::borrow::Cow::Owned(
                self.encoding.decode(bytes).into_bytes(),
            )),
        }
    }

    fn try_refresh_appended(&self) -> Result<Option<Self>> {
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

        let bytes = map_snapshot(&file, new_size, path)?;
        if !self.append_prefix_matches(&bytes) {
            return Ok(None);
        }

        let old_size = usize::try_from(old_size)
            .with_context(|| format!("日志文件过大，无法建立索引：{}", path.display()))?;
        let mut line_starts = self.line_starts.to_vec();
        let (
            longest_line_bytes,
            longest_completed_line_bytes,
            longest_line_columns,
            longest_completed_line_columns,
        ) = extend_line_index(
            &bytes,
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

        Ok(Some(Self::from_parts(
            path.to_path_buf(),
            bytes,
            indexed_lines,
            file_metadata.modified().ok(),
            new_size,
            self.encoding,
            None,
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
        let integrity_blocks =
            integrity_blocks.unwrap_or_else(|| calculate_integrity_blocks(&bytes).into());
        let content_digest = digest_integrity_blocks(&integrity_blocks);
        let append_fingerprint = AppendFingerprint {
            head: Arc::from(&bytes[..sample_len]),
            tail: Arc::from(&bytes[tail_start..]),
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
            line_starts: indexed_lines.starts.into(),
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
    bytes
        .chunks(APPEND_INTEGRITY_BLOCK_BYTES)
        .map(|block| Sha256::digest(block).into())
        .collect()
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

fn build_line_index_with_integrity(
    bytes: &[u8],
    encoding: FileEncoding,
) -> (IndexedLines, Arc<[[u8; 32]]>) {
    let parallel = bytes.len() >= PARALLEL_INDEX_HASH_MIN_BYTES
        && std::thread::available_parallelism().is_ok_and(|parallelism| parallelism.get() > 1);
    if !parallel {
        return (
            build_line_index(bytes, encoding),
            calculate_integrity_blocks(bytes).into(),
        );
    }

    std::thread::scope(|scope| {
        let integrity_task = scope.spawn(|| calculate_integrity_blocks(bytes));
        let indexed_lines = build_line_index(bytes, encoding);
        let integrity_blocks = integrity_task
            .join()
            .expect("日志完整性摘要线程不应异常终止")
            .into();
        (indexed_lines, integrity_blocks)
    })
}

fn integrity_blocks_match(bytes: &[u8], expected: &[[u8; 32]]) -> bool {
    bytes.len().div_ceil(APPEND_INTEGRITY_BLOCK_BYTES) == expected.len()
        && bytes
            .chunks(APPEND_INTEGRITY_BLOCK_BYTES)
            .zip(expected)
            .all(|(block, expected)| <[u8; 32]>::from(Sha256::digest(block)) == *expected)
}

fn map_snapshot(file: &File, file_size: u64, path: &Path) -> Result<DocumentBytes> {
    if file_size == 0 {
        return Ok(DocumentBytes::Empty);
    }
    let mapped_len = usize::try_from(file_size)
        .with_context(|| format!("日志文件过大，无法映射：{}", path.display()))?;

    // SAFETY: The mapping is read-only and its length is fixed to the metadata
    // captured above. VCLogg2 never exposes mutable access to mapped bytes.
    unsafe { MmapOptions::new().len(mapped_len).map(file) }
        .map(DocumentBytes::Mapped)
        .with_context(|| format!("无法映射日志文件：{}", path.display()))
}

fn index_cache_path(cache_dir: &Path, source_path: &Path) -> PathBuf {
    let source = source_path.to_string_lossy();
    let hash = source
        .as_bytes()
        .iter()
        .fold(0xcbf29ce484222325_u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        });
    cache_dir.join(format!("{hash:016x}.vclog-index"))
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
    if cached_path != source_path.to_string_lossy().as_bytes() {
        return None;
    }
    let mut cached_encoding = vec![0_u8; encoding_len];
    reader.read_exact(&mut cached_encoding).ok()?;
    let encoding = FileEncoding::from_cache_name(&cached_encoding)?;

    let mut starts = Vec::with_capacity(line_count);
    let mut previous = 0_u64;
    let mut consumed = 0_usize;
    for line_ix in 0..line_count {
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
    if (!starts.is_empty() && starts[0] != 0)
        || starts.windows(2).any(|pair| pair[0] >= pair[1])
        || starts
            .last()
            .is_some_and(|offset| u64::try_from(*offset).map_or(true, |offset| offset > file_size))
    {
        return None;
    }

    let mut integrity_blocks = Vec::with_capacity(integrity_block_count);
    for _ in 0..integrity_block_count {
        let mut digest = [0_u8; 32];
        reader.read_exact(&mut digest).ok()?;
        integrity_blocks.push(digest);
    }

    Some(CachedIndex {
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
    let source_path = pending.source_path.to_string_lossy();
    let source_path = source_path.as_bytes();
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
        writer.write_all(source_path)?;
        writer.write_all(encoding_name)?;
        let mut previous = 0_u64;
        for offset in pending.line_starts.iter().copied() {
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

fn encoded_offsets_len(offsets: &[usize]) -> usize {
    let mut previous = 0_u64;
    offsets.iter().copied().fold(0, |total, offset| {
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
    if bytes.is_empty() {
        return IndexedLines {
            starts: Vec::new(),
            longest_line_bytes: 0,
            longest_completed_line_bytes: 0,
            longest_line_columns: 0,
            longest_completed_line_columns: 0,
        };
    }

    if matches!(encoding, FileEncoding::Binary) {
        let starts = (0..bytes.len())
            .step_by(BINARY_BYTES_PER_LINE)
            .collect::<Vec<_>>();
        let longest_line_bytes = bytes.len().min(BINARY_BYTES_PER_LINE);
        let longest_line_columns = longest_line_bytes.saturating_mul(3).saturating_sub(1);
        return IndexedLines {
            starts,
            longest_line_bytes,
            longest_completed_line_bytes: longest_line_bytes,
            longest_line_columns,
            longest_completed_line_columns: longest_line_columns,
        };
    }

    let mut line_starts = vec![0];
    let mut longest_completed_line_bytes = 0;
    let mut longest_completed_line_columns = 0;
    let mut current_start = 0;

    _ = for_each_line_break(encoding, bytes, |line_end, width| {
        longest_completed_line_bytes =
            longest_completed_line_bytes.max(line_end.saturating_sub(current_start));
        longest_completed_line_columns =
            longest_completed_line_columns.max(display_columns_encoded(
                encoding,
                &bytes[current_start..line_end],
                current_start == 0,
            ));

        current_start = line_end + width;
        if current_start <= bytes.len() && line_starts.last().copied() != Some(current_start) {
            line_starts.push(current_start);
        }
        ControlFlow::Continue(())
    });

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

    IndexedLines {
        starts: line_starts,
        longest_line_bytes,
        longest_completed_line_bytes,
        longest_line_columns,
        longest_completed_line_columns,
    }
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
    line_starts: &mut Vec<usize>,
    mut longest_completed_line_bytes: usize,
    mut longest_completed_line_columns: usize,
) -> (usize, usize, usize, usize) {
    let mut current_start = if old_size == 0 {
        line_starts.push(0);
        0
    } else if bytes.get(old_size - 1) == Some(&b'\n') {
        if line_starts.last().copied() != Some(old_size) {
            line_starts.push(old_size);
        }
        old_size
    } else {
        line_starts.last().copied().unwrap_or_default()
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
        if current_start <= bytes.len() && line_starts.last().copied() != Some(current_start) {
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
    use std::{fs, path::PathBuf, time::SystemTime};

    use super::LogDocument;

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

        let preview = LogDocument::open_cached_preview(&path, &cache_directory, 2, 1)
            .expect("应能读取缓存预览")
            .expect("缓存预览应存在");

        assert_eq!(preview.segment_start_row(), 2);
        assert_eq!(preview.source_row(0), Some(2));
        assert_eq!(preview.local_row(2), Some(0));
        assert_eq!(preview.local_row(1), None);
        assert_eq!(preview.local_row(3), None);
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

    use super::{
        APPEND_INTEGRITY_BLOCK_BYTES, FileEncoding, FileIdentity, PendingIndexCacheWrite,
        read_index_cache,
    };

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
            line_starts: Arc::from([0, 11, 23]),
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
        assert_eq!(cached.indexed_lines.starts, [0, 11, 23]);
        assert_eq!(cached.integrity_blocks.as_ref(), &[[3; 32], [5; 32]]);
        _ = fs::remove_dir_all(directory);
    }
}
