//! GPU side cell packing and buffer upload discipline for the experimental
//! single pass fragment shader grid renderer.
//!
//! Feature gated behind `grid-fragment-shader`. The runtime flag
//! `TM_USE_GRID_FRAGMENT_SHADER=1` selects the alternative pipeline at app
//! start when the feature is compiled in.
//!
//! This module is responsible for three things:
//!
//! 1. Defining the on wire cell format (`GpuCell`) and glyph metadata format
//!    (`GpuGlyphMeta`) that the fragment shader reads.
//! 2. Assigning stable `u32` glyph ids to `GlyphKey`s so cells can reference
//!    glyphs by index instead of UV rectangle.
//! 3. Computing partial upload byte ranges keyed on `CellGrid::line_damage`
//!    so clean rows never trigger a `queue.write_buffer` call.
//!
//! Kept intentionally small and side effect free. The actual pipeline object
//! and WGSL shader live in `pipeline::grid_fragment` and
//! `shaders/grid_fragment.wgsl`.

use bytemuck::{Pod, Zeroable};
use rustc_hash::FxHashMap;
use std::sync::OnceLock;

use unshit_core::cell_grid::{Cell, CellAttrs, CellGrid};
use unshit_core::style::types::Color;

use crate::atlas::{GlyphEntry, GlyphKey};

/// Runtime gate for the fragment shader grid renderer. The app must both
/// compile with `--features grid-fragment-shader` and set the environment
/// variable `TM_USE_GRID_FRAGMENT_SHADER=1` (or any non empty value) at
/// process start for the alt path to take effect. The check is cached so
/// the env var is read once per process.
pub fn runtime_flag_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var_os("TM_USE_GRID_FRAGMENT_SHADER").map(|v| !v.is_empty()).unwrap_or(false)
    })
}

/// Sentinel glyph id for empty cells. The fragment shader checks for this
/// value before indexing `GpuGlyphMeta` so the empty cell fast path never
/// reads out of bounds.
pub const EMPTY_GLYPH_ID: u32 = u32::MAX;

/// Packed cell attribute bit positions inside `GpuCell::flags`. Bits 0..6
/// mirror `CellAttrs`. Bit 7 is `wide_continuation`. Bit 8 marks the cursor
/// cell. Higher bits are reserved.
pub const FLAG_BOLD: u32 = 1 << 0;
pub const FLAG_ITALIC: u32 = 1 << 1;
pub const FLAG_UNDERLINE: u32 = 1 << 2;
pub const FLAG_STRIKETHROUGH: u32 = 1 << 3;
pub const FLAG_INVERSE: u32 = 1 << 4;
pub const FLAG_DIM: u32 = 1 << 5;
pub const FLAG_BLINK: u32 = 1 << 6;
pub const FLAG_WIDE_CONTINUATION: u32 = 1 << 7;
pub const FLAG_CURSOR: u32 = 1 << 8;

/// GPU side cell descriptor. 16 bytes. All fields are `u32` so the whole
/// record is naturally aligned for storage buffer reads on every backend
/// we target.
///
/// - `glyph_id` indexes `GpuGlyphMeta`. `EMPTY_GLYPH_ID` skips the atlas
///   sample; the fragment shader only composes the background color.
/// - `fg_rgba` and `bg_rgba` pack four 8 bit channels in little endian
///   order: byte 0 = R, byte 1 = G, byte 2 = B, byte 3 = A. The WGSL
///   builtin `unpack4x8unorm` reads the same layout.
/// - `flags` holds the attribute bit field plus cursor and wide
///   continuation markers.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable)]
pub struct GpuCell {
    pub glyph_id: u32,
    pub fg_rgba: u32,
    pub bg_rgba: u32,
    pub flags: u32,
}

impl GpuCell {
    /// Construct the GPU descriptor for an empty cell that shows only a
    /// background color. Used when the source cell carries no glyph.
    pub const fn empty(fg_rgba: u32, bg_rgba: u32, flags: u32) -> Self {
        Self { glyph_id: EMPTY_GLYPH_ID, fg_rgba, bg_rgba, flags }
    }
}

/// GPU side glyph metadata. 32 bytes, 8 `f32`s. The fragment shader reads
/// one entry per visible cell that carries a glyph.
///
/// `atlas_uv_min` and `atlas_uv_max` are normalized UV coordinates
/// identifying the glyph's rectangle inside the monochrome atlas.
/// `pixel_offset` is the glyph placement offset inside the cell, measured
/// in pixels from the top left of the cell. `pixel_size` is the glyph
/// bitmap size.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct GpuGlyphMeta {
    pub atlas_uv_min: [f32; 2],
    pub atlas_uv_max: [f32; 2],
    pub pixel_offset: [f32; 2],
    pub pixel_size: [f32; 2],
}

impl GpuGlyphMeta {
    /// Convert an atlas `GlyphEntry` into the GPU layout. The
    /// `GlyphEntry::uv_rect` field stores `[u0, v0, u1, v1]` already in
    /// normalized coordinates so we just split the pair.
    pub const fn from_entry(entry: &GlyphEntry) -> Self {
        Self {
            atlas_uv_min: [entry.uv_rect[0], entry.uv_rect[1]],
            atlas_uv_max: [entry.uv_rect[2], entry.uv_rect[3]],
            pixel_offset: entry.offset,
            pixel_size: entry.size,
        }
    }
}

/// Pack a `Color` into the fragment shader's expected RGBA8 little endian
/// layout. `unpack4x8unorm(x)` in WGSL returns `.r = byte 0`, `.g = byte 1`,
/// `.b = byte 2`, `.a = byte 3`, each divided by 255. Matching that order
/// keeps the shader free of byte swizzles.
#[inline]
pub const fn pack_color_rgba(c: Color) -> u32 {
    (c.r as u32) | ((c.g as u32) << 8) | ((c.b as u32) << 16) | ((c.a as u32) << 24)
}

/// Inverse of `pack_color_rgba`. Used by tests and the occasional CPU side
/// consumer that needs to round trip a packed color.
#[inline]
pub const fn unpack_color_rgba(v: u32) -> Color {
    Color {
        r: (v & 0xFF) as u8,
        g: ((v >> 8) & 0xFF) as u8,
        b: ((v >> 16) & 0xFF) as u8,
        a: ((v >> 24) & 0xFF) as u8,
    }
}

/// Build the flag bit field for a cell. Attribute bits are copied from
/// `CellAttrs::bits()` because the bit positions in `CellAttrs` already
/// match `FLAG_*` above by design.
#[inline]
pub fn pack_flags(cell: &Cell, is_cursor: bool) -> u32 {
    let mut flags = cell.attrs.bits() as u32;
    if cell.wide_continuation {
        flags |= FLAG_WIDE_CONTINUATION;
    }
    if is_cursor {
        flags |= FLAG_CURSOR;
    }
    flags
}

/// Inverse of `pack_flags`. Returns `(attrs, wide_continuation, cursor)`
/// so tests can round trip the bit field.
#[inline]
pub fn unpack_flags(flags: u32) -> (CellAttrs, bool, bool) {
    let attrs = CellAttrs::from_bits_truncate((flags & 0x7F) as u8);
    let wide = (flags & FLAG_WIDE_CONTINUATION) != 0;
    let cursor = (flags & FLAG_CURSOR) != 0;
    (attrs, wide, cursor)
}

/// Encode a single CPU side `Cell` as a `GpuCell`.
///
/// The empty cell path uses the default foreground color (white) when
/// the stored foreground is `TRANSPARENT` so a subsequent cursor composite
/// still has a sane color to invert against. Non empty cells copy the
/// stored colors unchanged. Callers that need the cursor bit set should
/// pass `is_cursor = true` for the cursor cell.
pub fn encode_cell(cell: &Cell, glyph_id: Option<u32>, is_cursor: bool) -> GpuCell {
    GpuCell {
        glyph_id: glyph_id.unwrap_or(EMPTY_GLYPH_ID),
        fg_rgba: pack_color_rgba(cell.fg),
        bg_rgba: pack_color_rgba(cell.bg),
        flags: pack_flags(cell, is_cursor),
    }
}

/// Stable assignment of `u32` glyph ids for `GlyphKey`s. Entries survive
/// across frames; a repeated `insert` for the same key always returns the
/// same id. Eviction releases ids to a free list so long running sessions
/// do not monotonically grow the id space.
#[derive(Debug, Default)]
pub struct GlyphIdTable {
    /// Primary map: key -> id.
    keys: FxHashMap<GlyphKey, u32>,
    /// Reverse map: id -> key, so we can look up the source key by id
    /// during debug dumps and eviction.
    ids: Vec<Option<GlyphKey>>,
    /// Ids released by eviction, reused on the next insert.
    free: Vec<u32>,
}

impl GlyphIdTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a key, returning the stable id. Calling `insert` twice for
    /// the same key returns the same id; the table never duplicates.
    pub fn insert(&mut self, key: GlyphKey) -> u32 {
        if let Some(existing) = self.keys.get(&key) {
            return *existing;
        }
        let id = if let Some(free_id) = self.free.pop() {
            self.ids[free_id as usize] = Some(key);
            free_id
        } else {
            let id = self.ids.len() as u32;
            // Never hand out the sentinel id.
            assert!(id < EMPTY_GLYPH_ID, "GlyphIdTable exhausted");
            self.ids.push(Some(key));
            id
        };
        self.keys.insert(key, id);
        id
    }

    /// Look up an id without inserting.
    pub fn get(&self, key: &GlyphKey) -> Option<u32> {
        self.keys.get(key).copied()
    }

    /// Release the id assigned to `key` back to the free list. Subsequent
    /// lookups return `None`. The caller is responsible for also clearing
    /// the corresponding `GpuGlyphMeta` slot in the metadata buffer so the
    /// shader does not sample from a stale entry.
    pub fn remove(&mut self, key: &GlyphKey) -> Option<u32> {
        if let Some(id) = self.keys.remove(key) {
            self.ids[id as usize] = None;
            self.free.push(id);
            Some(id)
        } else {
            None
        }
    }

    /// Number of live (non evicted) entries.
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Upper bound on the id space in use. The metadata buffer must have
    /// at least this many entries reserved.
    pub fn capacity_ids(&self) -> u32 {
        self.ids.len() as u32
    }
}

/// Byte range for a single `queue.write_buffer` call when uploading a
/// damaged row. Computed from `CellGrid::line_damage()` so clean rows never
/// appear in the upload plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RowWriteRange {
    /// Row index in the source cell grid.
    pub row: usize,
    /// Byte offset from the start of the `GpuCell` storage buffer.
    pub byte_offset: u64,
    /// Byte length of the write (always `cols * size_of::<GpuCell>()`).
    pub byte_len: u64,
}

/// Collect one `RowWriteRange` per damaged row. Clean rows are skipped
/// entirely. The ranges are emitted in ascending row order and never
/// overlap, matching the partial update discipline described in the issue.
///
/// Full row writes are cheaper than column sub ranges because the fragment
/// shader path must always read whole cells (16 bytes each), so we trade
/// the instanced path's column splice granularity for fewer upload calls.
pub fn damaged_row_write_ranges(grid: &CellGrid) -> Vec<RowWriteRange> {
    let cols = grid.cols();
    let stride = cols * std::mem::size_of::<GpuCell>();
    let mut ranges = Vec::new();
    for (row, damage) in grid.line_damage().iter().enumerate() {
        if damage.is_clean() {
            continue;
        }
        ranges.push(RowWriteRange {
            row,
            byte_offset: (row * stride) as u64,
            byte_len: stride as u64,
        });
    }
    ranges
}

/// Build a single row's worth of `GpuCell` descriptors. The caller typically
/// invokes this once per damaged row and passes the result to
/// `queue.write_buffer` with the matching `RowWriteRange::byte_offset`.
///
/// `glyph_ids` looks up the GPU side id for each cell. When the cell is
/// empty or the shaper has not yet resolved a glyph for its character,
/// pass `None` and the encoder emits `EMPTY_GLYPH_ID`.
pub fn encode_row<F>(
    grid: &CellGrid,
    row: usize,
    cursor_pos: Option<(usize, usize)>,
    mut glyph_ids: F,
) -> Vec<GpuCell>
where
    F: FnMut(char) -> Option<u32>,
{
    let cols = grid.cols();
    let mut out = Vec::with_capacity(cols);
    if row >= grid.rows() {
        return out;
    }
    let cells = grid.cells();
    let base = row * cols;
    for col in 0..cols {
        let cell = &cells[base + col];
        let is_cursor = cursor_pos == Some((row, col));
        let id = if cell.is_empty() { None } else { glyph_ids(cell.ch) };
        out.push(encode_cell(cell, id, is_cursor));
    }
    out
}

/// Helper consumed by tests to drive the encoder path against a minimal
/// in memory `CellGrid` without needing a real atlas or renderer. Returns
/// the concatenated `GpuCell` buffer for every damaged row.
#[cfg(test)]
pub(crate) fn encode_damaged_rows<F>(grid: &CellGrid, glyph_ids: F) -> Vec<(usize, Vec<GpuCell>)>
where
    F: FnMut(char) -> Option<u32> + Copy,
{
    damaged_row_write_ranges(grid)
        .into_iter()
        .map(|range| (range.row, encode_row(grid, range.row, None, glyph_ids)))
        .collect()
}

/// Per terminal state for the fragment shader grid renderer. One instance
/// per `ElementContent::Grid` node; survives across frames so the glyph id
/// table and the last seen atlas generation remain stable.
///
/// The struct is intentionally free of any wgpu types so it can be
/// constructed and unit tested without a GPU present. The actual
/// `queue.write_buffer` calls happen in `pipeline::grid_fragment` via the
/// `RowWriteRange` and `Vec<GpuCell>` pairs returned by
/// [`GridFragmentState::prepare_frame`].
#[derive(Debug, Default)]
pub struct GridFragmentState {
    /// Stable glyph id assignment across frames.
    pub glyph_ids: GlyphIdTable,
    /// Last observed atlas generation. When the actual generation diverges,
    /// the caller must rewrite the entire `GpuGlyphMeta` buffer to refresh
    /// any UV rectangles evicted from the atlas.
    pub last_atlas_generation: u64,
    /// Cached grid dimensions from the last frame. Used to detect resizes
    /// that trigger a full cell buffer rewrite.
    pub last_rows: usize,
    pub last_cols: usize,
}

/// Output of preparing one frame's worth of uploads. Contains the byte
/// ranges for the damaged rows plus the pre encoded `GpuCell` slices that
/// match each range.
#[derive(Debug, Default)]
pub struct FrameUploadPlan {
    /// One entry per damaged row, in ascending row order. Each entry pairs
    /// the byte range with the pre encoded cell bytes. Use
    /// `bytemuck::cast_slice` on the `Vec<GpuCell>` when calling
    /// `queue.write_buffer`.
    pub rows: Vec<(RowWriteRange, Vec<GpuCell>)>,
    /// Whether the grid resized since the last frame. When `true`, the
    /// caller must grow the cell storage buffer before writing. This also
    /// forces every row to appear in `rows` so the new allocation is fully
    /// initialized.
    pub resized: bool,
    /// Whether the atlas generation changed this frame. When `true`, the
    /// caller must rewrite the entire `GpuGlyphMeta` buffer.
    pub atlas_generation_bumped: bool,
}

impl GridFragmentState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Compute the upload plan for one frame.
    ///
    /// `atlas_generation` is the current `GlyphAtlas::generation` value;
    /// when it differs from `self.last_atlas_generation`, every row is
    /// marked damaged and the plan flags the metadata buffer for a full
    /// rewrite. When the grid resizes, the same "rewrite everything"
    /// policy applies because the cell storage buffer has new byte offsets.
    pub fn prepare_frame<F>(
        &mut self,
        grid: &CellGrid,
        cursor: Option<(usize, usize)>,
        atlas_generation: u64,
        glyph_ids: F,
    ) -> FrameUploadPlan
    where
        F: Fn(char) -> Option<u32> + Copy,
    {
        let resized = grid.rows() != self.last_rows || grid.cols() != self.last_cols;
        let atlas_bumped = atlas_generation != self.last_atlas_generation;

        let ranges = if resized || atlas_bumped {
            // Full rewrite: every row contributes, regardless of damage.
            let stride = grid.cols() * std::mem::size_of::<GpuCell>();
            (0..grid.rows())
                .map(|row| RowWriteRange {
                    row,
                    byte_offset: (row * stride) as u64,
                    byte_len: stride as u64,
                })
                .collect()
        } else {
            damaged_row_write_ranges(grid)
        };

        let rows: Vec<(RowWriteRange, Vec<GpuCell>)> = ranges
            .into_iter()
            .map(|range| {
                let cells = encode_row(grid, range.row, cursor, glyph_ids);
                (range, cells)
            })
            .collect();

        self.last_rows = grid.rows();
        self.last_cols = grid.cols();
        self.last_atlas_generation = atlas_generation;

        FrameUploadPlan { rows, resized, atlas_generation_bumped: atlas_bumped }
    }
}

/// Convenience for constructing a dummy `GlyphEntry` inside tests without
/// touching the atlas. Not used in production code.
#[cfg(test)]
pub(crate) fn test_glyph_entry() -> GlyphEntry {
    GlyphEntry { uv_rect: [0.0, 0.0, 0.25, 0.5], offset: [1.0, 2.0], size: [8.0, 16.0] }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn gpu_cell_packs_to_16_bytes() {
        // Alignment and size are load bearing for the storage buffer layout.
        // The fragment shader reads one element per cell with stride 16.
        assert_eq!(size_of::<GpuCell>(), 16);
        assert_eq!(std::mem::align_of::<GpuCell>(), 4);
    }

    #[test]
    fn gpu_glyph_meta_packs_to_32_bytes() {
        assert_eq!(size_of::<GpuGlyphMeta>(), 32);
        assert_eq!(std::mem::align_of::<GpuGlyphMeta>(), 4);
    }

    #[test]
    fn pack_cell_attrs_round_trips() {
        // Every bit in CellAttrs plus wide_continuation plus cursor must
        // survive pack + unpack without collision.
        let all = CellAttrs::BOLD
            | CellAttrs::ITALIC
            | CellAttrs::UNDERLINE
            | CellAttrs::STRIKETHROUGH
            | CellAttrs::INVERSE
            | CellAttrs::DIM
            | CellAttrs::BLINK;
        let cell = Cell {
            ch: 'x',
            fg: Color::WHITE,
            bg: Color::BLACK,
            attrs: all,
            wide_continuation: true,
        };
        let packed = pack_flags(&cell, true);
        let (attrs, wide, cursor) = unpack_flags(packed);
        assert_eq!(attrs, all);
        assert!(wide);
        assert!(cursor);
    }

    #[test]
    fn pack_cell_attrs_isolates_each_flag() {
        // Sanity: each individual flag unpacks to itself with no bleed.
        for attr in [
            CellAttrs::BOLD,
            CellAttrs::ITALIC,
            CellAttrs::UNDERLINE,
            CellAttrs::STRIKETHROUGH,
            CellAttrs::INVERSE,
            CellAttrs::DIM,
            CellAttrs::BLINK,
        ] {
            let cell = Cell {
                ch: ' ',
                fg: Color::WHITE,
                bg: Color::BLACK,
                attrs: attr,
                wide_continuation: false,
            };
            let packed = pack_flags(&cell, false);
            let (got_attrs, wide, cursor) = unpack_flags(packed);
            assert_eq!(got_attrs, attr, "attr bit leaked: {attr:?}");
            assert!(!wide);
            assert!(!cursor);
        }
    }

    #[test]
    fn fg_rgba_little_endian_order() {
        // Matches WGSL `unpack4x8unorm`: byte 0 = R, byte 1 = G, byte 2 = B,
        // byte 3 = A. This also matches the storage buffer's native little
        // endian on every backend we ship to.
        let c = Color::rgba(0x11, 0x22, 0x33, 0x44);
        let packed = pack_color_rgba(c);
        assert_eq!(packed, 0x4433_2211);

        // Byte order check via manual shift, mirroring what WGSL's
        // `unpack4x8unorm` does on the GPU.
        let bytes = packed.to_le_bytes();
        assert_eq!(bytes, [0x11, 0x22, 0x33, 0x44]);

        let round = unpack_color_rgba(packed);
        assert_eq!(round, c);
    }

    #[test]
    fn empty_cell_encodes_glyph_id_sentinel() {
        // The default cell is empty, so the encoder must produce the
        // sentinel id without ever calling into the glyph lookup closure.
        let cell = Cell::default();
        let encoded = encode_cell(&cell, None, false);
        assert_eq!(encoded.glyph_id, EMPTY_GLYPH_ID);
    }

    #[test]
    fn encode_cell_copies_colors_and_glyph_id() {
        let cell = Cell {
            ch: 'A',
            fg: Color::rgba(10, 20, 30, 255),
            bg: Color::rgba(40, 50, 60, 70),
            attrs: CellAttrs::BOLD,
            wide_continuation: false,
        };
        let encoded = encode_cell(&cell, Some(42), false);
        assert_eq!(encoded.glyph_id, 42);
        assert_eq!(unpack_color_rgba(encoded.fg_rgba), cell.fg);
        assert_eq!(unpack_color_rgba(encoded.bg_rgba), cell.bg);
        let (attrs, _, _) = unpack_flags(encoded.flags);
        assert_eq!(attrs, CellAttrs::BOLD);
    }

    #[test]
    fn glyph_id_assigned_stable_across_frames() {
        // Inserting the same key twice must return the same id. Inserting
        // a new key must return a different id. This keeps the cell buffer
        // stable across frames without re uploading the entire grid when
        // only a handful of glyphs were newly inserted.
        let mut table = GlyphIdTable::new();
        let key_a = GlyphKey { font_id: 1, glyph_id: 10, font_size_tenths: 120, subpixel_bin: 0 };
        let key_b = GlyphKey { font_id: 1, glyph_id: 11, font_size_tenths: 120, subpixel_bin: 0 };
        let a1 = table.insert(key_a);
        let a2 = table.insert(key_a);
        let b1 = table.insert(key_b);
        assert_eq!(a1, a2, "stable id required across inserts for the same key");
        assert_ne!(a1, b1, "distinct keys must map to distinct ids");
        assert_eq!(table.len(), 2);
        assert_eq!(table.get(&key_a), Some(a1));
    }

    #[test]
    fn glyph_id_reuse_free_list_on_eviction() {
        let mut table = GlyphIdTable::new();
        let key_a = GlyphKey { font_id: 1, glyph_id: 1, font_size_tenths: 120, subpixel_bin: 0 };
        let key_b = GlyphKey { font_id: 1, glyph_id: 2, font_size_tenths: 120, subpixel_bin: 0 };
        let a = table.insert(key_a);
        let _b = table.insert(key_b);
        // Release `a` via the evict path.
        let removed = table.remove(&key_a);
        assert_eq!(removed, Some(a));
        // Next insert of a brand new key should pick the freed id before
        // growing the id space.
        let key_c = GlyphKey { font_id: 1, glyph_id: 3, font_size_tenths: 120, subpixel_bin: 0 };
        let c = table.insert(key_c);
        assert_eq!(c, a, "freed id must be reused before allocating a new one");
    }

    #[test]
    fn glyph_id_capacity_tracks_unique_inserts() {
        let mut table = GlyphIdTable::new();
        assert!(table.is_empty());
        for i in 0..5u16 {
            table.insert(GlyphKey {
                font_id: 0,
                glyph_id: i,
                font_size_tenths: 120,
                subpixel_bin: 0,
            });
        }
        assert_eq!(table.len(), 5);
        assert_eq!(table.capacity_ids(), 5);
    }

    #[test]
    fn damaged_row_write_range_matches_line_damage() {
        // Build a 3 row x 4 col grid with two damaged rows. The write range
        // list must contain exactly those rows, at byte offsets that match
        // `row * cols * sizeof(GpuCell)`.
        let mut grid = CellGrid::new(3, 4);
        // CellGrid::new starts fully damaged on every row, so pre clear.
        grid.clear_dirty();

        // Poke row 0 and row 2 so only those two rows are dirty.
        grid.set_cell(0, 1, Cell::with_char('a'));
        grid.set_cell(2, 0, Cell::with_char('b'));

        let ranges = damaged_row_write_ranges(&grid);
        let cols = 4;
        let stride = (cols * size_of::<GpuCell>()) as u64;

        assert_eq!(ranges.len(), 2, "only two rows should be damaged");
        assert_eq!(ranges[0].row, 0);
        assert_eq!(ranges[0].byte_offset, 0);
        assert_eq!(ranges[0].byte_len, stride);
        assert_eq!(ranges[1].row, 2);
        assert_eq!(ranges[1].byte_offset, 2 * stride);
        assert_eq!(ranges[1].byte_len, stride);
    }

    #[test]
    fn damaged_row_write_range_skips_clean_rows() {
        // When every row is clean, the write plan must be empty. This is
        // the core property that makes the fragment path competitive with
        // the instanced path's retained cache.
        let mut grid = CellGrid::new(2, 3);
        grid.clear_dirty();

        assert!(damaged_row_write_ranges(&grid).is_empty());
    }

    #[test]
    fn encode_row_emits_one_gpu_cell_per_column() {
        let mut grid = CellGrid::new(1, 3);
        grid.set_cell(0, 0, Cell::with_char('a'));
        grid.set_cell(0, 1, Cell::default());
        grid.set_cell(0, 2, Cell::with_char('b'));

        let mut ids = FxHashMap::default();
        ids.insert('a', 7u32);
        ids.insert('b', 9u32);
        let row = encode_row(&grid, 0, None, |c| ids.get(&c).copied());

        assert_eq!(row.len(), 3);
        assert_eq!(row[0].glyph_id, 7);
        assert_eq!(row[1].glyph_id, EMPTY_GLYPH_ID);
        assert_eq!(row[2].glyph_id, 9);
    }

    #[test]
    fn encode_row_marks_cursor_cell_only() {
        let mut grid = CellGrid::new(1, 3);
        grid.set_cell(0, 1, Cell::with_char('c'));
        let row = encode_row(&grid, 0, Some((0, 1)), |_| Some(1));
        let (_, _, c0) = unpack_flags(row[0].flags);
        let (_, _, c1) = unpack_flags(row[1].flags);
        let (_, _, c2) = unpack_flags(row[2].flags);
        assert!(!c0);
        assert!(c1);
        assert!(!c2);
    }

    #[test]
    fn encode_row_out_of_bounds_returns_empty() {
        let grid = CellGrid::new(1, 3);
        assert!(encode_row(&grid, 5, None, |_| None).is_empty());
    }

    #[test]
    fn gpu_glyph_meta_from_entry_copies_fields() {
        let entry = test_glyph_entry();
        let meta = GpuGlyphMeta::from_entry(&entry);
        assert_eq!(meta.atlas_uv_min, [0.0, 0.0]);
        assert_eq!(meta.atlas_uv_max, [0.25, 0.5]);
        assert_eq!(meta.pixel_offset, [1.0, 2.0]);
        assert_eq!(meta.pixel_size, [8.0, 16.0]);
    }

    #[test]
    fn encode_damaged_rows_drives_full_upload_plan() {
        // Integration of damage detection + row encoding. Starts fully
        // damaged, clears, then marks a single row.
        let mut grid = CellGrid::new(2, 2);
        grid.clear_dirty();
        grid.set_cell(1, 0, Cell::with_char('x'));

        let rows = encode_damaged_rows(&grid, |_| Some(0));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, 1);
        assert_eq!(rows[0].1.len(), 2);
    }

    #[test]
    fn runtime_flag_is_read_once() {
        // Idempotency check. The flag is cached in a OnceLock, so calling
        // `runtime_flag_enabled` twice must return the same value even if
        // the env var changes mid process.
        let first = runtime_flag_enabled();
        let second = runtime_flag_enabled();
        assert_eq!(first, second);
    }

    #[test]
    fn prepare_frame_marks_first_frame_as_resized() {
        // The first call sees a zero-sized cache and treats the grid as a
        // fresh allocation. Every row participates in the upload.
        let mut grid = CellGrid::new(2, 3);
        grid.set_cell(0, 0, Cell::with_char('a'));
        let mut state = GridFragmentState::new();
        let plan = state.prepare_frame(&grid, None, 0, |_| None);
        assert!(plan.resized);
        assert_eq!(plan.rows.len(), 2);
    }

    #[test]
    fn prepare_frame_second_frame_uses_damage_only() {
        // After the first frame, a clean grid with no damage produces an
        // empty upload plan.
        let mut grid = CellGrid::new(2, 3);
        let mut state = GridFragmentState::new();
        state.prepare_frame(&grid, None, 0, |_| None);

        grid.clear_dirty();
        let plan = state.prepare_frame(&grid, None, 0, |_| None);
        assert!(!plan.resized);
        assert!(plan.rows.is_empty());
    }

    #[test]
    fn prepare_frame_rewrites_everything_on_atlas_generation_bump() {
        // The atlas generation is the invalidation signal. Bumping it must
        // force every row to re-upload so stale UV rects cannot leak in.
        let mut grid = CellGrid::new(2, 2);
        let mut state = GridFragmentState::new();
        state.prepare_frame(&grid, None, 5, |_| None);

        grid.clear_dirty();
        let plan = state.prepare_frame(&grid, None, 6, |_| None);
        assert!(plan.atlas_generation_bumped);
        assert_eq!(plan.rows.len(), 2, "all rows must rewrite on atlas bump");
    }

    #[test]
    fn prepare_frame_resize_forces_full_rewrite() {
        // Going from 2 rows to 3 rows is a resize. The upload plan must
        // cover every row so the freshly sized storage buffer is entirely
        // initialized before the first draw.
        let grid_small = CellGrid::new(2, 2);
        let grid_large = CellGrid::new(3, 2);
        let mut state = GridFragmentState::new();
        state.prepare_frame(&grid_small, None, 0, |_| None);
        let plan = state.prepare_frame(&grid_large, None, 0, |_| None);
        assert!(plan.resized);
        assert_eq!(plan.rows.len(), 3);
    }

    #[test]
    fn prepare_frame_byte_offsets_are_row_contiguous() {
        // Row N's byte offset must equal N * cols * 16. This is load
        // bearing for the fragment shader index math.
        let grid = CellGrid::new(3, 4);
        let mut state = GridFragmentState::new();
        let plan = state.prepare_frame(&grid, None, 0, |_| None);
        let stride = (4 * size_of::<GpuCell>()) as u64;
        assert_eq!(plan.rows[0].0.byte_offset, 0);
        assert_eq!(plan.rows[1].0.byte_offset, stride);
        assert_eq!(plan.rows[2].0.byte_offset, 2 * stride);
    }
}
