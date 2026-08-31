use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet},
    ops::Range,
};

use vclogg_core::CompressedRows;

use crate::selectable_log_text::LogText;

/// Maps virtual-list coordinates to source-file rows without owning presentation state.
#[derive(Clone)]
pub(crate) enum LogRowProjection {
    All,
    SourceRows(CompressedRows),
}

/// Keeps decoded text for the active virtual-list window.
///
/// Highlighting, selection, markers, typography, and wrapping are deliberately excluded so a
/// presentation-only change never causes the source file to be read again. Direct consumers such
/// as copy commands may load another row on demand; the next virtual-window update prunes it.
pub(crate) struct VirtualLogLineCache<K> {
    lines: RefCell<BTreeMap<K, LogText>>,
    window: Cell<Option<(usize, usize)>>,
    overscan: Cell<usize>,
}

impl<K> Default for VirtualLogLineCache<K> {
    fn default() -> Self {
        Self {
            lines: RefCell::default(),
            window: Cell::default(),
            overscan: Cell::new(12),
        }
    }
}

impl<K: Clone + Ord> VirtualLogLineCache<K> {
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

    pub(crate) fn line(&self, key: K, load: impl FnOnce() -> Option<String>) -> Option<LogText> {
        if let Some(line) = self.lines.borrow().get(&key).cloned() {
            return Some(line);
        }
        let line = LogText::new(load()?.into());
        self.lines.borrow_mut().insert(key, line.clone());
        Some(line)
    }

    pub(crate) fn prepare_visible_rows(
        &self,
        visible_range: Range<usize>,
        row_count: usize,
        mut key_for_row: impl FnMut(usize) -> Option<K>,
        mut load: impl FnMut(&K) -> Option<String>,
    ) {
        let start = visible_range.start.saturating_sub(self.overscan.get());
        let end = visible_range
            .end
            .saturating_add(self.overscan.get())
            .min(row_count);
        if self.window.replace(Some((start, end))) == Some((start, end)) {
            return;
        }

        let desired = (start..end)
            .filter_map(&mut key_for_row)
            .collect::<BTreeSet<_>>();
        self.lines
            .borrow_mut()
            .retain(|key, _| desired.contains(key));
        let missing = desired
            .into_iter()
            .filter(|key| !self.lines.borrow().contains_key(key))
            .collect::<Vec<_>>();
        for key in missing {
            if let Some(line) = load(&key) {
                self.lines
                    .borrow_mut()
                    .insert(key, LogText::new(line.into()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::VirtualLogLineCache;

    #[test]
    fn reads_only_the_active_virtual_window() {
        let cache = VirtualLogLineCache::<usize>::default();
        cache.set_overscan(1);
        let loaded = RefCell::new(Vec::new());

        cache.prepare_visible_rows(10..13, 100, Some, |source_row| {
            loaded.borrow_mut().push(*source_row);
            Some(format!("line {source_row}"))
        });
        assert_eq!(*loaded.borrow(), vec![9, 10, 11, 12, 13]);

        cache.prepare_visible_rows(10..13, 100, Some, |_| {
            panic!("an unchanged virtual window must not be read twice")
        });
        cache.prepare_visible_rows(12..14, 100, Some, |source_row| {
            loaded.borrow_mut().push(*source_row);
            Some(format!("line {source_row}"))
        });
        assert_eq!(*loaded.borrow(), vec![9, 10, 11, 12, 13, 14]);
    }
}
