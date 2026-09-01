//! Compressed, ordered source-row result sets and positional projections.

use std::{io::Cursor, sync::Arc};

use roaring::RoaringTreemap;

/// Ordered source rows backed by a compressed roaring treemap.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CompressedRows {
    pub(crate) rows: Arc<RoaringTreemap>,
}

impl CompressedRows {
    pub fn from_inclusive_ranges(ranges: impl IntoIterator<Item = (usize, usize)>) -> Self {
        let mut rows = RoaringTreemap::new();
        for (start, end) in ranges {
            let (Ok(start), Ok(end)) = (u64::try_from(start), u64::try_from(end)) else {
                continue;
            };
            if start <= end {
                rows.insert_range(start..=end);
            }
        }
        Self {
            rows: Arc::new(rows),
        }
    }

    pub fn len(&self) -> usize {
        usize::try_from(self.rows.len()).unwrap_or(usize::MAX)
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn get(&self, index: usize) -> Option<usize> {
        self.rows
            .select(u64::try_from(index).ok()?)
            .and_then(|row| usize::try_from(row).ok())
    }

    pub fn first(&self) -> Option<usize> {
        self.get(0)
    }

    pub fn contains(&self, row: usize) -> bool {
        u64::try_from(row)
            .ok()
            .is_some_and(|row| self.rows.contains(row))
    }

    /// Whether every row in `other` is already present.
    pub fn contains_all(&self, other: &Self) -> bool {
        other.rows.is_subset(&self.rows)
    }

    pub fn position(&self, row: usize) -> Option<usize> {
        let row = u64::try_from(row).ok()?;
        self.rows
            .contains(row)
            .then(|| self.rows.rank(row).saturating_sub(1))
            .and_then(|rank| usize::try_from(rank).ok())
    }

    /// Return the positional index whose source row is closest to `row`.
    ///
    /// Ties prefer the preceding source row so a disappearing anchor does not
    /// unexpectedly advance past an equally close result.
    pub fn nearest_position(&self, row: usize) -> Option<usize> {
        let row_u64 = u64::try_from(row).ok()?;
        let after_position = usize::try_from(self.rows.rank(row_u64)).ok()?;
        let before_position = after_position.checked_sub(1);
        let after_position = (after_position < self.len()).then_some(after_position);

        match (before_position, after_position) {
            (Some(before), Some(after)) => {
                let before_row = self.get(before)?;
                let after_row = self.get(after)?;
                Some(if row.abs_diff(before_row) <= row.abs_diff(after_row) {
                    before
                } else {
                    after
                })
            }
            (Some(before), None) => Some(before),
            (None, Some(after)) => Some(after),
            (None, None) => None,
        }
    }

    /// Keep rows selected by inclusive ranges in this set's positional space.
    ///
    /// The mask is built from source-row intervals and intersected with the
    /// compressed set, so sparse projections remain sparse without expanding
    /// every selected row into an intermediate collection.
    pub fn rows_at_position_ranges(
        &self,
        ranges: impl IntoIterator<Item = (usize, usize)>,
    ) -> Self {
        let len = self.len();
        if len == 0 {
            return Self::default();
        }
        let mut mask = RoaringTreemap::new();
        for (first, last) in ranges {
            let start = first.min(last);
            if start >= len {
                continue;
            }
            let end = first.max(last).min(len - 1);
            if start == 0 && end == len - 1 {
                return self.clone();
            }
            let (Some(source_start), Some(source_end)) = (self.get(start), self.get(end)) else {
                continue;
            };
            let (Ok(source_start), Ok(source_end)) =
                (u64::try_from(source_start), u64::try_from(source_end))
            else {
                continue;
            };
            mask.insert_range(source_start..=source_end);
        }
        mask &= self.rows.as_ref();
        Self {
            rows: Arc::new(mask),
        }
    }

    /// Map an exact subset of these rows back to compact inclusive positional ranges.
    ///
    /// The smaller side of the selection is enumerated: dense selections walk the
    /// excluded rows and build their complement, while sparse selections walk only
    /// selected rows.
    pub fn position_ranges_for_subset(&self, selected: &Self) -> Vec<(usize, usize)> {
        let row_count = self.len();
        if row_count == 0 {
            return Vec::new();
        }
        let selected_count = usize::try_from(self.rows.intersection_len(&selected.rows))
            .unwrap_or(usize::MAX)
            .min(row_count);
        if selected_count == 0 {
            return Vec::new();
        }
        if selected_count == row_count {
            return vec![(0, row_count - 1)];
        }

        if selected_count <= row_count - selected_count {
            let selected_rows = self.rows.as_ref() & selected.rows.as_ref();
            let positions = selected_rows
                .iter()
                .filter_map(|row| usize::try_from(row).ok().and_then(|row| self.position(row)));
            return consecutive_ranges(positions);
        }

        let excluded_rows = self.rows.as_ref() - selected.rows.as_ref();
        let mut ranges = Vec::new();
        let mut next_selected = 0;
        for excluded_position in excluded_rows
            .iter()
            .filter_map(|row| usize::try_from(row).ok().and_then(|row| self.position(row)))
        {
            if next_selected < excluded_position {
                ranges.push((next_selected, excluded_position - 1));
            }
            next_selected = excluded_position.saturating_add(1);
        }
        if next_selected < row_count {
            ranges.push((next_selected, row_count - 1));
        }
        ranges
    }

    pub fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        self.rows.iter().filter_map(|row| usize::try_from(row).ok())
    }

    /// Serialize the compressed set without expanding individual source rows.
    pub fn to_portable_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.rows.serialized_size());
        self.rows
            .serialize_into(&mut bytes)
            .expect("serializing a roaring treemap into Vec cannot fail");
        bytes
    }

    /// Restore a set serialized by [`Self::to_portable_bytes`].
    pub fn from_portable_bytes(bytes: &[u8]) -> Option<Self> {
        let mut reader = Cursor::new(bytes);
        let rows = RoaringTreemap::deserialize_from(&mut reader).ok()?;
        if reader.position() != u64::try_from(bytes.len()).ok()?
            || rows.max().is_some_and(|row| usize::try_from(row).is_err())
        {
            return None;
        }
        Some(Self {
            rows: Arc::new(rows),
        })
    }

    /// Insert one source row while preserving cheap clone semantics.
    pub fn insert(&mut self, row: usize) -> bool {
        let Ok(row) = u64::try_from(row) else {
            return false;
        };
        if self.rows.contains(row) {
            return false;
        }
        Arc::make_mut(&mut self.rows).insert(row)
    }

    /// Remove one source row while preserving cheap clone semantics.
    pub fn remove(&mut self, row: usize) -> bool {
        let Ok(row) = u64::try_from(row) else {
            return false;
        };
        if !self.rows.contains(row) {
            return false;
        }
        Arc::make_mut(&mut self.rows).remove(row)
    }

    /// Add source rows with one copy-on-write detach at most.
    pub fn extend(&mut self, rows: impl IntoIterator<Item = usize>) {
        let mut rows = rows.into_iter().filter_map(|row| u64::try_from(row).ok());
        while let Some(row) = rows.next() {
            if self.rows.contains(row) {
                continue;
            }
            let writable = Arc::make_mut(&mut self.rows);
            writable.insert(row);
            writable.extend(rows);
            return;
        }
    }

    /// Drop source rows outside a document snapshot.
    pub fn retain_below(&mut self, upper_bound: usize) {
        let Ok(upper_bound) = u64::try_from(upper_bound) else {
            return;
        };
        if self.rows.max().is_none_or(|row| row < upper_bound) {
            return;
        }
        Arc::make_mut(&mut self.rows).remove_range(upper_bound..);
    }

    /// Union another compressed row set with at most one copy-on-write detach.
    pub fn insert_rows(&mut self, rows: &Self) {
        if self.contains_all(rows) {
            return;
        }
        *Arc::make_mut(&mut self.rows) |= rows.rows.as_ref();
    }

    /// Subtract another compressed row set with at most one copy-on-write detach.
    pub fn remove_rows(&mut self, rows: &Self) {
        if self.rows.intersection_len(&rows.rows) == 0 {
            return;
        }
        *Arc::make_mut(&mut self.rows) -= rows.rows.as_ref();
    }

    pub fn union(&self, rows: impl IntoIterator<Item = usize>) -> Self {
        let mut rows = rows.into_iter().filter_map(|row| u64::try_from(row).ok());
        while let Some(row) = rows.next() {
            if self.rows.contains(row) {
                continue;
            }
            let mut merged = self.rows.as_ref().clone();
            merged.insert(row);
            merged.extend(rows);
            return Self {
                rows: Arc::new(merged),
            };
        }
        self.clone()
    }
}

impl FromIterator<usize> for CompressedRows {
    fn from_iter<T: IntoIterator<Item = usize>>(iter: T) -> Self {
        let mut rows = RoaringTreemap::new();
        for row in iter {
            if let Ok(row) = u64::try_from(row) {
                rows.insert(row);
            }
        }
        Self {
            rows: Arc::new(rows),
        }
    }
}

fn consecutive_ranges(indices: impl IntoIterator<Item = usize>) -> Vec<(usize, usize)> {
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for index in indices {
        match ranges.last_mut() {
            Some((_, end)) if end.saturating_add(1) == index => *end = index,
            _ => ranges.push((index, index)),
        }
    }
    ranges
}
