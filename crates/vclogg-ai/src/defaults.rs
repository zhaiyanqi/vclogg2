//! Upgrade shipped Markdown only while the local copy still equals a known default.
use anyhow::{Context as _, Result};
use std::{
    fs,
    io::{Read as _, Write as _},
    path::Path,
};

pub(crate) fn update_default(
    path: &Path,
    current: &str,
    legacy: &[&str],
    create: bool,
) -> Result<bool> {
    let baseline = path.with_extension("default.md");
    let existing = match read_default_file(path) {
        Ok(text) => Some(text),
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
        {
            None
        }
        Err(error) => return Err(error),
    };
    // A missing file of an already-installed skill may be a deliberate deletion.
    if existing.is_none() && !create {
        return Ok(false);
    }
    let previous = if baseline.is_file() {
        Some(read_default_file(&baseline)?)
    } else {
        None
    };
    let replace = existing.as_ref().is_some_and(|text| {
        text != current && (previous.as_ref() == Some(text) || legacy.contains(&text.as_str()))
    });
    let mut changed = false;
    if existing.is_none() {
        fs::create_dir_all(path.parent().context("Missing default directory")?)?;
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        match options.open(path) {
            Ok(mut file) => {
                file.write_all(current.as_bytes())?;
                changed = true;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return Ok(false),
            Err(e) => return Err(e.into()),
        }
    } else if replace {
        // Recheck before replacing so a concurrent editor's saved change wins.
        if read_default_file(path)?.as_str() != existing.as_deref().unwrap_or_default() {
            return Ok(false);
        }
        crate::model::private_write(path, current.as_bytes())?;
        changed = true;
    }
    // Do not record a baseline for unknown user content: a later release must not
    // mistake that content for a shipped version and overwrite it.
    if (changed || existing.as_deref() == Some(current)) && previous.as_deref() != Some(current) {
        crate::model::private_write(&baseline, current.as_bytes())?;
    }
    Ok(changed)
}

fn read_default_file(path: &Path) -> Result<String> {
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take((crate::RESULT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() <= crate::RESULT_BYTES,
        "Default document exceeds 64 KiB"
    );
    String::from_utf8(bytes).context("Default document must be UTF-8")
}
