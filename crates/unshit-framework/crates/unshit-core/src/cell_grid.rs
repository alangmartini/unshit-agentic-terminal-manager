//! Character grid rendering primitive for terminal emulators, hex editors,
//! code editors, and matrix displays.
//!
//! Each cell holds a character, foreground/background colors, and attribute
//! flags. The grid bypasses cosmic-text shaping entirely for performance,
//! looking up monospace glyphs directly from the atlas.

use std::sync::atomic::AtomicU32;

use crate::style::types::Color;
use bitflags::bitflags;

/// Global cell metrics published by the renderer. Application code reads
/// these to compute PTY column/row counts that match the renderer exactly.
static GLOBAL_CELL_W: AtomicU32 = AtomicU32::new(0);
static GLOBAL_CELL_H: AtomicU32 = AtomicU32::new(0);
/// Pending grid dimensions computed by the renderer. The renderer writes
/// these after measuring cell metrics and element size; the app reads and
/// clears them to resize the PTY without a timing gap.
static GLOBAL_PENDING_COLS: AtomicU32 = AtomicU32::new(0);
static GLOBAL_PENDING_ROWS: AtomicU32 = AtomicU32::new(0);
/// Whether the application window currently has OS focus. 1 = focused, 0 = not.
static GLOBAL_WINDOW_FOCUSED: AtomicU32 = AtomicU32::new(1);

// ---------------------------------------------------------------------------
// Cell attributes
// ---------------------------------------------------------------------------

bitflags! {
    /// Visual attributes for a single grid cell.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
    pub struct CellAttrs: u8 {
        const BOLD          = 0b0000_0001;
        const ITALIC        = 0b0000_0010;
        const UNDERLINE     = 0b0000_0100;
        const STRIKETHROUGH = 0b0000_1000;
        const INVERSE       = 0b0001_0000;
        const DIM           = 0b0010_0000;
        const BLINK         = 0b0100_0000;
    }
}

// ---------------------------------------------------------------------------
// ANSI color palette
// ---------------------------------------------------------------------------

/// Standard 16 ANSI colors (0..15).
pub const ANSI_16: [Color; 16] = [
    // Standard 8
    Color { r: 0, g: 0, b: 0, a: 255 },       // 0  Black
    Color { r: 170, g: 0, b: 0, a: 255 },     // 1  Red
    Color { r: 0, g: 170, b: 0, a: 255 },     // 2  Green
    Color { r: 170, g: 85, b: 0, a: 255 },    // 3  Yellow
    Color { r: 0, g: 0, b: 170, a: 255 },     // 4  Blue
    Color { r: 170, g: 0, b: 170, a: 255 },   // 5  Magenta
    Color { r: 0, g: 170, b: 170, a: 255 },   // 6  Cyan
    Color { r: 170, g: 170, b: 170, a: 255 }, // 7  White
    // Bright 8
    Color { r: 85, g: 85, b: 85, a: 255 },    // 8  Bright Black
    Color { r: 255, g: 85, b: 85, a: 255 },   // 9  Bright Red
    Color { r: 85, g: 255, b: 85, a: 255 },   // 10 Bright Green
    Color { r: 255, g: 255, b: 85, a: 255 },  // 11 Bright Yellow
    Color { r: 85, g: 85, b: 255, a: 255 },   // 12 Bright Blue
    Color { r: 255, g: 85, b: 255, a: 255 },  // 13 Bright Magenta
    Color { r: 85, g: 255, b: 255, a: 255 },  // 14 Bright Cyan
    Color { r: 255, g: 255, b: 255, a: 255 }, // 15 Bright White
];

/// Look up a color from the 256-color palette.
///
/// - 0..15   : ANSI 16 standard/bright colors
/// - 16..231 : 6x6x6 color cube
/// - 232..255: 24-step grayscale ramp
pub fn color_256(index: u8) -> Color {
    if index < 16 {
        return ANSI_16[index as usize];
    }
    if index < 232 {
        // 6x6x6 color cube: indices 16..231
        let idx = (index - 16) as u16;
        let b_idx = idx % 6;
        let g_idx = (idx / 6) % 6;
        let r_idx = idx / 36;
        let to_val = |i: u16| -> u8 {
            if i == 0 {
                0
            } else {
                (55 + 40 * i) as u8
            }
        };
        Color::rgb(to_val(r_idx), to_val(g_idx), to_val(b_idx))
    } else {
        // Grayscale ramp: indices 232..255
        let v = 8 + 10 * (index - 232) as u16;
        Color::rgb(v as u8, v as u8, v as u8)
    }
}

// ---------------------------------------------------------------------------
// Cell
// ---------------------------------------------------------------------------

/// A single cell in a character grid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cell {
    /// The character displayed in this cell. `'\0'` means empty.
    pub ch: char,
    /// Foreground color.
    pub fg: Color,
    /// Background color.
    pub bg: Color,
    /// Visual attributes (bold, italic, etc.).
    pub attrs: CellAttrs,
    /// If `true`, this cell is the continuation (right half) of a wide
    /// character that started in the previous column. The renderer skips
    /// glyph emission for continuation cells.
    pub wide_continuation: bool,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            ch: '\0',
            fg: Color::WHITE,
            bg: Color::TRANSPARENT,
            attrs: CellAttrs::empty(),
            wide_continuation: false,
        }
    }
}

impl Cell {
    /// Create a cell with a character using default colors and no attributes.
    pub fn with_char(ch: char) -> Self {
        Self { ch, ..Default::default() }
    }

    /// Returns `true` when the cell has no visible character.
    pub fn is_empty(&self) -> bool {
        self.ch == '\0' || self.ch == ' '
    }
}

// ---------------------------------------------------------------------------
// Line damage tracking
// ---------------------------------------------------------------------------

/// Damage state for a single row.
///
/// Mirrors Alacritty's `LineDamageBounds` and WezTerm's `changed_since(seqno)`
/// pattern. `first_dirty_col..=last_dirty_col` is inclusive on both ends and
/// invalid (no damage) when `first_dirty_col > last_dirty_col`.
///
/// The monotonic `seqno` is bumped on every cell write on that row. The
/// renderer checkpoints the last seqno it rendered and may skip a row when
/// the stored seqno matches the checkpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineDamage {
    /// First dirty column (inclusive). `u16::MAX` when the row is clean.
    pub first_dirty_col: u16,
    /// Last dirty column (inclusive). `0` when the row is clean (paired with
    /// `first_dirty_col == u16::MAX` to indicate clean state).
    pub last_dirty_col: u16,
    /// Monotonically increasing write counter for this row. The renderer
    /// compares this against its last-seen value to decide whether the row
    /// needs re-rendering even when `first_dirty_col..=last_dirty_col` was
    /// already processed by an earlier pass this frame.
    pub seqno: u64,
}

impl Default for LineDamage {
    fn default() -> Self {
        // Start clean: first > last means no damage. seqno 0 is the initial
        // value. Renderers that have never seen this line use 0 too, so the
        // first frame still triggers a draw via the seqno compare.
        Self { first_dirty_col: u16::MAX, last_dirty_col: 0, seqno: 0 }
    }
}

impl LineDamage {
    /// `true` when this row has no pending damage to paint.
    pub fn is_clean(&self) -> bool {
        self.first_dirty_col == u16::MAX
    }

    /// Expand the damaged column range to include `col` and bump the seqno.
    pub fn mark_col(&mut self, col: u16) {
        self.mark_range(col, col);
    }

    /// Expand the damaged column range to `[start, end]` (inclusive) and bump
    /// the seqno. Used by full-row operations (clear, scroll) where touching
    /// every column is cheaper than calling `mark_col` in a loop.
    pub fn mark_range(&mut self, start: u16, end: u16) {
        if start > end {
            return;
        }
        if self.is_clean() {
            self.first_dirty_col = start;
            self.last_dirty_col = end;
        } else {
            if start < self.first_dirty_col {
                self.first_dirty_col = start;
            }
            if end > self.last_dirty_col {
                self.last_dirty_col = end;
            }
        }
        self.seqno = self.seqno.saturating_add(1);
    }

    /// Reset the dirty column range, leaving the seqno untouched. Called by
    /// the renderer after it has emitted quads for this row.
    pub fn clear_cols(&mut self) {
        self.first_dirty_col = u16::MAX;
        self.last_dirty_col = 0;
    }
}

// ---------------------------------------------------------------------------
// Style runs (run-length batching by style)
// ---------------------------------------------------------------------------

/// Rendered style signature for a cell. Two adjacent cells with the same
/// `StyleKey` can share a shaped text run and a single merged background
/// quad. `fg` and `bg` already account for the `INVERSE` attribute.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StyleKey {
    pub fg: Color,
    pub bg: Color,
    pub attrs: CellAttrs,
}

/// A maximal run of cells on a single row that share the same `StyleKey`.
/// `end_col` is exclusive.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StyleRun {
    pub start_col: usize,
    pub end_col: usize,
    pub style: StyleKey,
}

impl StyleRun {
    /// Number of columns covered by the run.
    pub fn col_count(&self) -> usize {
        self.end_col.saturating_sub(self.start_col)
    }
}

// ---------------------------------------------------------------------------
// CellGrid
// ---------------------------------------------------------------------------

/// A fixed-width cell grid. Provides the backing store and public API for
/// terminal/editor-style character grids.
#[derive(Clone, Debug, PartialEq)]
pub struct CellGrid {
    rows: usize,
    cols: usize,
    cells: Vec<Cell>,
    /// Per-cell dirty bits. When a cell is modified via `set_cell`, its
    /// corresponding entry is set to `true`. The renderer reads and clears
    /// these to determine which cells need re-batching.
    dirty: Vec<bool>,
    /// Per-row damage summary (first/last dirty column + seqno). The renderer
    /// iterates this and skips clean lines entirely without touching cells.
    line_damage: Vec<LineDamage>,
    /// Stable identity of the logical line currently at each row. Moves with
    /// the content on scroll / shift operations so cache consumers keyed by
    /// line identity (see `LineQuadCache`) can replay unchanged rows after
    /// the rows rotate. Mirrors Kitty's `linebuf_index`, Ghostty's
    /// `PageList`, and WezTerm's stable `id` attached to `Line` appdata.
    line_ids: Vec<u64>,
    /// Monotonic counter. Every time a row becomes a fresh "new" logical
    /// line (initial grid, scroll discard, resize, DECALN, clear) the
    /// counter is bumped and the resulting id is assigned.
    next_line_id: u64,
    cursor_row: usize,
    cursor_col: usize,
    cursor_visible: bool,
    /// Cell width in physical pixels as computed by the renderer.
    measured_cell_w: f32,
    measured_cell_h: f32,
}

impl CellGrid {
    /// Maximum dirty-column index representable by a `LineDamage` when a row
    /// has `cols` columns. `cols` is clamped to fit a `u16`.
    #[inline]
    fn last_col_u16(cols: usize) -> u16 {
        cols.saturating_sub(1).min(u16::MAX as usize) as u16
    }

    fn mark_all_lines_fully_damaged(lines: &mut [LineDamage], cols: usize) {
        let last_col = Self::last_col_u16(cols);
        for ld in lines {
            ld.mark_range(0, last_col);
        }
    }

    /// Copy `count` rows of cells from `src_row..src_row + count` to
    /// `dst_row..dst_row + count` using `copy_within`.
    ///
    /// The stable `line_ids` that identify the logical line at each row
    /// are copied alongside the cells so the destination rows inherit the
    /// source rows' identities. Because the retained line quad cache is
    /// keyed on `(NodeId, line_id)` rather than `(NodeId, row_index)`, the
    /// cached payload for each shifted line remains valid at its new row
    /// index: the line moves, its cache entry moves with it.
    ///
    /// Per-row damage rotates with the content too: a clean source row
    /// produces a clean destination row because the line's content has
    /// not changed; only its row index has. This replaces PR #62 / #70's
    /// behavior of full-damaging every destination row, which was only
    /// needed when the cache key included the row index. Callers that
    /// blank the vacated source rows (scroll_down, IL, DL) must also
    /// reset those rows' line identity via `reset_line_identity` so the
    /// cache misses against the blanked content.
    ///
    /// This is the efficient primitive terminal-level scroll / insert-line /
    /// delete-line ops use to reposition a contiguous block of rows without
    /// touching `set_cell` per cell. Overlapping source and destination
    /// ranges are handled correctly because `copy_within` performs the
    /// overlap-safe shift.
    ///
    /// `dst_row`, `src_row`, and `count` are clamped so the copy stays
    /// within `0..self.rows`. A zero-count shift is a no-op.
    pub fn shift_rows(&mut self, dst_row: usize, src_row: usize, count: usize) {
        if count == 0 || self.rows == 0 || self.cols == 0 {
            return;
        }
        let max_count = self.rows.saturating_sub(dst_row.max(src_row));
        let count = count.min(max_count);
        if count == 0 {
            return;
        }
        if dst_row != src_row {
            let src_start = src_row * self.cols;
            let src_end = src_start + count * self.cols;
            let dst_start = dst_row * self.cols;
            self.cells.copy_within(src_start..src_end, dst_start);
            // Per-cell dirty flags: the shifted cells are now at the
            // destination indices. Mark the destination cells dirty so
            // checkpoint-based per-cell consumers re-render.
            let dst_cells_end = dst_start + count * self.cols;
            self.dirty[dst_start..dst_cells_end].fill(true);
            // Stable line identity follows the content. Copy the source
            // ids into the destination range so the line quad cache (keyed
            // on line_id) replays the shifted lines at their new row
            // indices without a cache miss.
            self.line_ids.copy_within(src_row..src_row + count, dst_row);
            // Rotate per-row damage alongside the content so each row's
            // damage entry continues to describe the cells currently at
            // that row.
            self.line_damage.copy_within(src_row..src_row + count, dst_row);
        }
    }

    /// Allocate the next monotonic `line_id` and bump the counter.
    #[inline]
    fn allocate_line_id(&mut self) -> u64 {
        let id = self.next_line_id;
        self.next_line_id = self.next_line_id.wrapping_add(1);
        id
    }

    /// Reset the stable identity of `row` to a fresh `line_id`. Called when
    /// the logical line at `row` is discarded wholesale (scroll vacates a
    /// row, `clear_row` blanks a row, grid-wide `clear`, DECALN). The line
    /// quad cache keyed on `(NodeId, line_id)` will miss for this row
    /// because the old cached entry belongs to a line identity that no
    /// longer occupies `row`.
    pub fn reset_line_identity(&mut self, row: usize) {
        if row >= self.rows {
            return;
        }
        let new_id = self.allocate_line_id();
        self.line_ids[row] = new_id;
    }

    /// Create a new grid filled with default (empty) cells.
    pub fn new(rows: usize, cols: usize) -> Self {
        let len = rows * cols;
        // Start fully damaged on both the per-cell and per-line trackers so
        // the first render pass paints every row.
        let mut line_damage = vec![LineDamage::default(); rows];
        Self::mark_all_lines_fully_damaged(&mut line_damage, cols);
        // Allocate a unique stable id for each initial row. Starting at 1
        // keeps 0 available as a "never assigned" sentinel if callers ever
        // need it.
        let mut next_line_id: u64 = 1;
        let line_ids: Vec<u64> = (0..rows)
            .map(|_| {
                let id = next_line_id;
                next_line_id = next_line_id.wrapping_add(1);
                id
            })
            .collect();
        Self {
            rows,
            cols,
            cells: vec![Cell::default(); len],
            dirty: vec![true; len],
            line_damage,
            line_ids,
            next_line_id,
            cursor_row: 0,
            cursor_col: 0,
            cursor_visible: true,
            measured_cell_w: 0.0,
            measured_cell_h: 0.0,
        }
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    pub fn cols(&self) -> usize {
        self.cols
    }

    /// Access the underlying cell slice (read-only).
    pub fn cells(&self) -> &[Cell] {
        &self.cells
    }

    /// Access the dirty-tracking slice (read-only).
    pub fn dirty_flags(&self) -> &[bool] {
        &self.dirty
    }

    /// Access the per-line damage slice (read-only). Length equals `rows()`.
    pub fn line_damage(&self) -> &[LineDamage] {
        &self.line_damage
    }

    /// Borrow a single row's damage entry (returns `None` when `row` is out
    /// of bounds).
    pub fn line_damage_for(&self, row: usize) -> Option<&LineDamage> {
        self.line_damage.get(row)
    }

    /// Read-only slice of stable line ids, one per row. The id identifies
    /// the logical line currently occupying `row`; it moves with the cells
    /// across scroll and shift operations so caches keyed on line identity
    /// survive row-index rotation.
    pub fn line_ids(&self) -> &[u64] {
        &self.line_ids
    }

    /// Return the stable id of the logical line at `row`, or `None` when
    /// `row` is out of bounds.
    #[inline]
    pub fn line_id(&self, row: usize) -> Option<u64> {
        self.line_ids.get(row).copied()
    }

    /// Collect maximal runs of adjacent cells on `row` that share the same
    /// rendered style (foreground color, background color, and attributes
    /// flags). Used by the renderer to group cells into a single
    /// `BatchedTextRun` so shaping and background quads can be emitted once
    /// per run instead of per cell.
    ///
    /// When the row has zero cols or is out of bounds an empty vector is
    /// returned.
    pub fn compute_style_runs(&self, row: usize) -> Vec<StyleRun> {
        self.compute_style_runs_in_range(row, 0, self.cols)
    }

    /// Same as [`CellGrid::compute_style_runs`] but limited to the half-open
    /// column range `[start_col, end_col)`. Runs never cross the provided
    /// range boundaries.
    pub fn compute_style_runs_in_range(
        &self,
        row: usize,
        start_col: usize,
        end_col: usize,
    ) -> Vec<StyleRun> {
        if row >= self.rows {
            return Vec::new();
        }
        let end_col = end_col.min(self.cols);
        if start_col >= end_col {
            return Vec::new();
        }

        let mut runs: Vec<StyleRun> = Vec::new();
        let row_base = row * self.cols;
        let mut cur: Option<StyleRun> = None;
        for col in start_col..end_col {
            let cell = &self.cells[row_base + col];
            let (fg, bg) = if cell.attrs.contains(CellAttrs::INVERSE) {
                (cell.bg, cell.fg)
            } else {
                (cell.fg, cell.bg)
            };

            let key = StyleKey { fg, bg, attrs: cell.attrs };

            match cur.as_mut() {
                Some(run) if run.style == key => {
                    // Wide continuation cells belong to the run of the wide
                    // primary cell. Extending the run's end column keeps the
                    // merged background rect covering both halves.
                    run.end_col = col + 1;
                }
                _ => {
                    if let Some(finished) = cur.take() {
                        runs.push(finished);
                    }
                    cur = Some(StyleRun { start_col: col, end_col: col + 1, style: key });
                }
            }
        }
        if let Some(finished) = cur.take() {
            runs.push(finished);
        }
        runs
    }

    /// Clear all dirty flags (called by the renderer after batching). Also
    /// clears per-line damaged column ranges; seqnos are preserved so
    /// renderers can checkpoint their last-seen value.
    pub fn clear_dirty(&mut self) {
        self.dirty.fill(false);
        for ld in &mut self.line_damage {
            ld.clear_cols();
        }
    }

    /// Debug helper: render a row range as plain text, substituting empty cells
    /// with spaces so logs can compare terminal/parser output across stages.
    pub fn debug_row_string(&self, row: usize, start_col: usize, len: usize) -> String {
        if row >= self.rows {
            return String::new();
        }
        (start_col..start_col.saturating_add(len).min(self.cols))
            .map(|col| self.get_cell(row, col).map(|cell| cell.ch).unwrap_or('\0'))
            .map(|ch| if ch == '\0' { ' ' } else { ch })
            .collect()
    }

    /// Debug helper: dump the first `rows` rows up to `cols` columns.
    pub fn debug_rows(&self, rows: usize, cols: usize) -> Vec<String> {
        (0..rows.min(self.rows)).map(|row| self.debug_row_string(row, 0, cols)).collect()
    }

    /// Returns `true` if any cell is marked dirty.
    pub fn has_dirty_cells(&self) -> bool {
        self.dirty.iter().any(|&d| d)
    }

    // -- Cursor API ----------------------------------------------------------

    pub fn set_cursor(&mut self, row: usize, col: usize) {
        self.cursor_row = row.min(self.rows.saturating_sub(1));
        self.cursor_col = col.min(self.cols.saturating_sub(1));
    }

    pub fn set_cursor_visible(&mut self, visible: bool) {
        self.cursor_visible = visible;
    }

    pub fn cursor_row(&self) -> usize {
        self.cursor_row
    }

    pub fn cursor_col(&self) -> usize {
        self.cursor_col
    }

    pub fn cursor_visible(&self) -> bool {
        self.cursor_visible
    }

    /// Store the cell dimensions computed by the renderer.
    pub fn set_measured_cell_size(&mut self, w: f32, h: f32) {
        self.measured_cell_w = w;
        self.measured_cell_h = h;
    }

    pub fn measured_cell_w(&self) -> f32 {
        self.measured_cell_w
    }

    pub fn measured_cell_h(&self) -> f32 {
        self.measured_cell_h
    }

    // -- Global cell metrics (set by renderer, read by app) -------------------

    /// Store the most recently computed cell dimensions in a global so
    /// application code (resize handlers) can read the exact same values
    /// the renderer used. Thread-safe via atomics.
    pub fn publish_cell_metrics(w: f32, h: f32) {
        GLOBAL_CELL_W.store(w.to_bits(), std::sync::atomic::Ordering::Relaxed);
        GLOBAL_CELL_H.store(h.to_bits(), std::sync::atomic::Ordering::Relaxed);
    }

    /// Read the last published cell width (0.0 if never set).
    pub fn global_cell_w() -> f32 {
        f32::from_bits(GLOBAL_CELL_W.load(std::sync::atomic::Ordering::Relaxed))
    }

    /// Read the last published cell height (0.0 if never set).
    pub fn global_cell_h() -> f32 {
        f32::from_bits(GLOBAL_CELL_H.load(std::sync::atomic::Ordering::Relaxed))
    }

    /// Update the global window focus state.
    pub fn set_window_focused(focused: bool) {
        GLOBAL_WINDOW_FOCUSED
            .store(if focused { 1 } else { 0 }, std::sync::atomic::Ordering::Relaxed);
    }

    /// Read whether the window is focused.
    pub fn is_window_focused() -> bool {
        GLOBAL_WINDOW_FOCUSED.load(std::sync::atomic::Ordering::Relaxed) == 1
    }

    // -- Pending resize (renderer -> app) ------------------------------------

    /// Called by the renderer when it computes new grid dimensions.
    /// The app polls via `take_pending_resize` to apply the resize.
    pub fn publish_pending_resize(cols: u16, rows: u16) {
        GLOBAL_PENDING_COLS.store(cols as u32, std::sync::atomic::Ordering::Relaxed);
        GLOBAL_PENDING_ROWS.store(rows as u32, std::sync::atomic::Ordering::Relaxed);
    }

    /// Read and clear the pending resize. Returns `(cols, rows)` or `None`
    /// when no resize is pending.
    pub fn take_pending_resize() -> Option<(u16, u16)> {
        let cols = GLOBAL_PENDING_COLS.swap(0, std::sync::atomic::Ordering::Relaxed);
        let rows = GLOBAL_PENDING_ROWS.swap(0, std::sync::atomic::Ordering::Relaxed);
        if cols > 0 && rows > 0 {
            Some((cols as u16, rows as u16))
        } else {
            None
        }
    }

    // -- Coordinate helpers --------------------------------------------------

    #[inline]
    fn idx(&self, row: usize, col: usize) -> Option<usize> {
        if row < self.rows && col < self.cols {
            Some(row * self.cols + col)
        } else {
            None
        }
    }

    // -- Public API ----------------------------------------------------------

    /// Write a cell at `(row, col)`. Marks it dirty for the renderer.
    /// Out-of-bounds writes are silently ignored.
    pub fn set_cell(&mut self, row: usize, col: usize, cell: Cell) {
        if let Some(i) = self.idx(row, col) {
            self.cells[i] = cell;
            self.dirty[i] = true;
            if let Some(ld) = self.line_damage.get_mut(row) {
                ld.mark_col(col.min(u16::MAX as usize) as u16);
            }
        }
    }

    /// Read the cell at `(row, col)`. Returns `None` if out of bounds.
    pub fn get_cell(&self, row: usize, col: usize) -> Option<&Cell> {
        self.idx(row, col).map(|i| &self.cells[i])
    }

    /// Place a wide (CJK) character at `(row, col)`. The character occupies
    /// two columns: the primary cell at `col` and a continuation cell at
    /// `col + 1`.
    pub fn set_wide_cell(&mut self, row: usize, col: usize, cell: Cell) {
        if col + 1 >= self.cols {
            return; // not enough room
        }
        if let Some(i) = self.idx(row, col) {
            let mut primary = cell;
            primary.wide_continuation = false;
            self.cells[i] = primary;
            self.dirty[i] = true;
        }
        if let Some(i) = self.idx(row, col + 1) {
            self.cells[i] = Cell {
                ch: '\0',
                fg: cell.fg,
                bg: cell.bg,
                attrs: cell.attrs,
                wide_continuation: true,
            };
            self.dirty[i] = true;
        }
        if let Some(ld) = self.line_damage.get_mut(row) {
            let start = col.min(u16::MAX as usize) as u16;
            let end = (col + 1).min(u16::MAX as usize) as u16;
            ld.mark_range(start, end);
        }
    }

    /// Clear every cell to the default (empty) state and mark all dirty.
    /// Every row gets a fresh `line_id` because the logical line at every
    /// row has been discarded.
    pub fn clear(&mut self) {
        self.cells.fill(Cell::default());
        self.dirty.fill(true);
        Self::mark_all_lines_fully_damaged(&mut self.line_damage, self.cols);
        // Every logical line is gone. Assign fresh identities so caches
        // keyed on `line_id` miss for every row and rebuild against the
        // blank content.
        for row in 0..self.rows {
            let id = self.allocate_line_id();
            self.line_ids[row] = id;
        }
    }

    /// Scroll the grid contents up by `n` rows. The bottom `n` rows are
    /// filled with default (empty) cells and receive fresh `line_id`s.
    /// The surviving lines (previously at rows `[n..rows]`) rotate into
    /// rows `[0..rows - n]` carrying their stable `line_id`s with them, so
    /// the retained line quad cache (keyed on `(NodeId, line_id)`) replays
    /// those lines at their new row indices without re-emission.
    ///
    /// Damage is marked on the bottom `n` rows only (their content
    /// actually changed). The shifted rows rotate their damage entries
    /// alongside their content so each row's damage continues to reflect
    /// the row's current data. This reverts PR #62's unconditional
    /// full-damage-every-row behavior; with line identity in the cache
    /// key the surviving lines no longer need a forced re-emit.
    pub fn scroll_up(&mut self, n: usize) {
        if n == 0 {
            return;
        }
        let n = n.min(self.rows);
        let shift = n * self.cols;
        let total = self.rows * self.cols;

        self.cells.copy_within(shift..total, 0);

        let clear_start = total - shift;
        self.cells[clear_start..].fill(Cell::default());

        self.dirty.fill(true);

        // Rotate line ids left by `n`: the top `rows - n` rows inherit the
        // line identities of the rows that rotated into them. `copy_within`
        // handles overlapping source/destination correctly.
        let kept = self.rows - n;
        if kept > 0 {
            self.line_ids.copy_within(n..self.rows, 0);
            // Rotate per-row damage alongside the content so each row's
            // damage entry continues to describe the cells currently at
            // that row. Surviving rows carry their pre-scroll damage
            // state (clean stays clean, partial stays partial).
            self.line_damage.copy_within(n..self.rows, 0);
        }
        // Assign fresh ids to the `n` newly empty bottom rows and mark
        // only those rows fully damaged.
        let last_col = Self::last_col_u16(self.cols);
        for row in kept..self.rows {
            let id = self.allocate_line_id();
            self.line_ids[row] = id;
            if let Some(ld) = self.line_damage.get_mut(row) {
                ld.mark_range(0, last_col);
            }
        }
    }

    /// Resize the grid. Existing content in the overlapping region is
    /// preserved, along with the stable `line_id`s of rows that survive
    /// the resize. New rows receive fresh `line_id`s. All cells are marked
    /// dirty after resize so renderers re-emit regardless of cache state.
    pub fn resize(&mut self, new_rows: usize, new_cols: usize) {
        if new_rows == self.rows && new_cols == self.cols {
            return;
        }

        let mut new_cells = vec![Cell::default(); new_rows * new_cols];

        let copy_rows = self.rows.min(new_rows);
        let copy_cols = self.cols.min(new_cols);
        for r in 0..copy_rows {
            let src_start = r * self.cols;
            let dst_start = r * new_cols;
            new_cells[dst_start..dst_start + copy_cols]
                .copy_from_slice(&self.cells[src_start..src_start + copy_cols]);
        }

        // Preserve line identity for rows that survive the resize. New rows
        // allocate fresh ids so the line quad cache misses for them and
        // rebuilds against the blank content.
        let mut new_line_ids: Vec<u64> = Vec::with_capacity(new_rows);
        for row in 0..new_rows {
            if row < copy_rows {
                new_line_ids.push(self.line_ids[row]);
            } else {
                new_line_ids.push(self.allocate_line_id());
            }
        }

        self.rows = new_rows;
        self.cols = new_cols;
        self.cells = new_cells;
        self.dirty = vec![true; new_rows * new_cols];
        // Rebuild `line_damage` sized to `new_rows` and mark every row fully
        // damaged with a fresh seqno so renderers re-render regardless of
        // their previous checkpoint.
        let mut new_line_damage = vec![LineDamage::default(); new_rows];
        Self::mark_all_lines_fully_damaged(&mut new_line_damage, new_cols);
        self.line_damage = new_line_damage;
        self.line_ids = new_line_ids;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_grid_allocates_correct_size() {
        let g = CellGrid::new(24, 80);
        assert_eq!(g.cells().len(), 24 * 80);
    }

    #[test]
    fn default_cell_is_empty() {
        let c = Cell::default();
        assert!(c.is_empty());
        assert_eq!(c.fg, Color::WHITE);
        assert_eq!(c.bg, Color::TRANSPARENT);
        assert_eq!(c.attrs, CellAttrs::empty());
    }

    #[test]
    fn color_256_ansi_range() {
        for i in 0..16u8 {
            assert_eq!(color_256(i), ANSI_16[i as usize]);
        }
    }

    #[test]
    fn color_256_grayscale_range() {
        let first = color_256(232);
        assert_eq!(first.r, 8);
        assert_eq!(first.g, 8);
        assert_eq!(first.b, 8);

        let last = color_256(255);
        assert_eq!(last.r, 238);
        assert_eq!(last.g, 238);
        assert_eq!(last.b, 238);
    }

    // -- Pending resize tests -----------------------------------------------

    #[test]
    fn take_pending_resize_returns_none_when_nothing_published() {
        // Clear any leftover state from other tests.
        GLOBAL_PENDING_COLS.store(0, std::sync::atomic::Ordering::Relaxed);
        GLOBAL_PENDING_ROWS.store(0, std::sync::atomic::Ordering::Relaxed);

        assert!(
            CellGrid::take_pending_resize().is_none(),
            "take_pending_resize should return None when no resize was published"
        );
    }

    #[test]
    fn publish_then_take_pending_resize_round_trips() {
        CellGrid::publish_pending_resize(120, 40);
        let result = CellGrid::take_pending_resize();
        assert_eq!(result, Some((120, 40)));
    }

    #[test]
    fn take_pending_resize_clears_after_read() {
        CellGrid::publish_pending_resize(80, 24);
        let first = CellGrid::take_pending_resize();
        assert_eq!(first, Some((80, 24)));

        // Second take should return None since the values were cleared.
        let second = CellGrid::take_pending_resize();
        assert!(second.is_none(), "pending resize should be cleared after take");
    }

    #[test]
    fn publish_pending_resize_overwrites_previous() {
        CellGrid::publish_pending_resize(80, 24);
        CellGrid::publish_pending_resize(120, 40);

        let result = CellGrid::take_pending_resize();
        assert_eq!(result, Some((120, 40)), "should return the most recent publish");
    }

    // -- Global cell metrics tests ------------------------------------------

    #[test]
    fn publish_and_read_global_cell_metrics() {
        CellGrid::publish_cell_metrics(9.5, 18.0);
        let w = CellGrid::global_cell_w();
        let h = CellGrid::global_cell_h();
        assert!((w - 9.5).abs() < f32::EPSILON);
        assert!((h - 18.0).abs() < f32::EPSILON);
    }

    // -- shift_rows ---------------------------------------------------------

    #[test]
    fn shift_rows_copies_cells_and_rotates_identity_and_damage() {
        // After issue #52 Step 3, shift_rows rotates content, line_ids, and
        // per-row damage together. A clean source row produces a clean
        // destination row because the logical line's content has not
        // changed, only the index it occupies. The retained line quad
        // cache (keyed on `line_id`) replays the line at its new index.
        let mut g = CellGrid::new(4, 3);
        for r in 0..4 {
            let ch = (b'A' + r as u8) as char;
            for c in 0..3 {
                g.set_cell(r, c, Cell::with_char(ch));
            }
        }
        g.clear_dirty();
        let ids_before: Vec<u64> = g.line_ids().to_vec();

        // Shift rows 0..2 to rows 1..3 (mirrors scroll_down(1) inside rows 0..3).
        g.shift_rows(1, 0, 2);

        // Destination rows 1 and 2 contain the original rows 0 and 1.
        assert_eq!(g.get_cell(1, 0).unwrap().ch, 'A');
        assert_eq!(g.get_cell(2, 0).unwrap().ch, 'B');

        // Stable identity rotates with content: rows 1 and 2 now carry
        // the ids that rows 0 and 1 had before the shift.
        assert_eq!(g.line_id(1), Some(ids_before[0]));
        assert_eq!(g.line_id(2), Some(ids_before[1]));

        // Damage rotates alongside content; source rows were clean so
        // destination rows are clean too.
        assert!(
            g.line_damage()[1].is_clean(),
            "row 1 must be clean because source row 0 was clean",
        );
        assert!(
            g.line_damage()[2].is_clean(),
            "row 2 must be clean because source row 1 was clean",
        );
        // Rows outside the destination stay clean.
        assert!(g.line_damage()[0].is_clean(), "row 0 must remain clean");
        assert!(g.line_damage()[3].is_clean(), "row 3 must remain clean");
    }

    #[test]
    fn shift_rows_upward_overlapping_shift_preserves_content() {
        let mut g = CellGrid::new(4, 3);
        for r in 0..4 {
            let ch = (b'A' + r as u8) as char;
            for c in 0..3 {
                g.set_cell(r, c, Cell::with_char(ch));
            }
        }

        // Shift rows 1..4 up into rows 0..3 (mirrors scroll_up(1)).
        g.shift_rows(0, 1, 3);

        assert_eq!(g.get_cell(0, 0).unwrap().ch, 'B');
        assert_eq!(g.get_cell(1, 0).unwrap().ch, 'C');
        assert_eq!(g.get_cell(2, 0).unwrap().ch, 'D');
    }

    #[test]
    fn shift_rows_zero_count_is_noop() {
        let mut g = CellGrid::new(3, 3);
        g.clear_dirty();

        g.shift_rows(1, 0, 0);

        assert!(
            g.line_damage().iter().all(|ld| ld.is_clean()),
            "zero-count shift must leave all rows clean",
        );
    }

    #[test]
    fn shift_rows_clamps_out_of_bounds_count() {
        let mut g = CellGrid::new(3, 3);
        // Populate row 1 with a dirty write so its damage is non-clean;
        // this tests that damage rotates correctly into row 0.
        g.set_cell(1, 1, Cell::with_char('X'));
        let row1_damage_before = g.line_damage()[1];

        // Count past self.rows is clamped.
        g.shift_rows(0, 1, 100);

        // rows 0..2 (2 rows) are the clamped destination; row 0 inherits
        // the damage that was at row 1 before the shift.
        assert_eq!(
            g.line_damage()[0],
            row1_damage_before,
            "row 0 must inherit row 1's pre-shift damage",
        );
    }
}
