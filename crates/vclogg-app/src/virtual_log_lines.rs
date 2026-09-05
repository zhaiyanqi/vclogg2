use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet},
    ops::Range,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
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
const MIN_TRUNCATED_PREVIEW_RETAINED_BYTES: usize = '…'.len_utf8();
/// A source byte can expand to at most one full eight-column tab stop in [`LogText`].
pub(crate) const MAX_VISIBLE_LINE_COLUMNS: usize = DEFAULT_MAX_LINE_SOURCE_BYTES * 8;

#[derive(Clone)]
struct CachedLogLine {
    text: LogText,
    retained_bytes: usize,
    source_unavailable: bool,
}

pub(crate) struct VisibleLineLoadRequest<K> {
    revision: u64,
    keys: Vec<K>,
    source_limits: Vec<usize>,
    retained_limits: Vec<usize>,
    cancellation: Arc<AtomicBool>,
}

impl<K> VisibleLineLoadRequest<K> {
    pub(crate) fn keys(&self) -> &[K] {
        &self.keys
    }

    pub(crate) fn load(
        self,
        mut load: impl FnMut(&K, usize) -> Option<LinePreview>,
    ) -> VisibleLineLoadResult<K> {
        let mut lines = Vec::with_capacity(self.keys.len());
        for ((key, source_limit), retained_limit) in self
            .keys
            .into_iter()
            .zip(self.source_limits)
            .zip(self.retained_limits)
        {
            if self.cancellation.load(Ordering::Acquire) {
                break;
            }
            let preview = load(&key, source_limit);
            lines.push((key, preview, retained_limit));
        }
        VisibleLineLoadResult {
            revision: self.revision,
            lines,
        }
    }
}

pub(crate) struct VisibleLineLoadResult<K> {
    revision: u64,
    lines: Vec<(K, Option<LinePreview>, usize)>,
}

/// A bounded, decoded visible-window snapshot retained by a presentation session.
///
/// Search-scope switches restore this snapshot without reading source files. The snapshot owns
/// only the currently prepared rows and their overscan, never the complete result set.
#[derive(Clone)]
pub(crate) struct VisibleLineSnapshot<K> {
    window: Option<(usize, usize, usize, usize)>,
    priority_keys: Vec<K>,
    lines: BTreeMap<K, CachedLogLine>,
}

/// A visible window loaded without replacing the window that is currently painted.
///
/// Result-row navigation uses this staging path so the old log window remains intact while the
/// destination is read on a background thread. Installing the staged window and moving the
/// viewport can then happen in one foreground update.
pub(crate) struct StagedVisibleLineLoadRequest<K> {
    window: (usize, usize, usize, usize),
    priority_keys: Vec<K>,
    keys: Vec<K>,
    source_limits: Vec<usize>,
    retained_limits: Vec<usize>,
}

impl<K> StagedVisibleLineLoadRequest<K> {
    pub(crate) fn keys(&self) -> &[K] {
        &self.keys
    }

    #[cfg(test)]
    pub(crate) fn requires_source_load(&self) -> bool {
        !self.keys.is_empty()
    }

    pub(crate) fn load(
        self,
        mut load: impl FnMut(&K, usize) -> Option<LinePreview>,
    ) -> StagedVisibleLineLoadResult<K> {
        self.load_with_cancellation(None, &mut load)
    }

    pub(crate) fn load_cancellable(
        self,
        cancellation: &AtomicBool,
        mut load: impl FnMut(&K, usize) -> Option<LinePreview>,
    ) -> StagedVisibleLineLoadResult<K> {
        self.load_with_cancellation(Some(cancellation), &mut load)
    }

    fn load_with_cancellation(
        self,
        cancellation: Option<&AtomicBool>,
        load: &mut impl FnMut(&K, usize) -> Option<LinePreview>,
    ) -> StagedVisibleLineLoadResult<K> {
        let lines = self
            .keys
            .into_iter()
            .zip(self.source_limits)
            .zip(self.retained_limits)
            .take_while(|_| {
                cancellation.is_none_or(|cancellation| !cancellation.load(Ordering::Acquire))
            })
            .map(|((key, source_limit), retained_limit)| {
                let preview = load(&key, source_limit);
                (key, preview, retained_limit)
            })
            .collect();
        StagedVisibleLineLoadResult {
            window: self.window,
            priority_keys: self.priority_keys,
            lines,
        }
    }
}

#[derive(Clone)]
pub(crate) struct StagedVisibleLineLoadResult<K> {
    window: (usize, usize, usize, usize),
    priority_keys: Vec<K>,
    lines: Vec<(K, Option<LinePreview>, usize)>,
}

impl<K> StagedVisibleLineLoadResult<K> {
    pub(crate) fn has_unavailable_lines(&self) -> bool {
        self.lines.iter().any(|(_, preview, _)| preview.is_none())
    }
}

impl CachedLogLine {
    fn from_preview(preview: Option<LinePreview>, retained_limit: usize) -> Option<Self> {
        let (text, truncated, source_unavailable) = match preview {
            Some(preview) => {
                let (text, truncated) = preview.into_parts();
                (text, truncated, false)
            }
            None => (
                crate::tr!(
                    "源文件已改变，无法读取该快照行",
                    "The source changed; this snapshot line is unavailable"
                )
                .to_string(),
                false,
                true,
            ),
        };
        let text = LogText::preview_with_retained_limit(text, truncated, retained_limit)?;
        Some(Self {
            retained_bytes: text.retained_bytes(),
            text,
            source_unavailable,
        })
    }
}

/// Keeps byte-bounded decoded previews for the active virtual-list window.
///
/// Highlighting, selection, markers, typography, and wrapping are deliberately excluded so a
/// presentation-only change never causes the source file to be read again. Direct consumers such
/// as copy commands read the authoritative document directly. The UI thread only publishes a
/// revisioned visible-window request; background tasks load visible rows before overscan rows and
/// install them if that revision is still current. Retained preview payloads never exceed the
/// cache byte budget, and incidental queries never trigger source I/O.
pub(crate) struct VisibleLineStore<K> {
    lines: RefCell<BTreeMap<K, CachedLogLine>>,
    prepared_keys: RefCell<BTreeSet<K>>,
    prepared_priority: RefCell<Vec<K>>,
    window: Cell<Option<(usize, usize, usize, usize)>>,
    overscan: Cell<usize>,
    max_line_source_bytes: Cell<usize>,
    max_cache_retained_bytes: Cell<usize>,
    revision: Cell<u64>,
    load_cancellation: RefCell<Option<Arc<AtomicBool>>>,
}

impl<K> Default for VisibleLineStore<K> {
    fn default() -> Self {
        Self {
            lines: RefCell::default(),
            prepared_keys: RefCell::default(),
            prepared_priority: RefCell::default(),
            window: Cell::default(),
            overscan: Cell::new(0),
            max_line_source_bytes: Cell::new(DEFAULT_MAX_LINE_SOURCE_BYTES),
            max_cache_retained_bytes: Cell::new(DEFAULT_MAX_CACHE_RETAINED_BYTES),
            revision: Cell::new(1),
            load_cancellation: RefCell::default(),
        }
    }
}

impl<K> Drop for VisibleLineStore<K> {
    fn drop(&mut self) {
        if let Some(cancellation) = self.load_cancellation.get_mut().take() {
            cancellation.store(true, Ordering::Release);
        }
    }
}

impl<K: Clone + Ord> VisibleLineStore<K> {
    pub(crate) fn revision(&self) -> u64 {
        self.revision.get()
    }

    pub(crate) fn snapshot(&self) -> VisibleLineSnapshot<K> {
        VisibleLineSnapshot {
            window: self.window.get(),
            priority_keys: self.prepared_priority.borrow().clone(),
            lines: self.lines.borrow().clone(),
        }
    }

    pub(crate) fn install_snapshot(&self, snapshot: VisibleLineSnapshot<K>) {
        if let Some(cancellation) = self.load_cancellation.borrow_mut().take() {
            cancellation.store(true, Ordering::Release);
        }
        self.revision.set(self.revision.get().saturating_add(1));
        self.window.set(snapshot.window);
        *self.prepared_keys.borrow_mut() = snapshot.priority_keys.iter().cloned().collect();
        *self.prepared_priority.borrow_mut() = snapshot.priority_keys;
        *self.lines.borrow_mut() = snapshot.lines;
    }

    fn visible_window(
        &self,
        visible_range: Range<usize>,
        row_count: usize,
        mut key_for_row: impl FnMut(usize) -> Option<K>,
    ) -> ((usize, usize, usize, usize), Vec<K>) {
        let visible_start = visible_range.start.min(row_count);
        let visible_end = visible_range.end.min(row_count).max(visible_start);
        let start = visible_start.saturating_sub(self.overscan.get());
        let end = visible_end
            .saturating_add(self.overscan.get())
            .min(row_count);
        let window = (visible_start, visible_end, start, end);

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
        (window, priority_keys)
    }

    fn visible_range_is_ready(
        &self,
        visible_range: Range<usize>,
        row_count: usize,
        key_for_row: &mut impl FnMut(usize) -> Option<K>,
    ) -> bool {
        let visible_start = visible_range.start.min(row_count);
        let visible_end = visible_range.end.min(row_count).max(visible_start);
        let Some((_, _, prepared_start, prepared_end)) = self.window.get() else {
            return false;
        };
        if visible_start < prepared_start || visible_end > prepared_end {
            return false;
        }

        let prepared_keys = self.prepared_keys.borrow();
        let lines = self.lines.borrow();
        (visible_start..visible_end).all(|row_ix| {
            key_for_row(row_ix)
                .is_none_or(|key| prepared_keys.contains(&key) && lines.contains_key(&key))
        })
    }

    #[cfg(test)]
    pub(crate) fn set_overscan(&self, overscan: usize) {
        if self.overscan.replace(overscan) != overscan {
            self.invalidate_window();
        }
    }

    pub(crate) fn invalidate_window(&self) {
        if let Some(cancellation) = self.load_cancellation.borrow_mut().take() {
            cancellation.store(true, Ordering::Release);
        }
        self.revision.set(self.revision.get().saturating_add(1));
        self.window.set(None);
        self.prepared_keys.borrow_mut().clear();
        self.prepared_priority.borrow_mut().clear();
    }

    pub(crate) fn clear(&self) {
        self.lines.borrow_mut().clear();
        self.invalidate_window();
    }

    pub(crate) fn retain(&self, mut keep: impl FnMut(&K) -> bool) {
        self.lines.borrow_mut().retain(|key, _| keep(key));
        self.invalidate_window();
    }

    pub(crate) fn line(&self, key: K) -> Option<LogText> {
        if !self.prepared_keys.borrow().contains(&key) {
            return None;
        }
        self.lines.borrow().get(&key).map(|line| line.text.clone())
    }

    pub(crate) fn source_unavailable(&self, key: K) -> bool {
        self.prepared_keys.borrow().contains(&key)
            && self
                .lines
                .borrow()
                .get(&key)
                .is_some_and(|line| line.source_unavailable)
    }

    #[cfg(test)]
    pub(crate) fn cached_keys(&self) -> Vec<K> {
        self.lines.borrow().keys().cloned().collect()
    }

    pub(crate) fn request_visible_rows(
        &self,
        visible_range: Range<usize>,
        row_count: usize,
        mut key_for_row: impl FnMut(usize) -> Option<K>,
    ) -> Option<VisibleLineLoadRequest<K>> {
        if self.visible_range_is_ready(visible_range.clone(), row_count, &mut key_for_row) {
            return None;
        }
        let (window, priority_keys) = self.visible_window(visible_range, row_count, key_for_row);
        if self.window.get() == Some(window) && *self.prepared_priority.borrow() == priority_keys {
            return None;
        }
        if let Some(cancellation) = self.load_cancellation.borrow_mut().take() {
            cancellation.store(true, Ordering::Release);
        }
        let revision = self.revision.get().saturating_add(1);
        self.revision.set(revision);
        self.window.set(Some(window));
        *self.prepared_priority.borrow_mut() = priority_keys.clone();
        *self.prepared_keys.borrow_mut() = priority_keys.iter().cloned().collect();

        let byte_budget = self.max_cache_retained_bytes.get();
        let mut previous = std::mem::take(&mut *self.lines.borrow_mut());
        let mut next = BTreeMap::new();
        let mut reserved_bytes = 0usize;
        let mut keys = Vec::new();
        let mut source_limits = Vec::new();
        let mut retained_limits = Vec::new();
        for (key_ix, key) in priority_keys.iter().enumerate() {
            if reserved_bytes >= byte_budget {
                break;
            }
            let remaining = byte_budget - reserved_bytes;
            let remaining_keys = priority_keys.len().saturating_sub(key_ix).max(1);
            let fair_retained_limit = remaining / remaining_keys;
            let retained_limit = if fair_retained_limit >= MIN_TRUNCATED_PREVIEW_RETAINED_BYTES {
                fair_retained_limit
            } else {
                remaining
            };
            if retained_limit < MIN_TRUNCATED_PREVIEW_RETAINED_BYTES {
                break;
            }
            if let Some(line) = previous.remove(key)
                && line.retained_bytes <= retained_limit
            {
                reserved_bytes = reserved_bytes.saturating_add(line.retained_bytes);
                next.insert(key.clone(), line);
                continue;
            }
            let source_limit = self.max_line_source_bytes.get().min(retained_limit);
            keys.push(key.clone());
            source_limits.push(source_limit);
            retained_limits.push(retained_limit);
            reserved_bytes = reserved_bytes.saturating_add(retained_limit);
        }
        *self.lines.borrow_mut() = next;
        if keys.is_empty() {
            return None;
        }
        let cancellation = Arc::new(AtomicBool::new(false));
        *self.load_cancellation.borrow_mut() = Some(cancellation.clone());
        Some(VisibleLineLoadRequest {
            revision,
            keys,
            source_limits,
            retained_limits,
            cancellation,
        })
    }

    pub(crate) fn stage_visible_rows(
        &self,
        visible_range: Range<usize>,
        row_count: usize,
        mut key_for_row: impl FnMut(usize) -> Option<K>,
    ) -> Option<StagedVisibleLineLoadRequest<K>> {
        if self.visible_range_is_ready(visible_range.clone(), row_count, &mut key_for_row) {
            return None;
        }
        let (window, priority_keys) = self.visible_window(visible_range, row_count, key_for_row);
        let current_window_is_ready = self.window.get() == Some(window)
            && *self.prepared_priority.borrow() == priority_keys
            && priority_keys
                .iter()
                .all(|key| self.lines.borrow().contains_key(key));
        if current_window_is_ready {
            return None;
        }

        let byte_budget = self.max_cache_retained_bytes.get();
        let mut reserved_bytes = 0usize;
        let mut keys = Vec::new();
        let mut source_limits = Vec::new();
        let mut retained_limits = Vec::new();
        let lines = self.lines.borrow();
        for (key_ix, key) in priority_keys.iter().enumerate() {
            if reserved_bytes >= byte_budget {
                break;
            }
            let remaining = byte_budget - reserved_bytes;
            let remaining_keys = priority_keys.len().saturating_sub(key_ix).max(1);
            let fair_retained_limit = remaining / remaining_keys;
            let retained_limit = if fair_retained_limit >= MIN_TRUNCATED_PREVIEW_RETAINED_BYTES {
                fair_retained_limit
            } else {
                remaining
            };
            if retained_limit < MIN_TRUNCATED_PREVIEW_RETAINED_BYTES {
                break;
            }
            if let Some(line) = lines.get(key)
                && line.retained_bytes <= retained_limit
            {
                reserved_bytes = reserved_bytes.saturating_add(line.retained_bytes);
                continue;
            }
            keys.push(key.clone());
            source_limits.push(self.max_line_source_bytes.get().min(retained_limit));
            retained_limits.push(retained_limit);
            reserved_bytes = reserved_bytes.saturating_add(retained_limit);
        }
        Some(StagedVisibleLineLoadRequest {
            window,
            priority_keys,
            keys,
            source_limits,
            retained_limits,
        })
    }

    /// Stages a complete visible window for a projection or document that has not been
    /// installed yet. Unlike ordinary staging, this always returns a frame, even when every
    /// target key is already cached, so the caller can invalidate the old projection and install
    /// the target window in one foreground update.
    pub(crate) fn stage_replacement_visible_rows(
        &self,
        visible_range: Range<usize>,
        row_count: usize,
        key_for_row: impl FnMut(usize) -> Option<K>,
        mut can_reuse: impl FnMut(&K) -> bool,
    ) -> StagedVisibleLineLoadRequest<K> {
        let (window, priority_keys) = self.visible_window(visible_range, row_count, key_for_row);
        let byte_budget = self.max_cache_retained_bytes.get();
        let mut reserved_bytes = 0usize;
        let mut keys = Vec::new();
        let mut source_limits = Vec::new();
        let mut retained_limits = Vec::new();
        let lines = self.lines.borrow();
        for (key_ix, key) in priority_keys.iter().enumerate() {
            if reserved_bytes >= byte_budget {
                break;
            }
            let remaining = byte_budget - reserved_bytes;
            let remaining_keys = priority_keys.len().saturating_sub(key_ix).max(1);
            let fair_retained_limit = remaining / remaining_keys;
            let retained_limit = if fair_retained_limit >= MIN_TRUNCATED_PREVIEW_RETAINED_BYTES {
                fair_retained_limit
            } else {
                remaining
            };
            if retained_limit < MIN_TRUNCATED_PREVIEW_RETAINED_BYTES {
                break;
            }
            if can_reuse(key)
                && let Some(line) = lines.get(key)
                && line.retained_bytes <= retained_limit
            {
                reserved_bytes = reserved_bytes.saturating_add(line.retained_bytes);
                continue;
            }
            keys.push(key.clone());
            source_limits.push(self.max_line_source_bytes.get().min(retained_limit));
            retained_limits.push(retained_limit);
            reserved_bytes = reserved_bytes.saturating_add(retained_limit);
        }
        StagedVisibleLineLoadRequest {
            window,
            priority_keys,
            keys,
            source_limits,
            retained_limits,
        }
    }

    pub(crate) fn install_staged(&self, staged: StagedVisibleLineLoadResult<K>) {
        if let Some(cancellation) = self.load_cancellation.borrow_mut().take() {
            cancellation.store(true, Ordering::Release);
        }
        self.revision.set(self.revision.get().saturating_add(1));
        self.window.set(Some(staged.window));
        *self.prepared_keys.borrow_mut() = staged.priority_keys.iter().cloned().collect();
        *self.prepared_priority.borrow_mut() = staged.priority_keys;

        let mut loaded = staged
            .lines
            .into_iter()
            .filter_map(|(key, preview, retained_limit)| {
                Some((key, CachedLogLine::from_preview(preview, retained_limit)?))
            })
            .collect::<BTreeMap<_, _>>();
        let mut previous = std::mem::take(&mut *self.lines.borrow_mut());
        let mut next = BTreeMap::new();
        let byte_budget = self.max_cache_retained_bytes.get();
        let mut retained_bytes = 0usize;
        for key in self.prepared_priority.borrow().iter() {
            if retained_bytes >= byte_budget {
                break;
            }
            let remaining = byte_budget - retained_bytes;
            let line = loaded.remove(key).or_else(|| previous.remove(key));
            let Some(line) = line else {
                continue;
            };
            if line.retained_bytes <= remaining {
                retained_bytes = retained_bytes.saturating_add(line.retained_bytes);
                next.insert(key.clone(), line);
            }
        }
        *self.lines.borrow_mut() = next;
    }

    pub(crate) fn install_loaded(&self, loaded: VisibleLineLoadResult<K>) -> bool {
        if loaded.revision != self.revision.get() {
            return false;
        }
        self.load_cancellation.borrow_mut().take();
        let mut loaded = loaded
            .lines
            .into_iter()
            .filter_map(|(key, preview, retained_limit)| {
                Some((key, CachedLogLine::from_preview(preview, retained_limit)?))
            })
            .collect::<BTreeMap<_, _>>();
        let mut previous = std::mem::take(&mut *self.lines.borrow_mut());
        let mut next = BTreeMap::new();
        let byte_budget = self.max_cache_retained_bytes.get();
        let mut retained_bytes = 0usize;
        for key in self.prepared_priority.borrow().iter() {
            if retained_bytes >= byte_budget {
                break;
            }
            let remaining = byte_budget - retained_bytes;
            let line = previous.remove(key).or_else(|| loaded.remove(key));
            let Some(line) = line else {
                continue;
            };
            if line.retained_bytes <= remaining {
                retained_bytes = retained_bytes.saturating_add(line.retained_bytes);
                next.insert(key.clone(), line);
            }
        }
        *self.lines.borrow_mut() = next;
        true
    }

    #[cfg(test)]
    pub(crate) fn prepare_visible_rows(
        &self,
        visible_range: Range<usize>,
        row_count: usize,
        key_for_row: impl FnMut(usize) -> Option<K>,
        load: impl FnMut(&K, usize) -> Option<LinePreview>,
    ) {
        if let Some(request) = self.request_visible_rows(visible_range, row_count, key_for_row) {
            self.install_loaded(request.load(load));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        cell::RefCell,
        sync::atomic::{AtomicBool, Ordering},
    };

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
            panic!("rows already covered by prepared overscan must not be read: {source_row}")
        });
        assert_eq!(*loaded.borrow(), vec![10, 11, 12, 9, 13]);

        cache.prepare_visible_rows(13..15, 100, Some, |source_row, _| {
            loaded.borrow_mut().push(*source_row);
            Some(LinePreview::new(format!("line {source_row}"), false))
        });
        assert_eq!(*loaded.borrow(), vec![10, 11, 12, 9, 13, 14, 15]);
    }

    #[test]
    fn staging_a_window_without_source_rows_still_produces_an_atomic_frame() {
        let cache = VisibleLineStore::<usize>::default();
        let staged = cache
            .stage_visible_rows(0..2, 2, |_| None)
            .expect("a group-only window still needs a staged ownership handoff");
        assert!(!staged.requires_source_load());
        let staged = staged.load(|_, _| panic!("group rows do not read source lines"));

        cache.install_staged(staged);

        assert_eq!(cache.window.get(), Some((0, 2, 0, 2)));
        assert!(cache.prepared_keys.borrow().is_empty());
    }

    #[test]
    fn invalidated_projection_reprepares_the_same_viewport_coordinates() {
        let cache = VisibleLineStore::<usize>::default();
        cache.set_overscan(0);
        let loaded = RefCell::new(Vec::new());

        cache.prepare_visible_rows(0..2, 2, Some, |source_row, _| {
            loaded.borrow_mut().push(*source_row);
            Some(LinePreview::new(format!("line {source_row}"), false))
        });
        cache.invalidate_window();
        cache.prepare_visible_rows(
            0..2,
            2,
            |row_ix| Some(row_ix + 100),
            |source_row, _| {
                loaded.borrow_mut().push(*source_row);
                Some(LinePreview::new(format!("line {source_row}"), false))
            },
        );

        assert_eq!(*loaded.borrow(), [0, 1, 100, 101]);
        assert_eq!(
            cache.lines.borrow().keys().copied().collect::<Vec<_>>(),
            [100, 101]
        );
    }

    #[test]
    fn stable_row_order_change_reprepares_the_same_viewport_coordinates() {
        let cache = VisibleLineStore::<usize>::default();
        cache.set_overscan(1);
        cache.max_line_source_bytes.set(4);
        cache.max_cache_retained_bytes.set(4);
        let loaded = RefCell::new(Vec::new());

        cache.prepare_visible_rows(
            0..1,
            2,
            |row_ix| [10, 11].get(row_ix).copied(),
            |source_row, _| {
                loaded.borrow_mut().push(*source_row);
                Some(LinePreview::new("line", false))
            },
        );
        cache.prepare_visible_rows(
            0..1,
            2,
            |row_ix| [11, 10].get(row_ix).copied(),
            |source_row, _| {
                loaded.borrow_mut().push(*source_row);
                Some(LinePreview::new("line", false))
            },
        );

        assert_eq!(*loaded.borrow(), [10, 11]);
        assert_eq!(
            cache.lines.borrow().keys().copied().collect::<Vec<_>>(),
            [11]
        );
    }

    #[test]
    fn line_preview_is_bounded_and_visibly_marked() {
        let cache = VisibleLineStore::<usize>::default();
        cache.max_line_source_bytes.set(4);

        cache.prepare_visible_rows(0..1, 1, Some, |_, limit| {
            assert_eq!(limit, 4);
            Some(LinePreview::new("abcd", true))
        });

        let line = cache.line(0).expect("the preview should load");

        assert_eq!(line.source().as_ref(), "abcd…");
        assert!(line.source().ends_with('…'));
    }

    #[test]
    fn rows_outside_the_prepared_window_are_not_read() {
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

        assert!(cache.line(0).is_none());
        assert_eq!(
            cache.lines.borrow().keys().copied().collect::<Vec<_>>(),
            [10, 11]
        );

        for key in [10, 11] {
            cache
                .line(key)
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

    #[test]
    fn uncached_rows_inside_the_prepared_window_wait_for_async_installation() {
        let cache = VisibleLineStore::<usize>::default();
        cache.set_overscan(0);
        cache.max_line_source_bytes.set(4);
        cache.max_cache_retained_bytes.set(4);

        cache.prepare_visible_rows(0..2, 2, Some, |source_row, _| {
            Some(LinePreview::new(format!("row {source_row}"), false))
        });

        let line = cache.line(1);
        assert!(line.is_none());
        assert!(!cache.lines.borrow().contains_key(&1));
    }

    #[test]
    fn expanded_tabs_cannot_blank_later_visible_rows() {
        let cache = VisibleLineStore::<usize>::default();
        cache.set_overscan(0);
        cache.max_line_source_bytes.set(64);
        cache.max_cache_retained_bytes.set(192);

        cache.prepare_visible_rows(0..3, 3, Some, |_, source_limit| {
            Some(LinePreview::new("\t".repeat(source_limit), true))
        });

        assert_eq!(cache.lines.borrow().len(), 3);
        assert!((0..3).all(|row| cache.line(row).is_some()));
        assert!(
            cache
                .lines
                .borrow()
                .values()
                .map(|line| line.retained_bytes)
                .sum::<usize>()
                <= 192
        );
    }

    #[test]
    fn unavailable_source_is_distinct_from_a_pending_visible_line() {
        let cache = VisibleLineStore::<usize>::default();
        cache.set_overscan(0);
        let request = cache
            .request_visible_rows(0..1, 1, Some)
            .expect("可见行应产生加载请求");

        assert!(cache.line(0).is_none());
        assert!(!cache.source_unavailable(0));
        assert!(cache.install_loaded(request.load(|_, _| None)));
        assert!(cache.line(0).is_some());
        assert!(cache.source_unavailable(0));
    }

    #[test]
    fn stale_background_results_cannot_replace_a_newer_visible_window() {
        let cache = VisibleLineStore::<usize>::default();
        cache.set_overscan(0);
        let first = cache
            .request_visible_rows(0..1, 2, Some)
            .expect("首个窗口应请求正文");
        let first =
            first.load(|source_row, _| Some(LinePreview::new(format!("line {source_row}"), false)));
        let second = cache
            .request_visible_rows(1..2, 2, Some)
            .expect("滚动后的窗口应请求正文");

        assert!(!cache.install_loaded(first));
        assert!(cache.line(0).is_none());
        assert!(cache.install_loaded(second.load(|source_row, _| {
            Some(LinePreview::new(format!("line {source_row}"), false))
        })));
        assert_eq!(
            cache.line(1).expect("新窗口正文应被安装").source().as_ref(),
            "line 1"
        );
    }

    #[test]
    fn staged_window_keeps_current_lines_until_atomic_installation() {
        let cache = VisibleLineStore::<usize>::default();
        cache.set_overscan(0);
        cache.prepare_visible_rows(0..2, 20, Some, |source_row, _| {
            Some(LinePreview::new(format!("line {source_row}"), false))
        });

        let staged = cache
            .stage_visible_rows(10..12, 20, Some)
            .expect("远处窗口应在不清空当前窗口的情况下预加载")
            .load(|source_row, _| Some(LinePreview::new(format!("line {source_row}"), false)));

        assert_eq!(cache.line(0).unwrap().source().as_ref(), "line 0");
        assert_eq!(cache.line(1).unwrap().source().as_ref(), "line 1");
        assert!(cache.line(10).is_none());

        cache.install_staged(staged);

        assert!(cache.line(0).is_none());
        assert_eq!(cache.line(10).unwrap().source().as_ref(), "line 10");
        assert_eq!(cache.line(11).unwrap().source().as_ref(), "line 11");
    }

    #[test]
    fn replacement_frame_keeps_old_rows_and_can_force_reload_the_same_keys() {
        let cache = VisibleLineStore::<usize>::default();
        cache.set_overscan(0);
        cache.prepare_visible_rows(0..1, 1, Some, |_, _| {
            Some(LinePreview::new("old snapshot", false))
        });

        let staged = cache
            .stage_replacement_visible_rows(0..1, 1, Some, |_| false)
            .load(|_, _| Some(LinePreview::new("new snapshot", false)));

        assert_eq!(cache.line(0).unwrap().source().as_ref(), "old snapshot");
        cache.install_staged(staged);
        assert_eq!(cache.line(0).unwrap().source().as_ref(), "new snapshot");
    }

    #[test]
    fn staged_window_reuses_overlapping_cached_lines() {
        let cache = VisibleLineStore::<usize>::default();
        cache.set_overscan(0);
        cache.prepare_visible_rows(0..2, 20, Some, |source_row, _| {
            Some(LinePreview::new(format!("line {source_row}"), false))
        });
        let loaded = RefCell::new(Vec::new());

        let staged = cache
            .stage_visible_rows(1..3, 20, Some)
            .expect("the shifted window needs a staged ownership handoff")
            .load(|source_row, _| {
                loaded.borrow_mut().push(*source_row);
                Some(LinePreview::new(format!("line {source_row}"), false))
            });

        assert_eq!(*loaded.borrow(), vec![2]);
        cache.install_staged(staged);
        assert_eq!(cache.line(1).unwrap().source().as_ref(), "line 1");
        assert_eq!(cache.line(2).unwrap().source().as_ref(), "line 2");
    }

    #[test]
    fn cancelling_a_staged_window_stops_its_remaining_source_reads() {
        let cache = VisibleLineStore::<usize>::default();
        cache.set_overscan(0);
        let request = cache
            .stage_visible_rows(0..3, 3, Some)
            .expect("首屏预装应产生读取请求");
        let cancellation = AtomicBool::new(false);
        let loaded_rows = RefCell::new(Vec::new());

        let _staged = request.load_cancellable(&cancellation, |source_row, _| {
            loaded_rows.borrow_mut().push(*source_row);
            cancellation.store(true, Ordering::Release);
            Some(LinePreview::new(format!("line {source_row}"), false))
        });

        assert_eq!(*loaded_rows.borrow(), [0]);
    }

    #[test]
    fn invalidating_a_window_stops_its_remaining_source_reads() {
        let cache = VisibleLineStore::<usize>::default();
        cache.set_overscan(0);
        let request = cache
            .request_visible_rows(0..3, 3, Some)
            .expect("首个窗口应请求正文");
        let loaded_rows = RefCell::new(Vec::new());

        let stale = request.load(|source_row, _| {
            loaded_rows.borrow_mut().push(*source_row);
            cache.invalidate_window();
            Some(LinePreview::new(format!("line {source_row}"), false))
        });

        assert_eq!(*loaded_rows.borrow(), [0]);
        assert!(!cache.install_loaded(stale));
    }
}
