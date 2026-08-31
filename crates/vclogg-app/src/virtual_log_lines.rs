use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet},
    ops::Range,
};

use vclogg_core::{CompressedRows, LinePreview};

use crate::selectable_log_text::LogText;

/// Stable identity shared by local logs, projected results, and global result groups.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum LogRowKey {
    Row { document_id: u64, source_row: usize },
    FileGroup { document_id: u64 },
}

/// Maps virtual-list coordinates to source-file rows without owning presentation state.
#[derive(Clone)]
pub(crate) enum LogRowProjection {
    All,
    SourceRows(CompressedRows),
}

const DEFAULT_MAX_LINE_SOURCE_BYTES: usize = 64 * 1024;
const DEFAULT_MAX_CACHE_RETAINED_BYTES: usize = 32 * 1024 * 1024;

struct CachedLogLine {
    text: LogText,
    retained_bytes: usize,
}

impl CachedLogLine {
    fn from_preview(preview: LinePreview) -> Self {
        let (text, truncated) = preview.into_parts();
        let text = LogText::preview(text, truncated);
        Self {
            retained_bytes: text.retained_bytes(),
            text,
        }
    }
}

/// Keeps byte-bounded decoded previews for the active virtual-list window.
///
/// Highlighting, selection, markers, typography, and wrapping are deliberately excluded so a
/// presentation-only change never causes the source file to be read again. Direct consumers such
/// as copy commands read the authoritative document directly. Visible rows are loaded before
/// overscan rows, and retained preview payloads never exceed the cache byte budget. The prepared
/// window is the only cache writer: incidental reads outside that window return a bounded preview
/// without displacing visible rows.
pub(crate) struct VisibleLineStore<K> {
    lines: RefCell<BTreeMap<K, CachedLogLine>>,
    window: Cell<Option<(usize, usize, usize, usize)>>,
    overscan: Cell<usize>,
    max_line_source_bytes: Cell<usize>,
    max_cache_retained_bytes: Cell<usize>,
}

impl<K> Default for VisibleLineStore<K> {
    fn default() -> Self {
        Self {
            lines: RefCell::default(),
            window: Cell::default(),
            overscan: Cell::new(12),
            max_line_source_bytes: Cell::new(DEFAULT_MAX_LINE_SOURCE_BYTES),
            max_cache_retained_bytes: Cell::new(DEFAULT_MAX_CACHE_RETAINED_BYTES),
        }
    }
}

impl<K: Clone + Ord> VisibleLineStore<K> {
    pub(crate) fn set_overscan(&self, overscan: usize) {
        if self.overscan.replace(overscan) != overscan {
            self.invalidate_window();
        }
    }

    pub(crate) fn invalidate_window(&self) {
        self.window.set(None);
    }

    pub(crate) fn clear(&self) {
        self.lines.borrow_mut().clear();
        self.invalidate_window();
    }

    pub(crate) fn retain(&self, mut keep: impl FnMut(&K) -> bool) {
        self.lines.borrow_mut().retain(|key, _| keep(key));
        self.invalidate_window();
    }

    pub(crate) fn line(
        &self,
        key: K,
        load: impl FnOnce(usize) -> Option<LinePreview>,
    ) -> Option<LogText> {
        if let Some(line) = self.lines.borrow().get(&key) {
            return Some(line.text.clone());
        }
        let byte_budget = self.max_cache_retained_bytes.get();
        let source_limit = self.max_line_source_bytes.get().min(byte_budget);
        Some(CachedLogLine::from_preview(load(source_limit)?).text)
    }

    pub(crate) fn prepare_visible_rows(
        &self,
        visible_range: Range<usize>,
        row_count: usize,
        mut key_for_row: impl FnMut(usize) -> Option<K>,
        mut load: impl FnMut(&K, usize) -> Option<LinePreview>,
    ) {
        let visible_start = visible_range.start.min(row_count);
        let visible_end = visible_range.end.min(row_count).max(visible_start);
        let start = visible_start.saturating_sub(self.overscan.get());
        let end = visible_end
            .saturating_add(self.overscan.get())
            .min(row_count);
        let window = (visible_start, visible_end, start, end);
        if self.window.replace(Some(window)) == Some(window) {
            return;
        }

        let mut seen = BTreeSet::new();
        let mut priority_keys = Vec::new();
        for row_ix in (visible_start..visible_end)
            .chain((start..visible_start).rev())
            .chain(visible_end..end)
        {
            if let Some(key) = key_for_row(row_ix)
                && seen.insert(key.clone())
            {
                priority_keys.push(key);
            }
        }

        let byte_budget = self.max_cache_retained_bytes.get();
        let mut previous = std::mem::take(&mut *self.lines.borrow_mut());
        let mut next = BTreeMap::new();
        let mut retained_bytes = 0usize;
        for key in priority_keys {
            if retained_bytes >= byte_budget {
                break;
            }
            let remaining = byte_budget - retained_bytes;
            let line = previous.remove(&key).or_else(|| {
                let source_limit = self.max_line_source_bytes.get().min(remaining);
                load(&key, source_limit).map(CachedLogLine::from_preview)
            });
            let Some(line) = line else {
                continue;
            };
            if line.retained_bytes <= remaining {
                retained_bytes = retained_bytes.saturating_add(line.retained_bytes);
                next.insert(key, line);
            }
        }
        *self.lines.borrow_mut() = next;
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use vclogg_core::LinePreview;

    use super::VisibleLineStore;

    #[test]
    fn reads_only_the_active_virtual_window() {
        let cache = VisibleLineStore::<usize>::default();
        cache.set_overscan(1);
        let loaded = RefCell::new(Vec::new());

        cache.prepare_visible_rows(10..13, 100, Some, |source_row, _| {
            loaded.borrow_mut().push(*source_row);
            Some(LinePreview::new(format!("line {source_row}"), false))
        });
        assert_eq!(*loaded.borrow(), vec![10, 11, 12, 9, 13]);

        cache.prepare_visible_rows(10..13, 100, Some, |_, _| {
            panic!("an unchanged virtual window must not be read twice")
        });
        cache.prepare_visible_rows(12..14, 100, Some, |source_row, _| {
            loaded.borrow_mut().push(*source_row);
            Some(LinePreview::new(format!("line {source_row}"), false))
        });
        assert_eq!(*loaded.borrow(), vec![10, 11, 12, 9, 13, 14]);
    }

    #[test]
    fn line_preview_is_bounded_and_visibly_marked() {
        let cache = VisibleLineStore::<usize>::default();
        cache.max_line_source_bytes.set(4);

        let line = cache
            .line(0, |limit| {
                assert_eq!(limit, 4);
                Some(LinePreview::new("abcd", true))
            })
            .expect("the preview should load");

        assert_eq!(line.source().as_ref(), "abcd…");
        assert!(line.source().ends_with('…'));
    }

    #[test]
    fn incidental_reads_do_not_displace_the_prepared_window() {
        let cache = VisibleLineStore::<usize>::default();
        cache.set_overscan(0);
        cache.max_line_source_bytes.set(4);
        cache.max_cache_retained_bytes.set(8);
        let loaded = RefCell::new(Vec::new());

        cache.prepare_visible_rows(10..12, 100, Some, |source_row, _| {
            loaded.borrow_mut().push(*source_row);
            Some(LinePreview::new("line", false))
        });
        assert_eq!(*loaded.borrow(), vec![10, 11]);

        cache
            .line(0, |_| Some(LinePreview::new("away", false)))
            .expect("an incidental read should still return a preview");
        assert_eq!(
            cache.lines.borrow().keys().copied().collect::<Vec<_>>(),
            [10, 11]
        );

        for key in [10, 11] {
            cache
                .line(key, |_| panic!("prepared rows must remain cached"))
                .expect("the prepared row should remain available");
        }
    }

    #[test]
    fn cache_budget_prioritizes_visible_rows_over_overscan() {
        let cache = VisibleLineStore::<usize>::default();
        cache.set_overscan(1);
        cache.max_line_source_bytes.set(4);
        cache.max_cache_retained_bytes.set(4);
        let loaded = RefCell::new(Vec::new());

        cache.prepare_visible_rows(1..2, 3, Some, |source_row, limit| {
            loaded.borrow_mut().push((*source_row, limit));
            Some(LinePreview::new("line", false))
        });

        assert_eq!(*loaded.borrow(), vec![(1, 4)]);
        assert_eq!(cache.lines.borrow().len(), 1);
        assert!(cache.lines.borrow().contains_key(&1));
        assert!(
            cache
                .lines
                .borrow()
                .values()
                .map(|line| line.retained_bytes)
                .sum::<usize>()
                <= 4
        );
    }
}
