//! Persistent line-index cache format and atomic storage.

use super::*;

pub(super) fn index_cache_path(cache_dir: &Path, source_path: &Path) -> PathBuf {
    let source_identity = index_cache_source_identity(source_path);
    let digest = Sha256::digest(&source_identity);
    let mut hash = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(&mut hash, "{byte:02x}").expect("writing to String cannot fail");
    }
    cache_dir.join(format!("{hash}.vclog-index"))
}

pub(super) fn index_cache_source_identity(source_path: &Path) -> Vec<u8> {
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

pub(super) fn system_time_millis(time: Option<SystemTime>) -> u64 {
    time.and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

pub(super) fn read_index_cache(
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

pub(super) fn read_index_cache_while(
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

pub(super) fn write_index_cache(pending: &PendingIndexCacheWrite) -> Result<()> {
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
