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
    cursor_row: usize,
    cursor_col: usize,
    cursor_visible: bool,
    /// Cell width in physical pixels as computed by the renderer.
    measured_cell_w: f32,
    measured_cell_h: f32,
}

impl CellGrid {
    /// Create a new grid filled with default (empty) cells.
    pub fn new(rows: usize, cols: usize) -> Self {
        let len = rows * cols;
        Self {
            rows,
            cols,
            cells: vec![Cell::default(); len],
            dirty: vec![true; len],
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

    /// Clear all dirty flags (called by the renderer after batching).
    pub fn clear_dirty(&mut self) {
        self.dirty.fill(false);
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
        GLOBAL_WINDOW_FOCUSED.store(
            if focused { 1 } else { 0 },
            std::sync::atomic::Ordering::Relaxed,
        );
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
    }

    /// Clear every cell to the default (empty) state and mark all dirty.
    pub fn clear(&mut self) {
        self.cells.fill(Cell::default());
        self.dirty.fill(true);
    }

    /// Scroll the grid contents up by `n` rows. The bottom `n` rows are
    /// filled with default (empty) cells. All affected cells are marked dirty.
    pub fn scroll_up(&mut self, n: usize) {
        if n == 0 {
            return;
        }
        let n = n.min(self.rows);
        let shift = n * self.cols;
        let total = self.rows * self.cols;

        // Move surviving rows up
        self.cells.copy_within(shift..total, 0);

        // Clear the bottom n rows
        let clear_start = total - shift;
        for cell in &mut self.cells[clear_start..] {
            *cell = Cell::default();
        }

        // Mark everything dirty (conservative; a smarter approach could track
        // only the moved/cleared region, but full-dirty is correct).
        self.dirty.fill(true);
    }

    /// Resize the grid. Existing content in the overlapping region is
    /// preserved; new cells are default-initialized. All cells are marked
    /// dirty after resize.
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

        self.rows = new_rows;
        self.cols = new_cols;
        self.cells = new_cells;
        self.dirty = vec![true; new_rows * new_cols];
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
}
