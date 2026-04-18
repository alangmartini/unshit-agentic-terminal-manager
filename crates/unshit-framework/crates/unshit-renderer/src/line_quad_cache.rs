//! Retained per line quad cache for cell grids.
//!
//! The batch builder runs per frame. For a terminal grid it typically
//! iterates every cell on every frame, shaping glyphs and producing
//! vertex instances. When most rows of the grid are unchanged between
//! frames, that iteration and shaping work is pure waste.
//!
//! This cache stores the vertex instances produced for a single line
//! keyed on `(NodeId, line_id)` where `line_id` is a stable identity
//! assigned by `CellGrid`. Because line identity moves with the cells
//! across scroll and shift operations, a cached line replays at its
//! new row index with no re-emission. Each entry carries a content
//! signature computed from the row cells, plus the geometry inputs
//! (origin, cell width/height, font size, opacity, clip rectangle,
//! and the glyph atlas generation) the entry was built against.
//!
//! On a subsequent frame the batch builder first computes the
//! expected signature for the row. If the cached entry matches the
//! signature and geometry, its quad and glyph instances are appended
//! to the frame batch with no shaping, no atlas lookups, and no
//! iteration beyond the clone call. A mismatch falls back to the
//! existing per cell emit path and the fresh instances replace the
//! cached entry.
//!
//! This mirrors WezTerm's stable `id` attached to `Line` appdata,
//! Kitty's `linebuf_index`, and Ghostty's `PageList` line tracking,
//! adapted to the unshit renderer's instance based pipeline.

use rustc_hash::FxHashMap;
use std::hash::Hasher;

use unshit_core::cell_grid::{Cell, CellGrid};
use unshit_core::id::NodeId;
use unshit_core::style::types::Color;

use crate::atlas::GlyphKey;
use crate::pipeline::quad::QuadInstance;
use crate::pipeline::text::GlyphInstance;

/// Stable signature of the row geometry inputs used to build the
/// cached instances. Any change in these invalidates the entry.
///
/// Floats are converted to bits so the signature is a plain `u64`
/// tuple. Exact bit equality is the correct test: any drift in the
/// rendered position invalidates the cache because the cached
/// instances carry absolute coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LineGeometrySig {
    pub origin_x_bits: u32,
    pub origin_y_bits: u32,
    pub cell_w_bits: u32,
    pub cell_h_bits: u32,
    pub font_size_bits: u32,
    pub opacity_bits: u32,
    pub clip_bits: [u32; 4],
    pub cols: u32,
    /// Glyph atlas generation. When the atlas is evicted or rebuilt
    /// the generation bumps, invalidating every cached UV.
    pub atlas_generation: u64,
}

impl LineGeometrySig {
    #[inline]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        origin_x: f32,
        origin_y: f32,
        cell_w: f32,
        cell_h: f32,
        font_size: f32,
        opacity: f32,
        clip_rect: [f32; 4],
        cols: u32,
        atlas_generation: u64,
    ) -> Self {
        Self {
            origin_x_bits: origin_x.to_bits(),
            origin_y_bits: origin_y.to_bits(),
            cell_w_bits: cell_w.to_bits(),
            cell_h_bits: cell_h.to_bits(),
            font_size_bits: font_size.to_bits(),
            opacity_bits: opacity.to_bits(),
            clip_bits: [
                clip_rect[0].to_bits(),
                clip_rect[1].to_bits(),
                clip_rect[2].to_bits(),
                clip_rect[3].to_bits(),
            ],
            cols,
            atlas_generation,
        }
    }
}

/// Content hash of every cell in a single row. Includes character,
/// foreground, background, attribute flags, and continuation flag so
/// any visible change flips the signature.
#[inline]
pub fn hash_row_cells(cells: &[Cell], row: usize, cols: usize) -> u64 {
    let start = row * cols;
    let end = start + cols;
    let mut hasher = rustc_hash::FxHasher::default();
    // Version nibble so future layout changes can invalidate old caches.
    hasher.write_u8(1);
    hasher.write_usize(cols);
    for cell in &cells[start..end] {
        hash_cell(&mut hasher, cell);
    }
    hasher.finish()
}

#[inline]
fn hash_cell(hasher: &mut rustc_hash::FxHasher, cell: &Cell) {
    hasher.write_u32(cell.ch as u32);
    hash_color(hasher, cell.fg);
    hash_color(hasher, cell.bg);
    hasher.write_u8(cell.attrs.bits());
    hasher.write_u8(if cell.wide_continuation { 1 } else { 0 });
}

#[inline]
fn hash_color(hasher: &mut rustc_hash::FxHasher, c: Color) {
    hasher.write_u8(c.r);
    hasher.write_u8(c.g);
    hasher.write_u8(c.b);
    hasher.write_u8(c.a);
}

/// Cached vertex instances for a single row.
///
/// After issue #52 Step 4 the cache tracks which cached glyph belongs to
/// which column so the renderer can splice only the damaged column range
/// on a content-signature miss with matching geometry. Backgrounds are
/// always re-emitted fresh for the full row because style runs are
/// comparatively cheap (O(cols)) and can legally shift boundaries as
/// adjacent cell styles change.
///
/// `glyph_col_index[col] == Some(i)` means `glyphs[i]` and `glyph_keys[i]`
/// belong to the cell at column `col`. A `None` entry means the cell had
/// no glyph (empty cell, wide continuation, or glyph shaping failed).
#[derive(Clone, Debug, Default)]
pub struct CachedLineState {
    pub content_sig: u64,
    pub geometry: Option<LineGeometrySig>,
    pub quads: Vec<QuadInstance>,
    pub glyphs: Vec<GlyphInstance>,
    /// Glyph atlas keys referenced by this line. Used so LRU touches
    /// cover replayed lines even when the batch builder never re
    /// rasterized them this frame.
    pub glyph_keys: Vec<GlyphKey>,
    /// Per-column index into `glyphs`/`glyph_keys`. `None` when the cell
    /// at that column produced no glyph. Length is `cols`.
    pub glyph_col_index: Vec<Option<u32>>,
    /// Row index at the time the payload was built. The vertex instances
    /// in `quads` and `glyphs` carry absolute Y positions computed from
    /// this row. Issue #77: when a line's stable `line_id` rotates to a
    /// new row via `CellGrid::scroll_up` / `shift_rows`, replay must
    /// translate Y by `(current_row - cached_row) * cell_h` so the
    /// cached payload paints at its new row slot rather than the old.
    pub cached_row: usize,
}

/// Per element cache: one `CachedLineState` per `(NodeId, line_id)` where
/// `line_id` is the stable identity assigned by `CellGrid`. The pane's
/// root element `NodeId` namespaces the cache so multiple panes never
/// collide on line identities that happen to repeat.
#[derive(Default)]
pub struct LineQuadCache {
    lines: FxHashMap<(NodeId, u64), CachedLineState>,
}

impl LineQuadCache {
    pub fn new() -> Self {
        Self { lines: FxHashMap::default() }
    }

    /// Drop every cached row. Call on full rebuild triggers
    /// (window resize, font change, stylesheet reload).
    pub fn clear(&mut self) {
        self.lines.clear();
    }

    /// Total number of cached lines, across every element.
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Returns the cached line, if any.
    pub fn get(&self, node: NodeId, line_id: u64) -> Option<&CachedLineState> {
        self.lines.get(&(node, line_id))
    }

    /// Returns the cached line when its content signature and
    /// geometry match. Bumps no internal state, so callers that
    /// only need to read a replayable entry can call this.
    pub fn lookup_replayable(
        &self,
        node: NodeId,
        line_id: u64,
        content_sig: u64,
        geometry: LineGeometrySig,
    ) -> Option<&CachedLineState> {
        let cached = self.lines.get(&(node, line_id))?;
        if cached.content_sig == content_sig && cached.geometry == Some(geometry) {
            Some(cached)
        } else {
            None
        }
    }

    /// Insert or overwrite an entry for `(node, line_id)`.
    ///
    /// `glyph_col_index` maps each column of the row to an index into
    /// `glyphs`/`glyph_keys`. Callers that do not track per-column
    /// indices can pass an empty vec, which disables the Step 4 column
    /// splice fast path for that entry. See `CachedLineState` for the
    /// full contract.
    ///
    /// `cached_row` is the row index the vertex instances were built
    /// against. `lookup_and_retarget` uses it to translate Y when the
    /// line rotates to a new row via scroll / shift without a content
    /// change (issue #77).
    #[allow(clippy::too_many_arguments)]
    pub fn store(
        &mut self,
        node: NodeId,
        line_id: u64,
        content_sig: u64,
        geometry: LineGeometrySig,
        quads: Vec<QuadInstance>,
        glyphs: Vec<GlyphInstance>,
        glyph_keys: Vec<GlyphKey>,
        glyph_col_index: Vec<Option<u32>>,
        cached_row: usize,
    ) {
        self.lines.insert(
            (node, line_id),
            CachedLineState {
                content_sig,
                geometry: Some(geometry),
                quads,
                glyphs,
                glyph_keys,
                glyph_col_index,
                cached_row,
            },
        );
    }

    /// Look up a replayable entry and retarget its vertex Y positions
    /// to `current_row` when the cached row differs. Returns the
    /// (possibly translated) entry. The translation is applied in
    /// place and `cached_row` is updated so subsequent replays at the
    /// same row hit the pure slice fast path without recomputation.
    ///
    /// Issue #77: `CellGrid::scroll_up` / `shift_rows` rotate
    /// `line_id` alongside content. The cache keyed on stable
    /// `line_id` hits for a scrolled line, but the vertex instances
    /// still carry the pre-scroll absolute Y. Without this method,
    /// the pure slice fast path replays at the old row slot and
    /// overlaps whatever the new row is rendering there.
    pub fn lookup_and_retarget(
        &mut self,
        node: NodeId,
        line_id: u64,
        content_sig: u64,
        geometry: LineGeometrySig,
        current_row: usize,
        cell_h: f32,
    ) -> Option<&CachedLineState> {
        let cached = self.lines.get_mut(&(node, line_id))?;
        if cached.content_sig != content_sig || cached.geometry != Some(geometry) {
            return None;
        }
        if cached.cached_row != current_row {
            let dy = (current_row as f32 - cached.cached_row as f32) * cell_h;
            for q in cached.quads.iter_mut() {
                q.pos[1] += dy;
            }
            for g in cached.glyphs.iter_mut() {
                g.pos[1] += dy;
            }
            cached.cached_row = current_row;
        }
        Some(cached)
    }

    /// Drop any cached lines for this node whose `line_id` is not in
    /// `retain_ids`. Used after a grid shrink or a full wipe so stale
    /// identities don't linger and leak memory.
    pub fn retain_element_ids(&mut self, node: NodeId, retain_ids: &rustc_hash::FxHashSet<u64>) {
        self.lines.retain(|(n, id), _| *n != node || retain_ids.contains(id));
    }

    /// Drop every cached row for a single element. Used on
    /// node removal.
    pub fn forget_element(&mut self, node: NodeId) {
        self.lines.retain(|(n, _), _| *n != node);
    }
}

/// Compute the hash for every row in a grid as a flat vec indexed
/// by row index. Convenience helper for tests and callers that want
/// a cheap pre pass signature snapshot.
pub fn hash_all_rows(grid: &CellGrid) -> Vec<u64> {
    let cells = grid.cells();
    let cols = grid.cols();
    (0..grid.rows()).map(|row| hash_row_cells(cells, row, cols)).collect()
}

/// Returns `true` when any cell in row `row` is marked dirty by
/// the grid's per cell dirty flags. This is the cheap fast path
/// the batch builder uses before computing the full content hash.
#[inline]
pub fn row_has_dirty_cells(grid: &CellGrid, row: usize) -> bool {
    let cols = grid.cols();
    let dirty = grid.dirty_flags();
    let start = row * cols;
    let end = start + cols;
    dirty[start..end].iter().any(|&d| d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytemuck::Zeroable;
    use unshit_core::cell_grid::CellAttrs;

    fn mk_cell(ch: char, fg: Color, bg: Color) -> Cell {
        Cell { ch, fg, bg, attrs: CellAttrs::empty(), wide_continuation: false }
    }

    fn sample_grid() -> CellGrid {
        let mut g = CellGrid::new(3, 4);
        let fg = Color { r: 255, g: 255, b: 255, a: 255 };
        let bg = Color { r: 0, g: 0, b: 0, a: 0 };
        g.set_cell(0, 0, mk_cell('H', fg, bg));
        g.set_cell(0, 1, mk_cell('i', fg, bg));
        g.set_cell(1, 0, mk_cell('A', fg, bg));
        g
    }

    #[test]
    fn row_hash_is_stable_for_same_content() {
        let g1 = sample_grid();
        let g2 = sample_grid();
        for row in 0..g1.rows() {
            let h1 = hash_row_cells(g1.cells(), row, g1.cols());
            let h2 = hash_row_cells(g2.cells(), row, g2.cols());
            assert_eq!(h1, h2, "row {row} hash must be stable across identical grids");
        }
    }

    #[test]
    fn row_hash_differs_when_a_cell_changes() {
        let mut g = sample_grid();
        let before = hash_row_cells(g.cells(), 0, g.cols());
        let fg = Color { r: 255, g: 255, b: 255, a: 255 };
        let bg = Color { r: 0, g: 0, b: 0, a: 0 };
        g.set_cell(0, 0, mk_cell('X', fg, bg));
        let after = hash_row_cells(g.cells(), 0, g.cols());
        assert_ne!(before, after, "row hash must change when a cell changes");
    }

    #[test]
    fn row_hash_unaffected_by_other_rows() {
        let mut g = sample_grid();
        let row0_before = hash_row_cells(g.cells(), 0, g.cols());
        let fg = Color { r: 255, g: 255, b: 255, a: 255 };
        let bg = Color { r: 0, g: 0, b: 0, a: 0 };
        g.set_cell(2, 3, mk_cell('!', fg, bg));
        let row0_after = hash_row_cells(g.cells(), 0, g.cols());
        assert_eq!(row0_before, row0_after, "row 0 hash must not change when row 2 changes");
    }

    #[test]
    fn row_hash_differs_when_attrs_change() {
        let mut g = sample_grid();
        let before = hash_row_cells(g.cells(), 0, g.cols());
        let fg = Color { r: 255, g: 255, b: 255, a: 255 };
        let bg = Color { r: 0, g: 0, b: 0, a: 0 };
        let mut cell = mk_cell('H', fg, bg);
        cell.attrs = CellAttrs::BOLD;
        g.set_cell(0, 0, cell);
        let after = hash_row_cells(g.cells(), 0, g.cols());
        assert_ne!(before, after, "row hash must change when attrs change");
    }

    #[test]
    fn geometry_sig_equal_for_same_inputs() {
        let a =
            LineGeometrySig::new(1.0, 2.0, 9.0, 18.0, 14.0, 1.0, [0.0, 0.0, 800.0, 600.0], 80, 7);
        let b =
            LineGeometrySig::new(1.0, 2.0, 9.0, 18.0, 14.0, 1.0, [0.0, 0.0, 800.0, 600.0], 80, 7);
        assert_eq!(a, b);
    }

    #[test]
    fn geometry_sig_differs_when_atlas_generation_changes() {
        let a =
            LineGeometrySig::new(1.0, 2.0, 9.0, 18.0, 14.0, 1.0, [0.0, 0.0, 800.0, 600.0], 80, 7);
        let b =
            LineGeometrySig::new(1.0, 2.0, 9.0, 18.0, 14.0, 1.0, [0.0, 0.0, 800.0, 600.0], 80, 9);
        assert_ne!(a, b);
    }

    #[test]
    fn geometry_sig_differs_when_cell_w_changes() {
        let a =
            LineGeometrySig::new(1.0, 2.0, 9.0, 18.0, 14.0, 1.0, [0.0, 0.0, 800.0, 600.0], 80, 7);
        let b =
            LineGeometrySig::new(1.0, 2.0, 9.5, 18.0, 14.0, 1.0, [0.0, 0.0, 800.0, 600.0], 80, 7);
        assert_ne!(a, b);
    }

    #[test]
    fn geometry_sig_differs_when_origin_changes() {
        let a =
            LineGeometrySig::new(1.0, 2.0, 9.0, 18.0, 14.0, 1.0, [0.0, 0.0, 800.0, 600.0], 80, 7);
        let b =
            LineGeometrySig::new(1.5, 2.0, 9.0, 18.0, 14.0, 1.0, [0.0, 0.0, 800.0, 600.0], 80, 7);
        assert_ne!(a, b);
    }

    #[test]
    fn cache_store_and_lookup_round_trip() {
        let mut cache = LineQuadCache::new();
        let node = NodeId { index: 0, generation: 0 };
        let sig = LineGeometrySig::new(0.0, 0.0, 9.0, 18.0, 14.0, 1.0, [0.0; 4], 80, 0);
        let quads = vec![QuadInstance::zeroed()];
        let glyphs = vec![GlyphInstance::zeroed(); 2];
        let keys =
            vec![GlyphKey { font_id: 1, glyph_id: 2, font_size_tenths: 140, subpixel_bin: 0 }];

        // Cache key is (node, line_id): use a distinct id to prove the
        // map stores what we asked for.
        let line_id: u64 = 0xdeadbeef;
        cache.store(
            node,
            line_id,
            0xdeadbeef,
            sig,
            quads.clone(),
            glyphs.clone(),
            keys.clone(),
            vec![],
            0,
        );

        let hit = cache.lookup_replayable(node, line_id, 0xdeadbeef, sig);
        assert!(hit.is_some(), "must hit cache for matching content + geometry");
        let hit = hit.unwrap();
        assert_eq!(hit.quads.len(), 1);
        assert_eq!(hit.glyphs.len(), 2);
        assert_eq!(hit.glyph_keys, keys);
    }

    #[test]
    fn cache_miss_when_content_sig_changes() {
        let mut cache = LineQuadCache::new();
        let node = NodeId { index: 0, generation: 0 };
        let sig = LineGeometrySig::new(0.0, 0.0, 9.0, 18.0, 14.0, 1.0, [0.0; 4], 80, 0);
        let line_id: u64 = 7;
        cache.store(node, line_id, 0x1111, sig, vec![], vec![], vec![], vec![], 0);
        assert!(cache.lookup_replayable(node, line_id, 0x1111, sig).is_some());
        // Different content hash -> miss.
        assert!(cache.lookup_replayable(node, line_id, 0x2222, sig).is_none());
    }

    #[test]
    fn cache_miss_when_geometry_changes() {
        let mut cache = LineQuadCache::new();
        let node = NodeId { index: 0, generation: 0 };
        let sig = LineGeometrySig::new(0.0, 0.0, 9.0, 18.0, 14.0, 1.0, [0.0; 4], 80, 0);
        let line_id: u64 = 11;
        cache.store(node, line_id, 0x1, sig, vec![], vec![], vec![], vec![], 0);
        let moved = LineGeometrySig::new(10.0, 0.0, 9.0, 18.0, 14.0, 1.0, [0.0; 4], 80, 0);
        assert!(cache.lookup_replayable(node, line_id, 0x1, moved).is_none());
    }

    #[test]
    fn cache_miss_when_atlas_generation_bumps() {
        let mut cache = LineQuadCache::new();
        let node = NodeId { index: 0, generation: 0 };
        let sig = LineGeometrySig::new(0.0, 0.0, 9.0, 18.0, 14.0, 1.0, [0.0; 4], 80, 3);
        let line_id: u64 = 42;
        cache.store(node, line_id, 0xaaaa, sig, vec![], vec![], vec![], vec![], 0);
        let bumped = LineGeometrySig::new(0.0, 0.0, 9.0, 18.0, 14.0, 1.0, [0.0; 4], 80, 4);
        assert!(
            cache.lookup_replayable(node, line_id, 0xaaaa, bumped).is_none(),
            "atlas generation bump must invalidate cached rows"
        );
    }

    #[test]
    fn cache_retain_element_ids_drops_lines_not_in_set() {
        use rustc_hash::FxHashSet;
        let mut cache = LineQuadCache::new();
        let node = NodeId { index: 0, generation: 0 };
        let sig = LineGeometrySig::new(0.0, 0.0, 9.0, 18.0, 14.0, 1.0, [0.0; 4], 80, 0);
        for id in 0..5u64 {
            cache.store(node, id, id, sig, vec![], vec![], vec![], vec![], 0);
        }
        assert_eq!(cache.len(), 5);
        let mut retain: FxHashSet<u64> = FxHashSet::default();
        retain.insert(0);
        retain.insert(1);
        cache.retain_element_ids(node, &retain);
        assert_eq!(cache.len(), 2);
        assert!(cache.get(node, 0).is_some());
        assert!(cache.get(node, 1).is_some());
        assert!(cache.get(node, 2).is_none());
    }

    #[test]
    fn cache_forget_element_drops_all_rows_for_that_node() {
        let mut cache = LineQuadCache::new();
        let a = NodeId { index: 0, generation: 0 };
        let b = NodeId { index: 1, generation: 0 };
        let sig = LineGeometrySig::new(0.0, 0.0, 9.0, 18.0, 14.0, 1.0, [0.0; 4], 80, 0);
        cache.store(a, 0, 1, sig, vec![], vec![], vec![], vec![], 0);
        cache.store(b, 0, 2, sig, vec![], vec![], vec![], vec![], 0);
        cache.forget_element(a);
        assert!(cache.get(a, 0).is_none());
        assert!(cache.get(b, 0).is_some());
    }

    #[test]
    fn row_has_dirty_cells_detects_single_dirty_cell() {
        let mut g = CellGrid::new(3, 4);
        g.clear_dirty();
        assert!(!row_has_dirty_cells(&g, 0));
        let fg = Color { r: 255, g: 255, b: 255, a: 255 };
        let bg = Color { r: 0, g: 0, b: 0, a: 0 };
        g.set_cell(1, 2, mk_cell('!', fg, bg));
        assert!(row_has_dirty_cells(&g, 1));
        assert!(!row_has_dirty_cells(&g, 0));
        assert!(!row_has_dirty_cells(&g, 2));
    }

    #[test]
    fn scroll_survives_cache_because_line_id_follows_content() {
        // With the cache keyed on stable `line_id`, a scroll that moves
        // the same logical line to a different row index is a no-op for
        // the cache: looking up by the line's identity still hits. This
        // is the core win of the line-identity design: surviving lines
        // replay their cached payload without a re-emission.
        let mut cache = LineQuadCache::new();
        let node = NodeId { index: 0, generation: 0 };
        let sig = LineGeometrySig::new(0.0, 0.0, 9.0, 18.0, 14.0, 1.0, [0.0; 4], 80, 0);
        // Populate three distinct lines with distinct content hashes.
        let id_a: u64 = 100;
        let id_b: u64 = 101;
        let id_c: u64 = 102;
        cache.store(node, id_a, 0xaa, sig, vec![], vec![], vec![], vec![], 0);
        cache.store(node, id_b, 0xbb, sig, vec![], vec![], vec![], vec![], 0);
        cache.store(node, id_c, 0xcc, sig, vec![], vec![], vec![], vec![], 0);

        // After a scroll_up(1) inside CellGrid, the lines that held ids
        // 101 and 102 have rotated to earlier row indices while keeping
        // their identity and content. The cache must still hit for them.
        assert!(cache.lookup_replayable(node, id_b, 0xbb, sig).is_some());
        assert!(cache.lookup_replayable(node, id_c, 0xcc, sig).is_some());
    }

    #[test]
    fn lookup_and_retarget_translates_y_when_row_changes() {
        // Regression for issue #77. Cached vertex instances carry
        // absolute Y (origin_y + row * cell_h). When a line's stable
        // `line_id` rotates to a different row via
        // `CellGrid::scroll_up` / `shift_rows`, the cache hit path
        // must translate the payload's Y to the new row slot. Without
        // this, scrolled lines paint at their pre-scroll Y and overlap
        // whatever now renders at the old slot, which is the overlap
        // observed after heavy PTY output like `dir`.
        let mut cache = LineQuadCache::new();
        let node = NodeId { index: 0, generation: 0 };
        let cell_h: f32 = 18.0;
        let origin_y: f32 = 0.0;
        let sig = LineGeometrySig::new(0.0, origin_y, 9.0, cell_h, 14.0, 1.0, [0.0; 4], 80, 0);
        let line_id: u64 = 42;
        let cached_row: usize = 10;
        let cached_y = origin_y + cached_row as f32 * cell_h;

        let mut quad = QuadInstance::zeroed();
        quad.pos = [0.0, cached_y];
        quad.size = [9.0, cell_h];
        let mut glyph = GlyphInstance::zeroed();
        glyph.pos = [0.0, cached_y];
        glyph.size = [9.0, cell_h];
        cache.store(
            node,
            line_id,
            0xbeef,
            sig,
            vec![quad],
            vec![glyph],
            vec![],
            vec![],
            cached_row,
        );

        let current_row: usize = 9;
        let expected_y = origin_y + current_row as f32 * cell_h;
        let hit = cache
            .lookup_and_retarget(node, line_id, 0xbeef, sig, current_row, cell_h)
            .expect("cache must hit by stable line_id after scroll");
        assert!(
            (hit.quads[0].pos[1] - expected_y).abs() < f32::EPSILON,
            "quad Y must translate from {cached_y} to {expected_y} on row change, got {}",
            hit.quads[0].pos[1],
        );
        assert!(
            (hit.glyphs[0].pos[1] - expected_y).abs() < f32::EPSILON,
            "glyph Y must translate from {cached_y} to {expected_y} on row change, got {}",
            hit.glyphs[0].pos[1],
        );

        // Second replay at the same current_row hits the fast path:
        // cached_row is already up to date, no further translation.
        let hit2 = cache
            .lookup_and_retarget(node, line_id, 0xbeef, sig, current_row, cell_h)
            .expect("cache must still hit on second replay");
        assert!(
            (hit2.quads[0].pos[1] - expected_y).abs() < f32::EPSILON,
            "second replay must keep Y at {expected_y}, got {}",
            hit2.quads[0].pos[1],
        );
    }

    #[test]
    fn lookup_and_retarget_is_noop_when_row_unchanged() {
        // When the cached row matches the current row, the method must
        // leave the payload untouched. This is the steady-state hot
        // path: most frames reuse entries at the same row they were
        // stored from, so translation cost stays paid only on scroll.
        let mut cache = LineQuadCache::new();
        let node = NodeId { index: 0, generation: 0 };
        let cell_h: f32 = 18.0;
        let sig = LineGeometrySig::new(0.0, 0.0, 9.0, cell_h, 14.0, 1.0, [0.0; 4], 80, 0);
        let line_id: u64 = 7;
        let stored_y = 5.0 * cell_h;
        let mut quad = QuadInstance::zeroed();
        quad.pos = [0.0, stored_y];
        cache.store(node, line_id, 0xfeed, sig, vec![quad], vec![], vec![], vec![], 5);

        let hit = cache.lookup_and_retarget(node, line_id, 0xfeed, sig, 5, cell_h).unwrap();
        assert!(
            (hit.quads[0].pos[1] - stored_y).abs() < f32::EPSILON,
            "Y must remain at {stored_y} when cached_row == current_row",
        );
    }

    #[test]
    fn retain_does_not_touch_other_nodes() {
        use rustc_hash::FxHashSet;
        let mut cache = LineQuadCache::new();
        let a = NodeId { index: 0, generation: 0 };
        let b = NodeId { index: 1, generation: 0 };
        let sig = LineGeometrySig::new(0.0, 0.0, 9.0, 18.0, 14.0, 1.0, [0.0; 4], 80, 0);
        cache.store(a, 5, 1, sig, vec![], vec![], vec![], vec![], 0);
        cache.store(b, 5, 2, sig, vec![], vec![], vec![], vec![], 0);
        // Retain nothing under `a`; `b`'s entries must survive.
        let retain: FxHashSet<u64> = FxHashSet::default();
        cache.retain_element_ids(a, &retain);
        assert!(cache.get(a, 5).is_none());
        assert!(cache.get(b, 5).is_some());
    }

    #[test]
    fn hash_all_rows_length_matches_row_count() {
        let grid = sample_grid();
        let hashes = hash_all_rows(&grid);
        assert_eq!(hashes.len(), grid.rows());
        // First row has content, so its hash is non zero.
        assert_ne!(hashes[0], 0);
    }

    #[test]
    fn clean_row_reuses_cached_bytes_without_allocation() {
        // The core win of the cache: on a clean row we return borrowed
        // instances whose payload can be appended to the batch via
        // `extend_from_slice`. This test asserts the cached payload
        // remains byte-for-byte identical across lookups.
        let mut cache = LineQuadCache::new();
        let node = NodeId { index: 0, generation: 0 };
        let sig = LineGeometrySig::new(0.0, 0.0, 9.0, 18.0, 14.0, 1.0, [0.0; 4], 80, 0);

        let quad = QuadInstance {
            pos: [1.5, 2.5],
            size: [9.0, 18.0],
            color: [0.1, 0.2, 0.3, 1.0],
            border_color: [0.0; 4],
            border_width: [0.0; 4],
            border_radius: [0.0; 4],
            clip_rect: [0.0; 4],
            shadow_color: [0.0; 4],
            shadow_offset: [0.0; 2],
            shadow_params: [0.0; 2],
            shadow_spread: [0.0; 2],
            gradient_stop_colors: [[0.0; 4]; crate::pipeline::quad::MAX_GRADIENT_STOPS],
            gradient_stop_positions: [0.0; crate::pipeline::quad::MAX_GRADIENT_STOPS],
            gradient_params: [0.0; 4],
            gradient_extra: [0.0; 4],
        };
        let line_id: u64 = 9;
        cache.store(node, line_id, 42, sig, vec![quad], vec![], vec![], vec![], 0);

        let hit1 = cache.lookup_replayable(node, line_id, 42, sig).unwrap();
        let first_bytes = bytemuck::bytes_of(&hit1.quads[0]).to_vec();

        let hit2 = cache.lookup_replayable(node, line_id, 42, sig).unwrap();
        let second_bytes = bytemuck::bytes_of(&hit2.quads[0]).to_vec();

        assert_eq!(first_bytes, second_bytes);
    }

    // -- Issue #52 Step 3 regression tests ---------------------------------

    #[test]
    fn line_quad_cache_survives_scroll() {
        // Regression: Step 3 of issue #52 keyed the cache on stable
        // `line_id` so a scroll no longer forces every row to miss. A
        // line stored under id 42 must still be retrievable after a
        // simulated scroll (i.e., the "row index" of that line has
        // changed in the grid but its identity has not).
        let mut cache = LineQuadCache::new();
        let node = NodeId { index: 0, generation: 0 };
        let sig = LineGeometrySig::new(0.0, 0.0, 9.0, 18.0, 14.0, 1.0, [0.0; 4], 80, 0);
        let line_id: u64 = 42;
        cache.store(node, line_id, 0xbeef, sig, vec![], vec![], vec![], vec![], 0);

        // Simulate a scroll: the same line identity now occupies a
        // different row. Under the new cache key, this is a no-op.
        assert!(
            cache.get(node, line_id).is_some(),
            "cache lookup by stable line_id must survive scroll",
        );
        assert!(
            cache.lookup_replayable(node, line_id, 0xbeef, sig).is_some(),
            "replayable cache lookup by line_id must survive scroll",
        );
    }

    #[test]
    fn line_quad_cache_invalidates_on_content_change() {
        // Regression: content-signature gating still invalidates the
        // cache when the logical line's content changes, even though
        // line identity is stable. This is the Red path of issue #52
        // Step 3's correctness invariant.
        let mut cache = LineQuadCache::new();
        let node = NodeId { index: 0, generation: 0 };
        let sig = LineGeometrySig::new(0.0, 0.0, 9.0, 18.0, 14.0, 1.0, [0.0; 4], 80, 0);
        let line_id: u64 = 77;
        cache.store(node, line_id, 0xa1a1, sig, vec![], vec![], vec![], vec![], 0);
        // Same id, new content hash -> miss.
        assert!(
            cache.lookup_replayable(node, line_id, 0xb2b2, sig).is_none(),
            "content-sig mismatch must miss even when line identity is stable",
        );
    }
}
