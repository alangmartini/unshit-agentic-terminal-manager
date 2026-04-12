//! Damage tracking for incremental rendering.
//!
//! Elements that opt into persistent buffer rendering track which regions
//! of their content changed between frames. The batch builder uses this
//! information to patch only the affected portion of the GPU instance buffer
//! rather than rebuilding it from scratch.

use crate::id::NodeId;
use rustc_hash::FxHashMap;

/// A rectangular region of cells that changed since the last frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DamageRegion {
    pub row_start: u32,
    pub row_end: u32,
    pub col_start: u32,
    pub col_end: u32,
}

impl DamageRegion {
    /// Create a new damage region covering the given cell rectangle.
    pub fn new(row_start: u32, row_end: u32, col_start: u32, col_end: u32) -> Self {
        Self { row_start, row_end, col_start, col_end }
    }

    /// Returns the number of rows this region spans.
    pub fn row_count(&self) -> u32 {
        self.row_end.saturating_sub(self.row_start)
    }

    /// Returns the number of columns this region spans.
    pub fn col_count(&self) -> u32 {
        self.col_end.saturating_sub(self.col_start)
    }

    /// Returns total number of cells in this damage region.
    pub fn cell_count(&self) -> u32 {
        self.row_count() * self.col_count()
    }

    /// Returns true if this region overlaps with `other`.
    pub fn overlaps(&self, other: &DamageRegion) -> bool {
        self.row_start < other.row_end
            && self.row_end > other.row_start
            && self.col_start < other.col_end
            && self.col_end > other.col_start
    }

    /// Merge another region into this one, expanding to cover both.
    pub fn merge(&mut self, other: &DamageRegion) {
        self.row_start = self.row_start.min(other.row_start);
        self.row_end = self.row_end.max(other.row_end);
        self.col_start = self.col_start.min(other.col_start);
        self.col_end = self.col_end.max(other.col_end);
    }

    /// Create a damage region that covers the full grid.
    pub fn full(rows: u32, cols: u32) -> Self {
        Self { row_start: 0, row_end: rows, col_start: 0, col_end: cols }
    }
}

/// Tracks which rows of an element have been dirtied since the last batch build.
/// Uses a simple bitvec-style tracking with a Vec<u64> as bitfield storage.
#[derive(Clone, Debug)]
pub struct DirtyRows {
    /// Each bit represents one row. Bit N is set if row N changed.
    bits: Vec<u64>,
    /// Total number of rows being tracked.
    row_count: u32,
}

impl DirtyRows {
    /// Create a new tracker for `row_count` rows, all initially clean.
    pub fn new(row_count: u32) -> Self {
        let word_count = ((row_count as usize) + 63) / 64;
        Self { bits: vec![0; word_count], row_count }
    }

    /// Mark a single row as dirty.
    pub fn mark_row(&mut self, row: u32) {
        if row < self.row_count {
            let word = row as usize / 64;
            let bit = row as usize % 64;
            self.bits[word] |= 1 << bit;
        }
    }

    /// Mark a range of rows as dirty (inclusive start, exclusive end).
    pub fn mark_range(&mut self, start: u32, end: u32) {
        let end = end.min(self.row_count);
        for row in start..end {
            self.mark_row(row);
        }
    }

    /// Mark all rows as dirty.
    pub fn mark_all(&mut self) {
        for word in &mut self.bits {
            *word = u64::MAX;
        }
    }

    /// Clear all dirty flags.
    pub fn clear(&mut self) {
        for word in &mut self.bits {
            *word = 0;
        }
    }

    /// Returns true if any row is dirty.
    pub fn any_dirty(&self) -> bool {
        self.bits.iter().any(|&w| w != 0)
    }

    /// Returns true if the given row is dirty.
    pub fn is_row_dirty(&self, row: u32) -> bool {
        if row >= self.row_count {
            return false;
        }
        let word = row as usize / 64;
        let bit = row as usize % 64;
        (self.bits[word] >> bit) & 1 != 0
    }

    /// Returns the total number of rows being tracked.
    pub fn row_count(&self) -> u32 {
        self.row_count
    }

    /// Resize the tracker. New rows are marked clean.
    pub fn resize(&mut self, new_row_count: u32) {
        let new_word_count = ((new_row_count as usize) + 63) / 64;
        self.bits.resize(new_word_count, 0);
        self.row_count = new_row_count;
    }

    /// Collect contiguous dirty row ranges as `(start, end)` pairs.
    pub fn dirty_ranges(&self) -> Vec<(u32, u32)> {
        let mut ranges = Vec::new();
        let mut in_range = false;
        let mut range_start = 0u32;

        for row in 0..self.row_count {
            if self.is_row_dirty(row) {
                if !in_range {
                    range_start = row;
                    in_range = true;
                }
            } else if in_range {
                ranges.push((range_start, row));
                in_range = false;
            }
        }

        if in_range {
            ranges.push((range_start, self.row_count));
        }

        ranges
    }

    /// Convert dirty rows into damage regions spanning all columns.
    pub fn to_damage_regions(&self, cols: u32) -> Vec<DamageRegion> {
        self.dirty_ranges()
            .into_iter()
            .map(|(start, end)| DamageRegion::new(start, end, 0, cols))
            .collect()
    }
}

/// Per-element damage state stored in a central tracker.
#[derive(Clone, Debug)]
pub struct ElementDamageState {
    pub dirty_rows: DirtyRows,
    pub pending_regions: Vec<DamageRegion>,
    /// Grid dimensions (rows, cols) for this element.
    pub rows: u32,
    pub cols: u32,
    /// Set to true if a structural change happened (resize, reorder).
    pub structural_change: bool,
}

impl ElementDamageState {
    pub fn new(rows: u32, cols: u32) -> Self {
        Self {
            dirty_rows: DirtyRows::new(rows),
            pending_regions: Vec::new(),
            rows,
            cols,
            structural_change: false,
        }
    }

    /// Record a damage region and update dirty rows accordingly.
    pub fn record_damage(&mut self, region: DamageRegion) {
        self.dirty_rows.mark_range(region.row_start, region.row_end);
        self.pending_regions.push(region);
    }

    /// Mark the entire element as damaged (full rebuild needed).
    pub fn mark_full_damage(&mut self) {
        self.dirty_rows.mark_all();
        self.pending_regions.clear();
        self.pending_regions.push(DamageRegion::full(self.rows, self.cols));
    }

    /// Consume and coalesce all pending damage into a minimal set of regions.
    /// Clears the pending state after returning.
    pub fn take_coalesced_damage(&mut self) -> Vec<DamageRegion> {
        if self.pending_regions.is_empty() {
            return Vec::new();
        }

        // Sort by row_start then col_start for merging
        self.pending_regions
            .sort_by(|a, b| a.row_start.cmp(&b.row_start).then(a.col_start.cmp(&b.col_start)));

        let mut coalesced: Vec<DamageRegion> = Vec::new();
        for region in self.pending_regions.drain(..) {
            if let Some(last) = coalesced.last_mut() {
                if last.overlaps(&region)
                    || (last.row_end >= region.row_start
                        && last.col_start == region.col_start
                        && last.col_end == region.col_end)
                {
                    last.merge(&region);
                    continue;
                }
            }
            coalesced.push(region);
        }

        self.dirty_rows.clear();
        coalesced
    }

    /// Handle a resize: updates dimensions, marks structural change.
    pub fn resize(&mut self, new_rows: u32, new_cols: u32) {
        if new_rows != self.rows || new_cols != self.cols {
            self.rows = new_rows;
            self.cols = new_cols;
            self.dirty_rows.resize(new_rows);
            self.structural_change = true;
            self.mark_full_damage();
        }
    }

    /// Clear the structural change flag (after buffer rebuild).
    pub fn clear_structural_change(&mut self) {
        self.structural_change = false;
    }
}

/// Central damage tracker that manages per-element damage state.
#[derive(Default)]
pub struct DamageTracker {
    states: FxHashMap<NodeId, ElementDamageState>,
}

impl DamageTracker {
    pub fn new() -> Self {
        Self { states: FxHashMap::default() }
    }

    /// Register an element for damage tracking with given dimensions.
    pub fn register(&mut self, node: NodeId, rows: u32, cols: u32) {
        self.states.insert(node, ElementDamageState::new(rows, cols));
    }

    /// Unregister an element (called when element is removed from tree).
    pub fn unregister(&mut self, node: NodeId) {
        self.states.remove(&node);
    }

    /// Returns true if this element is being tracked.
    pub fn is_tracked(&self, node: NodeId) -> bool {
        self.states.contains_key(&node)
    }

    /// Get immutable access to an element's damage state.
    pub fn get(&self, node: NodeId) -> Option<&ElementDamageState> {
        self.states.get(&node)
    }

    /// Get mutable access to an element's damage state.
    pub fn get_mut(&mut self, node: NodeId) -> Option<&mut ElementDamageState> {
        self.states.get_mut(&node)
    }

    /// Record damage for an element.
    pub fn record_damage(&mut self, node: NodeId, region: DamageRegion) {
        if let Some(state) = self.states.get_mut(&node) {
            state.record_damage(region);
        }
    }

    /// Take coalesced damage for an element (frame coalescing).
    pub fn take_coalesced_damage(&mut self, node: NodeId) -> Vec<DamageRegion> {
        self.states.get_mut(&node).map(|s| s.take_coalesced_damage()).unwrap_or_default()
    }

    /// Returns all tracked node IDs.
    pub fn tracked_nodes(&self) -> Vec<NodeId> {
        self.states.keys().copied().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn damage_region_cell_count() {
        let r = DamageRegion::new(2, 5, 0, 10);
        assert_eq!(r.row_count(), 3);
        assert_eq!(r.col_count(), 10);
        assert_eq!(r.cell_count(), 30);
    }

    #[test]
    fn damage_region_overlap() {
        let a = DamageRegion::new(0, 5, 0, 10);
        let b = DamageRegion::new(3, 8, 5, 15);
        assert!(a.overlaps(&b));
        assert!(b.overlaps(&a));

        let c = DamageRegion::new(6, 8, 0, 10);
        assert!(!a.overlaps(&c));
    }

    #[test]
    fn damage_region_merge() {
        let mut a = DamageRegion::new(0, 5, 0, 10);
        let b = DamageRegion::new(3, 8, 5, 15);
        a.merge(&b);
        assert_eq!(a, DamageRegion::new(0, 8, 0, 15));
    }

    #[test]
    fn dirty_rows_basic() {
        let mut dr = DirtyRows::new(10);
        assert!(!dr.any_dirty());

        dr.mark_row(3);
        assert!(dr.is_row_dirty(3));
        assert!(!dr.is_row_dirty(2));
        assert!(dr.any_dirty());

        dr.clear();
        assert!(!dr.any_dirty());
    }

    #[test]
    fn dirty_rows_range() {
        let mut dr = DirtyRows::new(100);
        dr.mark_range(10, 20);
        for r in 0..10 {
            assert!(!dr.is_row_dirty(r));
        }
        for r in 10..20 {
            assert!(dr.is_row_dirty(r));
        }
        for r in 20..100 {
            assert!(!dr.is_row_dirty(r));
        }
    }

    #[test]
    fn dirty_rows_dirty_ranges() {
        let mut dr = DirtyRows::new(20);
        dr.mark_range(2, 5);
        dr.mark_range(10, 13);
        let ranges = dr.dirty_ranges();
        assert_eq!(ranges, vec![(2, 5), (10, 13)]);
    }

    #[test]
    fn dirty_rows_resize() {
        let mut dr = DirtyRows::new(10);
        dr.mark_row(5);
        dr.resize(20);
        assert!(dr.is_row_dirty(5));
        assert_eq!(dr.row_count(), 20);
    }

    #[test]
    fn element_damage_coalesce() {
        let mut state = ElementDamageState::new(24, 80);
        state.record_damage(DamageRegion::new(0, 3, 0, 80));
        state.record_damage(DamageRegion::new(2, 5, 0, 80));
        let coalesced = state.take_coalesced_damage();
        // The two overlapping regions should merge into one
        assert_eq!(coalesced.len(), 1);
        assert_eq!(coalesced[0], DamageRegion::new(0, 5, 0, 80));
    }

    #[test]
    fn element_damage_separate_regions() {
        let mut state = ElementDamageState::new(24, 80);
        state.record_damage(DamageRegion::new(0, 2, 0, 80));
        state.record_damage(DamageRegion::new(10, 12, 0, 80));
        let coalesced = state.take_coalesced_damage();
        assert_eq!(coalesced.len(), 2);
    }

    #[test]
    fn element_damage_resize_marks_structural() {
        let mut state = ElementDamageState::new(24, 80);
        assert!(!state.structural_change);
        state.resize(30, 100);
        assert!(state.structural_change);
        assert_eq!(state.rows, 30);
        assert_eq!(state.cols, 100);
    }

    #[test]
    fn damage_tracker_register_unregister() {
        let mut tracker = DamageTracker::new();
        let node = NodeId { index: 0, generation: 0 };
        tracker.register(node, 24, 80);
        assert!(tracker.is_tracked(node));
        tracker.unregister(node);
        assert!(!tracker.is_tracked(node));
    }

    #[test]
    fn dirty_rows_mark_all() {
        let mut dr = DirtyRows::new(200);
        dr.mark_all();
        for r in 0..200 {
            assert!(dr.is_row_dirty(r));
        }
    }

    #[test]
    fn dirty_rows_to_damage_regions() {
        let mut dr = DirtyRows::new(10);
        dr.mark_range(2, 5);
        dr.mark_range(7, 9);
        let regions = dr.to_damage_regions(80);
        assert_eq!(regions.len(), 2);
        assert_eq!(regions[0], DamageRegion::new(2, 5, 0, 80));
        assert_eq!(regions[1], DamageRegion::new(7, 9, 0, 80));
    }
}
