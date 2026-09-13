//! Bounded file navigation summaries. Call these services on a background worker.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

use anyhow::{Result, bail, ensure};

use crate::{CancellationToken, LinePreviewReader, LogDocument};

#[derive(Clone, Debug)]
pub struct DirectoryEntry {
    path: PathBuf,
    directory: bool,
    symlink: bool,
}

impl DirectoryEntry {
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn is_directory(&self) -> bool {
        self.directory
    }
    pub fn is_symlink(&self) -> bool {
        self.symlink
    }
}

/// Enumerate one directory without traversing child directories or symlinks.
pub fn navigation_directory(
    path: &Path,
    cancellation: &CancellationToken,
) -> Result<Option<Vec<DirectoryEntry>>> {
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(path)? {
        if cancellation.is_cancelled() {
            return Ok(None);
        }
        let entry = entry?;
        let kind = entry.file_type()?;
        entries.push(DirectoryEntry {
            path: entry.path(),
            directory: kind.is_dir(),
            symlink: kind.is_symlink(),
        });
    }
    entries.sort_by(|a, b| {
        b.directory
            .cmp(&a.directory)
            .then_with(|| a.path.file_name().cmp(&b.path.file_name()))
    });
    Ok(Some(entries))
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct LogMinute {
    year: Option<u16>,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    zone: Option<String>,
}

impl LogMinute {
    fn short_label(&self) -> String {
        format!(
            "{:02}-{:02} {:02}:{:02}",
            self.month, self.day, self.hour, self.minute
        )
    }

    /// Full identity for ambiguous years or zones; missing values are never inferred.
    pub fn label(&self) -> String {
        let date = self.short_label();
        let date = self
            .year
            .map_or_else(|| date.clone(), |year| format!("{year:04}-{date}"));
        self.zone
            .as_ref()
            .map_or_else(|| date.clone(), |zone| format!("{date} {zone}"))
    }
}

#[derive(Clone, Debug)]
pub struct MinuteGroup {
    minute: LogMinute,
    first_row: usize,
    count: usize,
    ambiguous: bool,
}

impl MinuteGroup {
    pub fn label(&self) -> String {
        if self.ambiguous {
            self.minute.label()
        } else {
            self.minute.short_label()
        }
    }
    pub fn minute(&self) -> &LogMinute {
        &self.minute
    }
    pub fn first_row(&self) -> usize {
        self.first_row
    }
    pub fn count(&self) -> usize {
        self.count
    }
}

#[derive(Clone, Debug, Default)]
pub struct OverviewBucket {
    rows: u64,
    columns: u64,
    nonempty: u64,
}

impl OverviewBucket {
    pub fn width_fraction(&self) -> f32 {
        self.columns as f32 / (self.rows.max(1) * 256) as f32
    }
    pub fn density(&self) -> f32 {
        self.nonempty as f32 / self.rows.max(1) as f32
    }
}

#[derive(Clone, Debug)]
pub struct NavigationSummary {
    groups: Vec<MinuteGroup>,
    buckets: Vec<OverviewBucket>,
    bucket_rows: usize,
    line_count: usize,
    tail: Option<(Option<LogMinute>, u64, u64)>,
}

impl Default for NavigationSummary {
    fn default() -> Self {
        Self {
            groups: Vec::new(),
            buckets: Vec::new(),
            bucket_rows: 1,
            line_count: 0,
            tail: None,
        }
    }
}

impl NavigationSummary {
    pub fn groups(&self) -> &[MinuteGroup] {
        &self.groups
    }
    pub fn buckets(&self) -> &[OverviewBucket] {
        &self.buckets
    }
    pub fn bucket_rows(&self) -> usize {
        self.bucket_rows
    }
    pub fn line_count(&self) -> usize {
        self.line_count
    }
}

/// Build a full-source minute index and at most 4096 overview buckets.
/// `appended` may only be supplied after `DocumentRefreshKind::Appended` for the
/// exact source snapshot that produced that summary. Reprocess the previous tail
/// because an append can finish an unterminated timestamp or line.
pub fn summarize_navigation(
    document: &LogDocument,
    appended: Option<&NavigationSummary>,
    cancellation: &CancellationToken,
    progress: &AtomicUsize,
) -> Result<Option<NavigationSummary>> {
    ensure!(
        document.has_complete_line_index(),
        "Navigation requires a complete line index"
    );
    let mut summary = appended.cloned().unwrap_or_default();
    ensure!(
        summary.line_count <= document.line_count(),
        "Navigation source was truncated"
    );
    let start = summary.line_count.saturating_sub(1);
    if let Some((minute, columns, nonempty)) = summary.tail.take() {
        if let Some(minute) = minute {
            if let Some(group) = summary
                .groups
                .iter_mut()
                .find(|group| group.minute == minute)
            {
                group.count -= 1;
            }
            summary.groups.retain(|group| group.count > 0);
        }
        if let Some(bucket) = summary.buckets.get_mut(start / summary.bucket_rows) {
            bucket.rows -= 1;
            bucket.columns -= columns;
            bucket.nonempty -= nonempty;
        }
    }
    let mut groups: HashMap<LogMinute, usize> = summary
        .groups
        .iter()
        .enumerate()
        .map(|(ix, group)| (group.minute.clone(), ix))
        .collect();
    let mut reader = LinePreviewReader::default();
    for row in start..document.line_count() {
        if cancellation.is_cancelled() {
            return Ok(None);
        }
        let Some(line) = reader.line_preview(document, row, 1024) else {
            bail!("Navigation source changed at line {}", row + 1);
        };
        let minute = parse_log_minute(line.text());
        if let Some(minute) = &minute {
            let ix = *groups.entry(minute.clone()).or_insert_with(|| {
                summary.groups.push(MinuteGroup {
                    minute: minute.clone(),
                    first_row: row,
                    count: 0,
                    ambiguous: false,
                });
                summary.groups.len() - 1
            });
            summary.groups[ix].count += 1;
        }
        while row / summary.bucket_rows >= 4096 {
            summary.buckets = summary
                .buckets
                .chunks(2)
                .map(|pair| OverviewBucket {
                    rows: pair.iter().map(|bucket| bucket.rows).sum(),
                    columns: pair.iter().map(|bucket| bucket.columns).sum(),
                    nonempty: pair.iter().map(|bucket| bucket.nonempty).sum(),
                })
                .collect();
            summary.bucket_rows *= 2;
        }
        let ix = row / summary.bucket_rows;
        summary.buckets.resize_with(ix + 1, OverviewBucket::default);
        let columns = line.text().chars().take(256).count() as u64;
        let nonempty = u64::from(!line.text().trim().is_empty());
        let bucket = &mut summary.buckets[ix];
        bucket.rows += 1;
        bucket.columns += columns;
        bucket.nonempty += nonempty;
        summary.tail = Some((minute, columns, nonempty));
        if row % 1024 == 0 {
            progress.store(row + 1, Ordering::Relaxed);
        }
    }
    if cancellation.is_cancelled() {
        return Ok(None);
    }
    ensure!(
        document.source_identity_matches(),
        "Navigation source changed during indexing"
    );
    let mut short_labels = HashMap::new();
    for group in &summary.groups {
        if cancellation.is_cancelled() {
            return Ok(None);
        }
        *short_labels
            .entry((
                group.minute.month,
                group.minute.day,
                group.minute.hour,
                group.minute.minute,
            ))
            .or_insert(0_usize) += 1;
    }
    for group in &mut summary.groups {
        if cancellation.is_cancelled() {
            return Ok(None);
        }
        group.ambiguous = short_labels[&(
            group.minute.month,
            group.minute.day,
            group.minute.hour,
            group.minute.minute,
        )] > 1;
    }
    summary.line_count = document.line_count();
    progress.store(summary.line_count, Ordering::Relaxed);
    Ok(Some(summary))
}

/// Parse only a leading date/time, optionally enclosed in square brackets.
/// Missing years and explicit zones remain distinct rather than being inferred.
pub fn parse_log_minute(text: &str) -> Option<LogMinute> {
    let text = text.trim_start();
    let bracketed = text.starts_with('[');
    let text = if bracketed { &text[1..] } else { text };
    let bytes = text.as_bytes();
    let number = |start: usize, len: usize| -> Option<u16> {
        bytes
            .get(start..start + len)?
            .iter()
            .try_fold(0_u16, |value, digit| {
                digit
                    .is_ascii_digit()
                    .then(|| value * 10 + u16::from(digit - b'0'))
            })
    };
    let (year, offset) = if bytes.get(4) == Some(&b'-') {
        (Some(number(0, 4)?), 5)
    } else {
        (None, 0)
    };
    if bytes.get(offset + 2) != Some(&b'-')
        || !matches!(bytes.get(offset + 5), Some(b' ' | b'T'))
        || bytes.get(offset + 8) != Some(&b':')
        || bytes.get(offset + 11) != Some(&b':')
    {
        return None;
    }
    let month = number(offset, 2)? as u8;
    let day = number(offset + 3, 2)? as u8;
    let hour = number(offset + 6, 2)? as u8;
    let minute = number(offset + 9, 2)? as u8;
    let second = number(offset + 12, 2)?;
    let leap = year.is_none_or(|year| year % 4 == 0 && (year % 100 != 0 || year % 400 == 0));
    let days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if !(1..=12).contains(&month)
        || day == 0
        || day > days[month as usize - 1]
        || hour > 23
        || minute > 59
        || second > 59
        || year == Some(0)
    {
        return None;
    }
    let mut end = offset + 14;
    if matches!(bytes.get(end), Some(b'.' | b',')) {
        end += 1;
        let start = end;
        while bytes.get(end).is_some_and(u8::is_ascii_digit) {
            end += 1;
        }
        if start == end {
            return None;
        }
    }
    let zone = match bytes.get(end) {
        Some(b'Z') => {
            end += 1;
            Some("Z".to_string())
        }
        Some(b'+' | b'-') => {
            let start = end;
            let hours = number(end + 1, 2)?;
            let colon = bytes.get(end + 3) == Some(&b':');
            let minutes = number(end + if colon { 4 } else { 3 }, 2)?;
            if hours > 23 || minutes > 59 {
                return None;
            }
            end += if colon { 6 } else { 5 };
            Some(format!(
                "{}{:02}:{:02}",
                char::from(bytes[start]),
                hours,
                minutes
            ))
        }
        _ => None,
    };
    if bracketed && bytes.get(end) != Some(&b']') {
        return None;
    }
    if !bracketed
        && bytes
            .get(end)
            .is_some_and(|byte| !byte.is_ascii_whitespace())
    {
        return None;
    }
    Some(LogMinute {
        year,
        month,
        day,
        hour,
        minute,
        zone,
    })
}
