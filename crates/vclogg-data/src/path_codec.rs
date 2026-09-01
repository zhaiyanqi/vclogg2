//! Lossless, platform-aware path encoding for durable storage keys.

use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};

#[cfg(windows)]
use std::os::windows::ffi::{OsStrExt as _, OsStringExt as _};

const ENCODED_PATH_PREFIX: &str = "\0vclogg-path-v1:";

/// Encode a path into a persistence-safe string while leaving ordinary Unicode paths unchanged.
pub fn encode_persisted_path(path: &Path) -> String {
    if let Some(path) = path.to_str()
        && !path.starts_with(ENCODED_PATH_PREFIX)
    {
        return path.to_owned();
    }

    #[cfg(unix)]
    let (platform, bytes) = ("u:", path.as_os_str().as_bytes().to_vec());
    #[cfg(windows)]
    let (platform, bytes) = (
        "w:",
        path.as_os_str()
            .encode_wide()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>(),
    );
    #[cfg(not(any(unix, windows)))]
    let (platform, bytes) = ("o:", path.to_string_lossy().as_bytes().to_vec());

    let mut encoded = String::with_capacity(ENCODED_PATH_PREFIX.len() + 2 + bytes.len() * 2);
    encoded.push_str(ENCODED_PATH_PREFIX);
    encoded.push_str(platform);
    push_hex(&mut encoded, &bytes);
    encoded
}

/// Decode a path written by [`encode_persisted_path`], accepting legacy plain Unicode strings.
pub fn decode_persisted_path(stored: &str) -> PathBuf {
    #[cfg(unix)]
    if let Some(encoded) = stored
        .strip_prefix(ENCODED_PATH_PREFIX)
        .and_then(|encoded| encoded.strip_prefix("u:"))
        && let Some(bytes) = decode_hex(encoded)
    {
        return PathBuf::from(std::ffi::OsString::from_vec(bytes));
    }
    #[cfg(windows)]
    if let Some(encoded) = stored
        .strip_prefix(ENCODED_PATH_PREFIX)
        .and_then(|encoded| encoded.strip_prefix("w:"))
        && let Some(bytes) = decode_hex(encoded)
    {
        let (units, remainder) = bytes.as_slice().as_chunks::<2>();
        if remainder.is_empty() {
            let wide = units
                .iter()
                .map(|bytes| u16::from_le_bytes(*bytes))
                .collect::<Vec<_>>();
            return PathBuf::from(std::ffi::OsString::from_wide(&wide));
        }
    }
    PathBuf::from(stored)
}

fn push_hex(destination: &mut String, bytes: &[u8]) {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    for byte in bytes {
        destination.push(DIGITS[usize::from(byte >> 4)] as char);
        destination.push(DIGITS[usize::from(byte & 0x0f)] as char);
    }
}

fn decode_hex(encoded: &str) -> Option<Vec<u8>> {
    if !encoded.len().is_multiple_of(2) {
        return None;
    }
    let (pairs, remainder) = encoded.as_bytes().as_chunks::<2>();
    debug_assert!(remainder.is_empty());
    pairs
        .iter()
        .map(|digits| {
            let high = (digits[0] as char).to_digit(16)?;
            let low = (digits[1] as char).to_digit(16)?;
            Some(((high << 4) | low) as u8)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    use std::ffi::OsString;

    #[cfg(unix)]
    #[test]
    fn persisted_paths_preserve_non_utf8_identity() {
        let first = PathBuf::from(OsString::from_vec(b"source-\x80.log".to_vec()));
        let second = PathBuf::from(OsString::from_vec(b"source-\x81.log".to_vec()));

        assert_eq!(first.to_string_lossy(), second.to_string_lossy());
        assert_ne!(
            encode_persisted_path(&first),
            encode_persisted_path(&second)
        );
        assert_eq!(decode_persisted_path(&encode_persisted_path(&first)), first);
        assert_eq!(
            decode_persisted_path(&encode_persisted_path(&second)),
            second
        );
    }
}
