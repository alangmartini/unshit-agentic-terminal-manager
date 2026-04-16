//! Retained per line quad cache for cell grids.
//!
//! The batch builder runs per frame. For a terminal grid it typically
//! iterates every cell on every frame, shaping glyphs and producing
//! vertex instances. When most rows of the grid are unchanged between
//! frames, that iteration and shaping work is pure waste.
//!
//! This cache stores the vertex instances produced for a single row
//! keyed on `(NodeId, row_index)`. Each entry carries a content
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
//! This mirrors the WezTerm `line_quad_cache` pattern from commit
//! f03dc68f96161229f8c213c13df9a8dacc9d5fc6 (2022 08 26), adapted to
//! the unshit renderer's instance based pipeline.

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
}

/// Per element cache: one `CachedLineState` per `(NodeId, row)`.
#[derive(Default)]
pub struct LineQuadCache {
    lines: FxHashMap<(NodeId, u32), CachedLineState>,
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

    /// Total number of cached rows, across every element.
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Returns the cached line, if any.
    pub fn get(&self, node: NodeId, row: u32) -> Option<&CachedLineState> {
        self.lines.get(&(node, row))
    }

    /// Returns the cached line when its content signature and
    /// geometry match. Bumps no internal state, so callers that
    /// only need to read a replayable entry can call this.
    pub fn lookup_replayable(
        &self,
        node: NodeId,
        row: u32,
        content_sig: u64,
        geometry: LineGeometrySig,
    ) -> Option<&CachedLineState> {
        let cached = self.lines.get(&(node, row))?;
        if cached.content_sig == content_sig && cached.geometry == Some(geometry) {
            Some(cached)
        } else {
            None
        }
    }

    /// Insert or overwrite an entry for `(node, row)`.
    pub fn store(
        &mut self,
        node: NodeId,
        row: u32,
        content_sig: u64,
        geometry: LineGeometrySig,
        quads: Vec<QuadInstance>,
        glyphs: Vec<GlyphInstance>,
        glyph_keys: Vec<GlyphKey>,
    ) {
        self.lines.insert(
            (node, row),
            CachedLineState { content_sig, geometry: Some(geometry), quads, glyphs, glyph_keys },
        );
    }

    /// Drop any cached rows at row indices >= `rows` for this node.
    /// Used after a grid shrink so stale rows don't linger.
    pub fn truncate_element(&mut self, node: NodeId, rows: u32) {
        self.lines.retain(|(n, r), _| !(*n == node && *r >= rows));
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

        cache.store(node, 3, 0xdeadbeef, sig, quads.clone(), glyphs.clone(), keys.clone());

        let hit = cache.lookup_replayable(node, 3, 0xdeadbeef, sig);
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
        cache.store(node, 0, 0x1111, sig, vec![], vec![], vec![]);
        assert!(cache.lookup_replayable(node, 0, 0x1111, sig).is_some());
        // Different content hash -> miss.
        assert!(cache.lookup_replayable(node, 0, 0x2222, sig).is_none());
    }

    #[test]
    fn cache_miss_when_geometry_changes() {
        let mut cache = LineQuadCache::new();
        let node = NodeId { index: 0, generation: 0 };
        let sig = LineGeometrySig::new(0.0, 0.0, 9.0, 18.0, 14.0, 1.0, [0.0; 4], 80, 0);
        cache.store(node, 0, 0x1, sig, vec![], vec![], vec![]);
        let moved = LineGeometrySig::new(10.0, 0.0, 9.0, 18.0, 14.0, 1.0, [0.0; 4], 80, 0);
        assert!(cache.lookup_replayable(node, 0, 0x1, moved).is_none());
    }

    #[test]
    fn cache_miss_when_atlas_generation_bumps() {
        let mut cache = LineQuadCache::new();
        let node = NodeId { index: 0, generation: 0 };
        let sig = LineGeometrySig::new(0.0, 0.0, 9.0, 18.0, 14.0, 1.0, [0.0; 4], 80, 3);
        cache.store(node, 0, 0xaaaa, sig, vec![], vec![], vec![]);
        let bumped = LineGeometrySig::new(0.0, 0.0, 9.0, 18.0, 14.0, 1.0, [0.0; 4], 80, 4);
        assert!(
            cache.lookup_replayable(node, 0, 0xaaaa, bumped).is_none(),
            "atlas generation bump must invalidate cached rows"
        );
    }

    #[test]
    fn cache_truncate_element_drops_rows_past_new_height() {
        let mut cache = LineQuadCache::new();
        let node = NodeId { index: 0, generation: 0 };
        let sig = LineGeometrySig::new(0.0, 0.0, 9.0, 18.0, 14.0, 1.0, [0.0; 4], 80, 0);
        for row in 0..5u32 {
            cache.store(node, row, row as u64, sig, vec![], vec![], vec![]);
        }
        assert_eq!(cache.len(), 5);
        cache.truncate_element(node, 2);
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
        cache.store(a, 0, 1, sig, vec![], vec![], vec![]);
        cache.store(b, 0, 2, sig, vec![], vec![], vec![]);
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
        cache.store(node, 0, 42, sig, vec![quad], vec![], vec![]);

        let hit1 = cache.lookup_replayable(node, 0, 42, sig).unwrap();
        let first_bytes = bytemuck::bytes_of(&hit1.quads[0]).to_vec();

        let hit2 = cache.lookup_replayable(node, 0, 42, sig).unwrap();
        let second_bytes = bytemuck::bytes_of(&hit2.quads[0]).to_vec();

        assert_eq!(first_bytes, second_bytes);
    }
}
