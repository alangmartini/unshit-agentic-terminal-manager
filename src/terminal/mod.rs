//! VTE-based terminal emulator that drives a `CellGrid`.
//!
//! Parses ANSI escape sequences from PTY output using the `vte` crate (0.13)
//! and renders them into a `CellGrid` from the unshit framework. Supports
//! cursor movement, scrolling, text attributes (bold, italic, underline, etc.),
//! 256-color and true-color SGR, erase operations, and window title (OSC).

use std::collections::VecDeque;

use unshit::core::cell_grid::{color_256, Cell, CellAttrs, CellGrid, ANSI_16};
use unshit::core::style::types::Color;
use unshit::core::trace::{append_terminal_trace_line, terminal_trace_enabled};
use vte::{Params, Perform};

pub mod keys;

/// Maximum number of scrollback lines retained per terminal.
const MAX_SCROLLBACK: usize = 10_000;

fn preview_bytes(bytes: &[u8], limit: usize) -> String {
    let mut preview = String::from_utf8_lossy(&bytes[..bytes.len().min(limit)]).into_owned();
    preview = preview
        .replace('\r', "\\r")
        .replace('\n', "\\n")
        .replace('\u{1b}', "\\x1b");
    if bytes.len() > limit {
        preview.push_str("...");
    }
    preview
}

/// Terminal emulator state.
///
/// Holds a `CellGrid` plus cursor position, saved cursor, current text
/// attributes, and the VTE parser. Feed PTY output through `process_bytes`
/// to update the grid.
///
/// The `scrollback` buffer stores lines that scroll off the top of the
/// visible grid. The user can browse history with `scroll_view_up` /
/// `scroll_view_down`; `display_grid` returns the composed view.
pub struct Terminal {
    grid: CellGrid,
    cursor_row: usize,
    cursor_col: usize,
    saved_cursor: (usize, usize),
    fg: Color,
    bg: Color,
    attrs: CellAttrs,
    parser: vte::Parser,
    rows: usize,
    cols: usize,
    title: String,
    /// Lines that scrolled off the top. Index 0 = oldest line.
    scrollback: VecDeque<Vec<Cell>>,
    /// How many lines the user has scrolled back (0 = at bottom / live).
    scroll_offset: usize,
}

/// Default foreground: warm amber.
const DEFAULT_FG: Color = Color {
    r: 212,
    g: 163,
    b: 72,
    a: 255,
};

/// Default background: fully transparent black.
const DEFAULT_BG: Color = Color {
    r: 0,
    g: 0,
    b: 0,
    a: 0,
};

impl Terminal {
    /// Create a new terminal emulator with the given dimensions.
    ///
    /// The default foreground is warm amber (212, 163, 72) and the default
    /// background is transparent.
    pub fn new(rows: usize, cols: usize) -> Self {
        Self {
            grid: CellGrid::new(rows, cols),
            cursor_row: 0,
            cursor_col: 0,
            saved_cursor: (0, 0),
            fg: DEFAULT_FG,
            bg: DEFAULT_BG,
            attrs: CellAttrs::empty(),
            parser: vte::Parser::new(),
            rows,
            cols,
            title: String::new(),
            scrollback: VecDeque::new(),
            scroll_offset: 0,
        }
    }

    /// Feed raw bytes (from PTY output) through the VTE parser.
    ///
    /// The parser is temporarily moved out of `self` so that a `Performer`
    /// helper can borrow `&mut self` without conflicting with the parser's
    /// own `&mut self` requirement.
    pub fn process_bytes(&mut self, bytes: &[u8]) {
        // New output from the PTY snaps the view back to the live screen.
        self.scroll_offset = 0;

        let mut parser = std::mem::take(&mut self.parser);
        for &byte in bytes {
            let mut performer = Performer { terminal: self };
            parser.advance(&mut performer, byte);
        }
        self.parser = parser;
        // Sync cursor position to the grid so the renderer can draw it.
        self.grid.set_cursor(self.cursor_row, self.cursor_col);

        if terminal_trace_enabled() && !bytes.is_empty() {
            let rows = self.grid.debug_rows(4, 96);
            append_terminal_trace_line(&format!(
                "terminal-trace stage=process_bytes bytes={} cursor=({}, {}) rows={} cols={} row0={:?} row1={:?} row2={:?} row3={:?}",
                preview_bytes(bytes, 120),
                self.cursor_row,
                self.cursor_col,
                self.rows,
                self.cols,
                rows.first().cloned().unwrap_or_default(),
                rows.get(1).cloned().unwrap_or_default(),
                rows.get(2).cloned().unwrap_or_default(),
                rows.get(3).cloned().unwrap_or_default(),
            ));
        }
    }

    /// Immutable reference to the backing cell grid.
    pub fn grid(&self) -> &CellGrid {
        &self.grid
    }

    /// Mutable reference to the backing cell grid.
    pub fn grid_mut(&mut self) -> &mut CellGrid {
        &mut self.grid
    }

    /// Resize the terminal and its grid to new dimensions.
    ///
    /// The cursor is clamped to stay within the new bounds.
    pub fn resize(&mut self, rows: usize, cols: usize) {
        self.rows = rows;
        self.cols = cols;
        self.grid.resize(rows, cols);
        // Clamp cursor to new bounds.
        if self.cursor_row >= rows {
            self.cursor_row = rows.saturating_sub(1);
        }
        if self.cursor_col >= cols {
            self.cursor_col = cols.saturating_sub(1);
        }
        self.grid.set_cursor(self.cursor_row, self.cursor_col);
    }

    /// The current window title (set via OSC 0 or OSC 2).
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Current cursor position as (row, col), both zero-indexed.
    pub fn cursor_position(&self) -> (usize, usize) {
        (self.cursor_row, self.cursor_col)
    }

    // -- scrollback API -------------------------------------------------------

    /// Number of lines stored in the scrollback buffer.
    pub fn scrollback_len(&self) -> usize {
        self.scrollback.len()
    }

    /// Current scroll offset (0 = at bottom / live view).
    pub fn scroll_offset(&self) -> usize {
        self.scroll_offset
    }

    /// Scroll the view backward (toward older history) by `n` lines.
    pub fn scroll_view_up(&mut self, n: usize) {
        let max = self.scrollback.len();
        self.scroll_offset = (self.scroll_offset + n).min(max);
    }

    /// Scroll the view forward (toward live screen) by `n` lines.
    pub fn scroll_view_down(&mut self, n: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(n);
    }

    /// Snap the view back to the live screen.
    pub fn reset_scroll(&mut self) {
        self.scroll_offset = 0;
    }

    /// Build a `CellGrid` representing what should be displayed.
    ///
    /// When `scroll_offset == 0` this is just a clone of the live grid.
    /// When scrolled back, the grid is composed from scrollback lines and
    /// the upper portion of the live screen, with the cursor hidden.
    pub fn display_grid(&self) -> CellGrid {
        if self.scroll_offset == 0 {
            let view = self.grid.clone();
            if terminal_trace_enabled() {
                let rows = view.debug_rows(4, 96);
                append_terminal_trace_line(&format!(
                    "terminal-trace stage=display_grid_live scroll_offset=0 cursor=({}, {}) visible={} row0={:?} row1={:?} row2={:?} row3={:?}",
                    self.cursor_row,
                    self.cursor_col,
                    self.grid.cursor_visible(),
                    rows.first().cloned().unwrap_or_default(),
                    rows.get(1).cloned().unwrap_or_default(),
                    rows.get(2).cloned().unwrap_or_default(),
                    rows.get(3).cloned().unwrap_or_default(),
                ));
            }
            return view;
        }

        let mut view = CellGrid::new(self.rows, self.cols);
        let sb_len = self.scrollback.len();

        for display_row in 0..self.rows {
            // Virtual line index into (scrollback ++ screen).
            // At offset 0 the view starts at sb_len (the screen top).
            // At offset N it starts N lines earlier.
            let virtual_line = sb_len.saturating_sub(self.scroll_offset) + display_row;

            if virtual_line < sb_len {
                // This row comes from scrollback.
                let sb_row = &self.scrollback[virtual_line];
                for col in 0..self.cols {
                    if let Some(cell) = sb_row.get(col) {
                        view.set_cell(display_row, col, *cell);
                    }
                    // If scrollback row is shorter (resize), Cell::default fills.
                }
            } else {
                // This row comes from the live screen.
                let screen_row = virtual_line - sb_len;
                if screen_row < self.rows {
                    for col in 0..self.cols {
                        if let Some(cell) = self.grid.get_cell(screen_row, col) {
                            view.set_cell(display_row, col, *cell);
                        }
                    }
                }
            }
        }

        // Hide cursor when scrolled back.
        view.set_cursor_visible(false);
        if terminal_trace_enabled() {
            let rows = view.debug_rows(4, 96);
            append_terminal_trace_line(&format!(
                "terminal-trace stage=display_grid_scrollback scroll_offset={} cursor=({}, {}) row0={:?} row1={:?} row2={:?} row3={:?}",
                self.scroll_offset,
                self.cursor_row,
                self.cursor_col,
                rows.first().cloned().unwrap_or_default(),
                rows.get(1).cloned().unwrap_or_default(),
                rows.get(2).cloned().unwrap_or_default(),
                rows.get(3).cloned().unwrap_or_default(),
            ));
        }
        view
    }

    // -- helpers --------------------------------------------------------------

    /// Scroll the grid up by one line. The top row is saved to the
    /// scrollback buffer before it is lost.
    fn scroll_up(&mut self) {
        // Capture the top row before it scrolls off.
        let mut row = Vec::with_capacity(self.cols);
        for col in 0..self.cols {
            row.push(self.grid.get_cell(0, col).copied().unwrap_or_default());
        }
        self.scrollback.push_back(row);
        if self.scrollback.len() > MAX_SCROLLBACK {
            self.scrollback.pop_front();
        }

        self.grid.scroll_up(1);
        self.clear_row(self.rows.saturating_sub(1));
    }

    /// Scroll the grid down by one line. Moves all rows down and clears the
    /// top row.
    ///
    /// Uses `CellGrid::shift_rows` (copy_within + mark destination rows
    /// fully damaged) to mirror the PR #62 `scroll_up` invariant: every
    /// affected row is marked fully damaged so the retained line quad cache
    /// (keyed by row index + content hash) re-emits against the post-shift
    /// row indices. `clear_row` handles the vacated top row.
    fn scroll_down(&mut self) {
        if self.rows == 0 {
            return;
        }
        // Shift rows 0..rows-1 down into rows 1..rows, then blank row 0.
        self.grid.shift_rows(1, 0, self.rows - 1);
        self.clear_row(0);
    }

    /// Fill an entire row with blank cells using the current background color.
    fn clear_row(&mut self, row: usize) {
        let blank = Cell {
            ch: ' ',
            fg: self.fg,
            bg: self.bg,
            attrs: CellAttrs::empty(),
            wide_continuation: false,
        };
        for col in 0..self.cols {
            self.grid.set_cell(row, col, blank);
        }
    }

    /// Clear a rectangular region (inclusive on all sides).
    fn clear_region(&mut self, start_row: usize, start_col: usize, end_row: usize, end_col: usize) {
        let blank = Cell {
            ch: ' ',
            fg: self.fg,
            bg: self.bg,
            attrs: CellAttrs::empty(),
            wide_continuation: false,
        };
        let er = end_row.min(self.rows.saturating_sub(1));
        let ec = end_col.min(self.cols.saturating_sub(1));
        for r in start_row..=er {
            for c in start_col..=ec {
                self.grid.set_cell(r, c, blank);
            }
        }
    }

    /// Write a character at the current cursor position with the current
    /// attributes, then advance the cursor.
    fn put_char(&mut self, c: char) {
        if self.rows == 0 || self.cols == 0 {
            return;
        }
        let cell = Cell {
            ch: c,
            fg: self.fg,
            bg: self.bg,
            attrs: self.attrs,
            wide_continuation: false,
        };
        self.grid.set_cell(self.cursor_row, self.cursor_col, cell);
        self.cursor_col += 1;
        // Line wrap.
        if self.cursor_col >= self.cols {
            self.cursor_col = 0;
            self.cursor_row += 1;
            if self.cursor_row >= self.rows {
                self.cursor_row = self.rows - 1;
                self.scroll_up();
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Default for vte::Parser so std::mem::take works
// ---------------------------------------------------------------------------

impl Default for Terminal {
    fn default() -> Self {
        Self::new(24, 80)
    }
}

// ---------------------------------------------------------------------------
// VTE Performer
// ---------------------------------------------------------------------------

/// Borrows `&mut Terminal` to implement `vte::Perform`.
///
/// `vte::Parser::advance` requires `&mut self` on both the parser and the
/// performer. Because `Terminal` owns the parser, we temporarily move the
/// parser out (see `process_bytes`) and hand a performer that borrows the
/// rest of `Terminal` to the parser.
struct Performer<'a> {
    terminal: &'a mut Terminal,
}

impl<'a> Perform for Performer<'a> {
    /// Printable character: write at cursor and advance.
    fn print(&mut self, c: char) {
        self.terminal.put_char(c);
    }

    /// C0/C1 control bytes.
    fn execute(&mut self, byte: u8) {
        let t = &mut *self.terminal;
        match byte {
            // Line Feed
            0x0A => {
                t.cursor_row += 1;
                if t.cursor_row >= t.rows {
                    t.cursor_row = t.rows.saturating_sub(1);
                    t.scroll_up();
                }
            }
            // Carriage Return
            0x0D => {
                t.cursor_col = 0;
            }
            // Horizontal Tab
            0x09 => {
                let next_tab = (t.cursor_col / 8 + 1) * 8;
                t.cursor_col = next_tab.min(t.cols.saturating_sub(1));
            }
            // Backspace
            0x08 => {
                t.cursor_col = t.cursor_col.saturating_sub(1);
            }
            // Bell: ignored
            0x07 => {}
            _ => {}
        }
    }

    /// CSI (Control Sequence Introducer) dispatch.
    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], _ignore: bool, action: char) {
        let t = &mut *self.terminal;

        // Collect the first subparam of each param into a flat Vec<u16> for
        // easy indexed access. This mirrors how most CSI params are single
        // values (no subparams).
        let pv: Vec<u16> = params.iter().map(|sub| sub[0]).collect();

        // Convenience: first param with a default value.
        let p = |idx: usize, default: u16| -> u16 {
            pv.get(idx).copied().unwrap_or(0).max(default) // treat 0 as default
        };
        let p0 = || p(0, 1) as usize;

        // Helper to extract a param or return 0 (not clamped to 1).
        let raw = |idx: usize| -> u16 { pv.get(idx).copied().unwrap_or(0) };

        match action {
            // -- Cursor movement -----------------------------------------------

            // CUU: Cursor Up
            'A' => {
                t.cursor_row = t.cursor_row.saturating_sub(p0());
            }
            // CUD: Cursor Down
            'B' => {
                t.cursor_row = (t.cursor_row + p0()).min(t.rows.saturating_sub(1));
            }
            // CUF: Cursor Forward
            'C' => {
                t.cursor_col = (t.cursor_col + p0()).min(t.cols.saturating_sub(1));
            }
            // CUB: Cursor Back
            'D' => {
                t.cursor_col = t.cursor_col.saturating_sub(p0());
            }
            // CUP / HVP: Set cursor position (1-based params).
            'H' | 'f' => {
                let row = p(0, 1) as usize;
                let col = p(1, 1) as usize;
                t.cursor_row = row.saturating_sub(1).min(t.rows.saturating_sub(1));
                t.cursor_col = col.saturating_sub(1).min(t.cols.saturating_sub(1));
            }
            // VPA: Vertical Position Absolute (1-based row).
            'd' => {
                let row = p0();
                t.cursor_row = row.saturating_sub(1).min(t.rows.saturating_sub(1));
            }
            // CHA: Cursor Character Absolute (1-based column).
            'G' => {
                let col = p0();
                t.cursor_col = col.saturating_sub(1).min(t.cols.saturating_sub(1));
            }

            // -- Erase operations ----------------------------------------------

            // ED: Erase in Display
            'J' => {
                let mode = raw(0);
                match mode {
                    // 0: erase from cursor to end of display
                    0 => {
                        t.clear_region(
                            t.cursor_row,
                            t.cursor_col,
                            t.cursor_row,
                            t.cols.saturating_sub(1),
                        );
                        if t.cursor_row + 1 < t.rows {
                            t.clear_region(
                                t.cursor_row + 1,
                                0,
                                t.rows - 1,
                                t.cols.saturating_sub(1),
                            );
                        }
                    }
                    // 1: erase from start of display to cursor
                    1 => {
                        if t.cursor_row > 0 {
                            t.clear_region(0, 0, t.cursor_row - 1, t.cols.saturating_sub(1));
                        }
                        t.clear_region(t.cursor_row, 0, t.cursor_row, t.cursor_col);
                    }
                    // 2: erase entire display
                    2 => {
                        t.clear_region(0, 0, t.rows.saturating_sub(1), t.cols.saturating_sub(1));
                    }
                    // 3: erase entire display AND scrollback buffer
                    3 => {
                        t.clear_region(0, 0, t.rows.saturating_sub(1), t.cols.saturating_sub(1));
                        t.scrollback.clear();
                        t.scroll_offset = 0;
                    }
                    _ => {}
                }
            }
            // EL: Erase in Line
            'K' => {
                let mode = raw(0);
                match mode {
                    // 0: erase from cursor to end of line
                    0 => {
                        t.clear_region(
                            t.cursor_row,
                            t.cursor_col,
                            t.cursor_row,
                            t.cols.saturating_sub(1),
                        );
                    }
                    // 1: erase from start of line to cursor
                    1 => {
                        t.clear_region(t.cursor_row, 0, t.cursor_row, t.cursor_col);
                    }
                    // 2: erase entire line
                    2 => {
                        t.clear_region(t.cursor_row, 0, t.cursor_row, t.cols.saturating_sub(1));
                    }
                    _ => {}
                }
            }

            // -- Line insert/delete --------------------------------------------

            // IL: Insert Lines
            //
            // Uses `CellGrid::shift_rows` to move rows at/below the cursor
            // down by `n`, then blanks the newly exposed rows. Mirrors the
            // PR #62 `scroll_up` invariant: every row at and below the
            // cursor is marked fully damaged so the retained line quad
            // cache re-emits against the post-shift row indices.
            'L' => {
                let n = p0();
                let cursor_row = t.cursor_row;
                let rows = t.rows;
                let n = n.min(rows.saturating_sub(cursor_row));
                if n > 0 {
                    let move_count = rows.saturating_sub(cursor_row + n);
                    if move_count > 0 {
                        t.grid.shift_rows(cursor_row + n, cursor_row, move_count);
                    }
                    for row in cursor_row..cursor_row + n {
                        t.clear_row(row);
                    }
                }
            }
            // DL: Delete Lines
            //
            // Uses `CellGrid::shift_rows` to move rows below the cursor up
            // by `n`, then blanks the newly exposed rows at the bottom.
            // Mirrors the PR #62 `scroll_up` invariant: every row at and
            // below the cursor is marked fully damaged so the retained line
            // quad cache re-emits against the post-shift row indices.
            'M' if intermediates.is_empty() => {
                let n = p0();
                let cursor_row = t.cursor_row;
                let rows = t.rows;
                let n = n.min(rows.saturating_sub(cursor_row));
                if n > 0 {
                    let move_count = rows.saturating_sub(cursor_row + n);
                    if move_count > 0 {
                        t.grid.shift_rows(cursor_row, cursor_row + n, move_count);
                    }
                    for row in rows.saturating_sub(n)..rows {
                        t.clear_row(row);
                    }
                }
            }

            // -- Scroll -------------------------------------------------------

            // SU: Scroll Up
            'S' => {
                let n = p0();
                for _ in 0..n {
                    t.scroll_up();
                }
            }
            // SD: Scroll Down
            'T' => {
                let n = p0();
                for _ in 0..n {
                    t.scroll_down();
                }
            }

            // -- Character insert/delete ---------------------------------------

            // ICH: Insert Characters (blank spaces at cursor, shifting right)
            '@' => {
                let n = p0().min(t.cols - t.cursor_col);
                // Shift characters right.
                for col in (t.cursor_col + n..t.cols).rev() {
                    if let Some(cell) = t.grid.get_cell(t.cursor_row, col - n).copied() {
                        t.grid.set_cell(t.cursor_row, col, cell);
                    }
                }
                // Insert blanks.
                let blank = Cell {
                    ch: ' ',
                    fg: t.fg,
                    bg: t.bg,
                    attrs: CellAttrs::empty(),
                    wide_continuation: false,
                };
                for col in t.cursor_col..t.cursor_col + n {
                    t.grid.set_cell(t.cursor_row, col, blank);
                }
            }
            // DCH: Delete Characters (shift left, blank at end)
            'P' => {
                let n = p0().min(t.cols - t.cursor_col);
                // Shift characters left.
                for col in t.cursor_col..t.cols.saturating_sub(n) {
                    if let Some(cell) = t.grid.get_cell(t.cursor_row, col + n).copied() {
                        t.grid.set_cell(t.cursor_row, col, cell);
                    }
                }
                // Blank the end.
                let blank = Cell {
                    ch: ' ',
                    fg: t.fg,
                    bg: t.bg,
                    attrs: CellAttrs::empty(),
                    wide_continuation: false,
                };
                for col in t.cols.saturating_sub(n)..t.cols {
                    t.grid.set_cell(t.cursor_row, col, blank);
                }
            }

            // -- SGR: Select Graphic Rendition ---------------------------------
            'm' => {
                handle_sgr(t, &pv);
            }

            // -- DECSTBM: Set Scrolling Region (stored but not enforced) -------
            'r' if intermediates.is_empty() => {
                // Intentionally ignored for now.
            }

            _ => {
                // Unrecognized CSI sequence: silently ignored.
            }
        }
    }

    /// Operating System Command dispatch.
    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        // OSC 0 and OSC 2 both set the window title.
        if params.len() >= 2 {
            let cmd = params[0];
            if cmd == b"0" || cmd == b"2" {
                if let Ok(title) = std::str::from_utf8(params[1]) {
                    self.terminal.title = title.to_string();
                }
            }
        }
    }

    /// ESC dispatch for standalone escape sequences.
    fn esc_dispatch(&mut self, _intermediates: &[u8], _ignore: bool, byte: u8) {
        let t = &mut *self.terminal;
        match byte {
            // DECSC: Save Cursor Position
            b'7' => {
                t.saved_cursor = (t.cursor_row, t.cursor_col);
            }
            // DECRC: Restore Cursor Position
            b'8' => {
                t.cursor_row = t.saved_cursor.0.min(t.rows.saturating_sub(1));
                t.cursor_col = t.saved_cursor.1.min(t.cols.saturating_sub(1));
            }
            // RI: Reverse Index (move cursor up; scroll down if at top)
            b'M' => {
                if t.cursor_row == 0 {
                    t.scroll_down();
                } else {
                    t.cursor_row -= 1;
                }
            }
            _ => {}
        }
    }

    // DCS hooks are not needed for basic terminal emulation.
    fn hook(&mut self, _params: &Params, _intermediates: &[u8], _ignore: bool, _action: char) {}
    fn put(&mut self, _byte: u8) {}
    fn unhook(&mut self) {}
}

// ---------------------------------------------------------------------------
// SGR (Select Graphic Rendition) handler
// ---------------------------------------------------------------------------

/// Process an SGR parameter list, updating foreground, background, and
/// attribute flags on the terminal.
fn handle_sgr(t: &mut Terminal, pv: &[u16]) {
    // SGR with no params is the same as SGR 0 (reset).
    if pv.is_empty() {
        reset_attrs(t);
        return;
    }

    let mut i = 0;
    while i < pv.len() {
        let code = pv[i];
        match code {
            // Reset all attributes.
            0 => reset_attrs(t),

            // Set attribute flags.
            1 => t.attrs |= CellAttrs::BOLD,
            2 => t.attrs |= CellAttrs::DIM,
            3 => t.attrs |= CellAttrs::ITALIC,
            4 => t.attrs |= CellAttrs::UNDERLINE,
            7 => t.attrs |= CellAttrs::INVERSE,
            9 => t.attrs |= CellAttrs::STRIKETHROUGH,

            // Unset attribute flags.
            22 => t.attrs &= !(CellAttrs::BOLD | CellAttrs::DIM),
            23 => t.attrs &= !CellAttrs::ITALIC,
            24 => t.attrs &= !CellAttrs::UNDERLINE,
            27 => t.attrs &= !CellAttrs::INVERSE,
            29 => t.attrs &= !CellAttrs::STRIKETHROUGH,

            // Standard foreground colors (30..37).
            30..=37 => {
                t.fg = ANSI_16[(code - 30) as usize];
            }
            // Extended foreground: 38;5;N (256-color) or 38;2;R;G;B (RGB).
            38 => {
                i += 1;
                if i < pv.len() {
                    match pv[i] {
                        5 => {
                            // 256-color
                            i += 1;
                            if i < pv.len() {
                                t.fg = color_256(pv[i] as u8);
                            }
                        }
                        2 => {
                            // True color RGB
                            if i + 3 < pv.len() {
                                let r = pv[i + 1] as u8;
                                let g = pv[i + 2] as u8;
                                let b = pv[i + 3] as u8;
                                t.fg = Color::rgb(r, g, b);
                                i += 3;
                            }
                        }
                        _ => {}
                    }
                }
            }
            // Default foreground.
            39 => {
                t.fg = DEFAULT_FG;
            }

            // Standard background colors (40..47).
            40..=47 => {
                t.bg = ANSI_16[(code - 40) as usize];
            }
            // Extended background: 48;5;N (256-color) or 48;2;R;G;B (RGB).
            48 => {
                i += 1;
                if i < pv.len() {
                    match pv[i] {
                        5 => {
                            // 256-color
                            i += 1;
                            if i < pv.len() {
                                t.bg = color_256(pv[i] as u8);
                            }
                        }
                        2 => {
                            // True color RGB
                            if i + 3 < pv.len() {
                                let r = pv[i + 1] as u8;
                                let g = pv[i + 2] as u8;
                                let b = pv[i + 3] as u8;
                                t.bg = Color::rgb(r, g, b);
                                i += 3;
                            }
                        }
                        _ => {}
                    }
                }
            }
            // Default background.
            49 => {
                t.bg = DEFAULT_BG;
            }

            // Bright foreground colors (90..97).
            90..=97 => {
                t.fg = ANSI_16[(code - 90 + 8) as usize];
            }
            // Bright background colors (100..107).
            100..=107 => {
                t.bg = ANSI_16[(code - 100 + 8) as usize];
            }

            _ => {
                // Unknown SGR code: silently skip.
            }
        }
        i += 1;
    }
}

/// Reset all text attributes and colors to their defaults.
fn reset_attrs(t: &mut Terminal) {
    t.fg = DEFAULT_FG;
    t.bg = DEFAULT_BG;
    t.attrs = CellAttrs::empty();
}

// ---------------------------------------------------------------------------
// Tests: character preservation and glyph verification
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: extract the text content of a terminal row as a trimmed string.
    fn row_text(term: &Terminal, row: usize) -> String {
        let mut s = String::new();
        for col in 0..term.cols {
            if let Some(cell) = term.grid.get_cell(row, col) {
                s.push(cell.ch);
            }
        }
        s.trim_end_matches([' ', '\0']).to_string()
    }

    /// Helper: extract the char stored in a cell at (row, col).
    fn cell_char(term: &Terminal, row: usize, col: usize) -> char {
        term.grid()
            .get_cell(row, col)
            .expect("cell should exist")
            .ch
    }

    /// Helper: read a run of characters from a row starting at `start_col`.
    fn read_row_str(term: &Terminal, row: usize, start_col: usize, len: usize) -> String {
        (start_col..start_col + len)
            .map(|col| cell_char(term, row, col))
            .collect()
    }

    // -- Construction ---------------------------------------------------------

    #[test]
    fn new_terminal_dimensions() {
        let t = Terminal::new(24, 80);
        assert_eq!(t.rows, 24);
        assert_eq!(t.cols, 80);
        assert_eq!(t.cursor_position(), (0, 0));
    }

    #[test]
    fn default_terminal_is_24x80() {
        let t = Terminal::default();
        assert_eq!(t.rows, 24);
        assert_eq!(t.cols, 80);
    }

    // -- Basic text output ----------------------------------------------------

    #[test]
    fn print_hello() {
        let mut t = Terminal::new(5, 20);
        t.process_bytes(b"Hello");
        assert_eq!(row_text(&t, 0), "Hello");
        assert_eq!(t.cursor_position(), (0, 5));
    }

    #[test]
    fn print_with_newline() {
        let mut t = Terminal::new(5, 20);
        t.process_bytes(b"line1\r\nline2");
        assert_eq!(row_text(&t, 0), "line1");
        assert_eq!(row_text(&t, 1), "line2");
    }

    #[test]
    fn line_wrap() {
        let mut t = Terminal::new(3, 5);
        t.process_bytes(b"abcdefgh");
        assert_eq!(row_text(&t, 0), "abcde");
        assert_eq!(row_text(&t, 1), "fgh");
        assert_eq!(t.cursor_position(), (1, 3));
    }

    #[test]
    fn scroll_on_overflow() {
        let mut t = Terminal::new(3, 10);
        t.process_bytes(b"line1\r\nline2\r\nline3\r\nline4");
        // line1 should have scrolled off
        assert_eq!(row_text(&t, 0), "line2");
        assert_eq!(row_text(&t, 1), "line3");
        assert_eq!(row_text(&t, 2), "line4");
    }

    // -- Control characters ---------------------------------------------------

    #[test]
    fn carriage_return() {
        let mut t = Terminal::new(3, 20);
        t.process_bytes(b"hello\rworld");
        assert_eq!(row_text(&t, 0), "world");
    }

    #[test]
    fn backspace() {
        let mut t = Terminal::new(3, 20);
        t.process_bytes(b"abc\x08X");
        assert_eq!(row_text(&t, 0), "abX");
    }

    #[test]
    fn tab_stops() {
        let mut t = Terminal::new(3, 40);
        t.process_bytes(b"a\tb");
        // Tab should move to column 8
        assert_eq!(t.cursor_position(), (0, 9)); // 'b' is at col 8, cursor at 9
        let cell = t.grid.get_cell(0, 8).unwrap();
        assert_eq!(cell.ch, 'b');
    }

    // -- Cursor movement (CSI) ------------------------------------------------

    #[test]
    fn cursor_up() {
        let mut t = Terminal::new(10, 10);
        t.process_bytes(b"\x1b[5;5H"); // move to row 5, col 5
        assert_eq!(t.cursor_position(), (4, 4));
        t.process_bytes(b"\x1b[2A"); // up 2
        assert_eq!(t.cursor_position(), (2, 4));
    }

    #[test]
    fn cursor_down() {
        let mut t = Terminal::new(10, 10);
        t.process_bytes(b"\x1b[1;1H"); // top left
        t.process_bytes(b"\x1b[3B"); // down 3
        assert_eq!(t.cursor_position(), (3, 0));
    }

    #[test]
    fn cursor_forward() {
        let mut t = Terminal::new(10, 10);
        t.process_bytes(b"\x1b[5C"); // forward 5
        assert_eq!(t.cursor_position(), (0, 5));
    }

    #[test]
    fn cursor_back() {
        let mut t = Terminal::new(10, 10);
        t.process_bytes(b"\x1b[1;8H"); // col 8
        t.process_bytes(b"\x1b[3D"); // back 3
        assert_eq!(t.cursor_position(), (0, 4));
    }

    #[test]
    fn cursor_position_absolute() {
        let mut t = Terminal::new(10, 20);
        t.process_bytes(b"\x1b[3;10H");
        assert_eq!(t.cursor_position(), (2, 9)); // 1-based to 0-based
    }

    #[test]
    fn cursor_clamps_to_bounds() {
        let mut t = Terminal::new(5, 10);
        t.process_bytes(b"\x1b[100;100H");
        assert_eq!(t.cursor_position(), (4, 9)); // clamped
    }

    #[test]
    fn cursor_cha() {
        let mut t = Terminal::new(5, 20);
        t.process_bytes(b"hello");
        t.process_bytes(b"\x1b[3G"); // CHA: move to column 3
        assert_eq!(t.cursor_position(), (0, 2));
    }

    #[test]
    fn cursor_vpa() {
        let mut t = Terminal::new(10, 10);
        t.process_bytes(b"\x1b[5d"); // VPA: move to row 5
        assert_eq!(t.cursor_position(), (4, 0));
    }

    // -- Erase operations -----------------------------------------------------

    #[test]
    fn erase_to_end_of_line() {
        let mut t = Terminal::new(3, 10);
        t.process_bytes(b"0123456789");
        t.process_bytes(b"\x1b[1;4H"); // move to col 4
        t.process_bytes(b"\x1b[0K"); // erase to end of line
        assert_eq!(row_text(&t, 0), "012");
    }

    #[test]
    fn erase_to_start_of_line() {
        let mut t = Terminal::new(3, 10);
        t.process_bytes(b"0123456789");
        t.process_bytes(b"\x1b[1;4H"); // move to col 4 (0-indexed: 3)
        t.process_bytes(b"\x1b[1K"); // erase from start to cursor
                                     // Cols 0..3 should be blank, 4..9 preserved
        let cell0 = t.grid.get_cell(0, 0).unwrap();
        assert_eq!(cell0.ch, ' ');
        let cell4 = t.grid.get_cell(0, 4).unwrap();
        assert_eq!(cell4.ch, '4');
    }

    #[test]
    fn erase_entire_line() {
        let mut t = Terminal::new(3, 10);
        t.process_bytes(b"0123456789");
        t.process_bytes(b"\x1b[1;5H");
        t.process_bytes(b"\x1b[2K"); // erase entire line
        assert_eq!(row_text(&t, 0), "");
    }

    #[test]
    fn erase_display_from_cursor() {
        let mut t = Terminal::new(3, 15);
        t.process_bytes(b"aaa\r\nbbb\r\nccc");
        // Cursor is now at row 2, col 3.
        // Move to row 2, col 2 (1-based: row 2, col 2).
        t.process_bytes(b"\x1b[2;2H"); // row 2, col 2 (0-indexed: row 1, col 1)
        t.process_bytes(b"\x1b[0J"); // erase from cursor to end
        assert_eq!(row_text(&t, 0), "aaa");
        // Row 1: col 0 = 'b', col 1 onward erased
        let cell = t.grid.get_cell(1, 0).unwrap();
        assert_eq!(cell.ch, 'b');
        let cell = t.grid.get_cell(1, 1).unwrap();
        assert_eq!(cell.ch, ' ');
        assert_eq!(row_text(&t, 2), "");
    }

    #[test]
    fn erase_entire_display() {
        let mut t = Terminal::new(3, 10);
        t.process_bytes(b"aaaaaaaaaa\r\nbbbbbbbbbb\r\ncccccccccc");
        t.process_bytes(b"\x1b[2J"); // erase entire display
        assert_eq!(row_text(&t, 0), "");
        assert_eq!(row_text(&t, 1), "");
        assert_eq!(row_text(&t, 2), "");
    }

    // -- Resize ---------------------------------------------------------------

    #[test]
    fn resize_clamps_cursor_from_large_position() {
        let mut t = Terminal::new(10, 20);
        t.process_bytes(b"\x1b[8;15H"); // row 8, col 15
        assert_eq!(t.cursor_position(), (7, 14));

        t.resize(5, 10);
        assert_eq!(t.rows, 5);
        assert_eq!(t.cols, 10);
        assert_eq!(t.cursor_position(), (4, 9)); // clamped
    }

    // -- SGR (text attributes) ------------------------------------------------

    #[test]
    fn sgr_bold() {
        let mut t = Terminal::new(3, 20);
        t.process_bytes(b"\x1b[1mBold\x1b[0m");
        let cell = t.grid.get_cell(0, 0).unwrap();
        assert!(cell.attrs.contains(CellAttrs::BOLD));
        assert_eq!(cell.ch, 'B');
    }

    #[test]
    fn sgr_italic_underline() {
        let mut t = Terminal::new(3, 20);
        t.process_bytes(b"\x1b[3;4mtext\x1b[0m");
        let cell = t.grid.get_cell(0, 0).unwrap();
        assert!(cell.attrs.contains(CellAttrs::ITALIC));
        assert!(cell.attrs.contains(CellAttrs::UNDERLINE));
    }

    #[test]
    fn sgr_reset() {
        let mut t = Terminal::new(3, 20);
        t.process_bytes(b"\x1b[1;3mX\x1b[0mY");
        let x = t.grid.get_cell(0, 0).unwrap();
        assert!(x.attrs.contains(CellAttrs::BOLD));
        let y = t.grid.get_cell(0, 1).unwrap();
        assert!(!y.attrs.contains(CellAttrs::BOLD));
        assert!(!y.attrs.contains(CellAttrs::ITALIC));
    }

    #[test]
    fn sgr_foreground_standard() {
        let mut t = Terminal::new(3, 20);
        t.process_bytes(b"\x1b[31mR"); // red foreground
        let cell = t.grid.get_cell(0, 0).unwrap();
        assert_eq!(cell.fg, ANSI_16[1]); // index 1 = red
    }

    #[test]
    fn sgr_background_standard() {
        let mut t = Terminal::new(3, 20);
        t.process_bytes(b"\x1b[42mG"); // green background
        let cell = t.grid.get_cell(0, 0).unwrap();
        assert_eq!(cell.bg, ANSI_16[2]); // index 2 = green
    }

    #[test]
    fn sgr_256_color() {
        let mut t = Terminal::new(3, 20);
        t.process_bytes(b"\x1b[38;5;196mR"); // 256-color fg
        let cell = t.grid.get_cell(0, 0).unwrap();
        assert_eq!(cell.fg, color_256(196));
    }

    #[test]
    fn sgr_truecolor() {
        let mut t = Terminal::new(3, 20);
        t.process_bytes(b"\x1b[38;2;100;150;200mX"); // RGB fg
        let cell = t.grid.get_cell(0, 0).unwrap();
        assert_eq!(cell.fg, Color::rgb(100, 150, 200));
    }

    #[test]
    fn sgr_default_fg_bg() {
        let mut t = Terminal::new(3, 20);
        t.process_bytes(b"\x1b[31;42m"); // set colors
        t.process_bytes(b"\x1b[39;49m"); // reset to default
        t.process_bytes(b"X");
        let cell = t.grid.get_cell(0, 0).unwrap();
        assert_eq!(cell.fg, DEFAULT_FG);
        assert_eq!(cell.bg, DEFAULT_BG);
    }

    #[test]
    fn sgr_bright_colors() {
        let mut t = Terminal::new(3, 20);
        t.process_bytes(b"\x1b[91mX"); // bright red fg
        let cell = t.grid.get_cell(0, 0).unwrap();
        assert_eq!(cell.fg, ANSI_16[9]); // bright red = 8+1

        t.process_bytes(b"\x1b[102mY"); // bright green bg
        let cell = t.grid.get_cell(0, 1).unwrap();
        assert_eq!(cell.bg, ANSI_16[10]); // bright green = 8+2
    }

    // -- ESC sequences --------------------------------------------------------

    #[test]
    fn save_restore_cursor() {
        let mut t = Terminal::new(10, 20);
        t.process_bytes(b"\x1b[3;5H"); // move to (2,4)
        t.process_bytes(b"\x1b7"); // save cursor
        t.process_bytes(b"\x1b[1;1H"); // move to (0,0)
        assert_eq!(t.cursor_position(), (0, 0));
        t.process_bytes(b"\x1b8"); // restore cursor
        assert_eq!(t.cursor_position(), (2, 4));
    }

    #[test]
    fn reverse_index_at_top_scrolls_down() {
        let mut t = Terminal::new(3, 10);
        t.process_bytes(b"line1\r\nline2\r\nline3");
        t.process_bytes(b"\x1b[1;1H"); // go to top
        t.process_bytes(b"\x1bM"); // reverse index
                                   // line1 should move to row 1, row 0 should be blank
        assert_eq!(row_text(&t, 0), "");
        assert_eq!(row_text(&t, 1), "line1");
    }

    #[test]
    fn reverse_index_not_at_top_just_moves_up() {
        let mut t = Terminal::new(5, 10);
        t.process_bytes(b"\x1b[3;1H"); // row 3
        t.process_bytes(b"\x1bM"); // reverse index
        assert_eq!(t.cursor_position(), (1, 0)); // moved up one
    }

    // -- OSC (window title) ---------------------------------------------------

    #[test]
    fn osc_sets_title() {
        let mut t = Terminal::new(3, 20);
        t.process_bytes(b"\x1b]0;My Terminal\x07");
        assert_eq!(t.title(), "My Terminal");
    }

    #[test]
    fn osc2_sets_title() {
        let mut t = Terminal::new(3, 20);
        t.process_bytes(b"\x1b]2;Another Title\x07");
        assert_eq!(t.title(), "Another Title");
    }

    // -- Insert/Delete characters ---------------------------------------------

    #[test]
    fn insert_characters() {
        let mut t = Terminal::new(3, 10);
        t.process_bytes(b"abcde");
        t.process_bytes(b"\x1b[1;3H"); // col 3 (0-indexed: 2)
        t.process_bytes(b"\x1b[2@"); // insert 2 blanks
        let cell_a = t.grid.get_cell(0, 0).unwrap();
        assert_eq!(cell_a.ch, 'a');
        let cell_b = t.grid.get_cell(0, 1).unwrap();
        assert_eq!(cell_b.ch, 'b');
        let cell_blank = t.grid.get_cell(0, 2).unwrap();
        assert_eq!(cell_blank.ch, ' ');
        let cell_c = t.grid.get_cell(0, 4).unwrap();
        assert_eq!(cell_c.ch, 'c');
    }

    #[test]
    fn delete_characters() {
        let mut t = Terminal::new(3, 10);
        t.process_bytes(b"abcde");
        t.process_bytes(b"\x1b[1;2H"); // col 2 (0-indexed: 1)
        t.process_bytes(b"\x1b[2P"); // delete 2 chars
        assert_eq!(row_text(&t, 0), "ade");
    }

    // -- Scroll operations ----------------------------------------------------

    #[test]
    fn scroll_up_csi() {
        let mut t = Terminal::new(3, 10);
        t.process_bytes(b"line1\r\nline2\r\nline3");
        t.process_bytes(b"\x1b[1S"); // scroll up 1
        assert_eq!(row_text(&t, 0), "line2");
        assert_eq!(row_text(&t, 1), "line3");
        assert_eq!(row_text(&t, 2), "");
    }

    #[test]
    fn scroll_down_csi() {
        let mut t = Terminal::new(3, 10);
        t.process_bytes(b"line1\r\nline2\r\nline3");
        t.process_bytes(b"\x1b[1T"); // scroll down 1
        assert_eq!(row_text(&t, 0), "");
        assert_eq!(row_text(&t, 1), "line1");
        assert_eq!(row_text(&t, 2), "line2");
    }

    // -- Insert/Delete lines --------------------------------------------------

    #[test]
    fn insert_lines() {
        let mut t = Terminal::new(4, 10);
        t.process_bytes(b"aaaa\r\nbbbb\r\ncccc\r\ndddd");
        t.process_bytes(b"\x1b[2;1H"); // row 2
        t.process_bytes(b"\x1b[1L"); // insert 1 line
        assert_eq!(row_text(&t, 0), "aaaa");
        assert_eq!(row_text(&t, 1), ""); // inserted blank
        assert_eq!(row_text(&t, 2), "bbbb");
        assert_eq!(row_text(&t, 3), "cccc");
    }

    #[test]
    fn delete_lines() {
        let mut t = Terminal::new(4, 10);
        t.process_bytes(b"aaaa\r\nbbbb\r\ncccc\r\ndddd");
        t.process_bytes(b"\x1b[2;1H"); // row 2
        t.process_bytes(b"\x1b[1M"); // delete 1 line
        assert_eq!(row_text(&t, 0), "aaaa");
        assert_eq!(row_text(&t, 1), "cccc");
        assert_eq!(row_text(&t, 2), "dddd");
        assert_eq!(row_text(&t, 3), ""); // blank bottom
    }

    // -- Edge cases -----------------------------------------------------------

    #[test]
    fn zero_size_terminal_does_not_panic() {
        let mut t = Terminal::new(0, 0);
        t.process_bytes(b"hello"); // should not panic
    }

    #[test]
    fn empty_sgr_resets() {
        let mut t = Terminal::new(3, 20);
        t.process_bytes(b"\x1b[1m"); // bold on
        t.process_bytes(b"\x1b[m"); // SGR with no params = reset
        t.process_bytes(b"X");
        let cell = t.grid.get_cell(0, 0).unwrap();
        assert!(!cell.attrs.contains(CellAttrs::BOLD));
    }

    // -- SGR attribute setting: DIM, INVERSE, STRIKETHROUGH -------------------

    #[test]
    fn sgr_dim() {
        let mut t = Terminal::new(3, 20);
        t.process_bytes(b"\x1b[2mD");
        let cell = t.grid.get_cell(0, 0).unwrap();
        assert!(cell.attrs.contains(CellAttrs::DIM));
    }

    #[test]
    fn sgr_inverse() {
        let mut t = Terminal::new(3, 20);
        t.process_bytes(b"\x1b[7mI");
        let cell = t.grid.get_cell(0, 0).unwrap();
        assert!(cell.attrs.contains(CellAttrs::INVERSE));
    }

    #[test]
    fn sgr_strikethrough() {
        let mut t = Terminal::new(3, 20);
        t.process_bytes(b"\x1b[9mS");
        let cell = t.grid.get_cell(0, 0).unwrap();
        assert!(cell.attrs.contains(CellAttrs::STRIKETHROUGH));
    }

    // -- SGR attribute unsetting: codes 22-29 ---------------------------------

    #[test]
    fn sgr_unbold_undim() {
        let mut t = Terminal::new(3, 20);
        t.process_bytes(b"\x1b[1;2m"); // bold + dim on
        assert!(t.attrs.contains(CellAttrs::BOLD));
        assert!(t.attrs.contains(CellAttrs::DIM));
        t.process_bytes(b"\x1b[22m"); // unbold/undim
        t.process_bytes(b"X");
        let cell = t.grid.get_cell(0, 0).unwrap();
        assert!(!cell.attrs.contains(CellAttrs::BOLD));
        assert!(!cell.attrs.contains(CellAttrs::DIM));
    }

    #[test]
    fn sgr_unitalic() {
        let mut t = Terminal::new(3, 20);
        t.process_bytes(b"\x1b[3m"); // italic on
        t.process_bytes(b"\x1b[23m"); // italic off
        t.process_bytes(b"X");
        let cell = t.grid.get_cell(0, 0).unwrap();
        assert!(!cell.attrs.contains(CellAttrs::ITALIC));
    }

    #[test]
    fn sgr_ununderline() {
        let mut t = Terminal::new(3, 20);
        t.process_bytes(b"\x1b[4m"); // underline on
        t.process_bytes(b"\x1b[24m"); // underline off
        t.process_bytes(b"X");
        let cell = t.grid.get_cell(0, 0).unwrap();
        assert!(!cell.attrs.contains(CellAttrs::UNDERLINE));
    }

    #[test]
    fn sgr_uninverse() {
        let mut t = Terminal::new(3, 20);
        t.process_bytes(b"\x1b[7m"); // inverse on
        t.process_bytes(b"\x1b[27m"); // inverse off
        t.process_bytes(b"X");
        let cell = t.grid.get_cell(0, 0).unwrap();
        assert!(!cell.attrs.contains(CellAttrs::INVERSE));
    }

    #[test]
    fn sgr_unstrikethrough() {
        let mut t = Terminal::new(3, 20);
        t.process_bytes(b"\x1b[9m"); // strikethrough on
        t.process_bytes(b"\x1b[29m"); // strikethrough off
        t.process_bytes(b"X");
        let cell = t.grid.get_cell(0, 0).unwrap();
        assert!(!cell.attrs.contains(CellAttrs::STRIKETHROUGH));
    }

    // -- SGR background extended colors ---------------------------------------

    #[test]
    fn sgr_bg_256_color() {
        let mut t = Terminal::new(3, 20);
        t.process_bytes(b"\x1b[48;5;82mX"); // 256-color bg
        let cell = t.grid.get_cell(0, 0).unwrap();
        assert_eq!(cell.bg, color_256(82));
    }

    #[test]
    fn sgr_bg_truecolor() {
        let mut t = Terminal::new(3, 20);
        t.process_bytes(b"\x1b[48;2;50;100;150mX"); // RGB bg
        let cell = t.grid.get_cell(0, 0).unwrap();
        assert_eq!(cell.bg, Color::rgb(50, 100, 150));
    }

    #[test]
    fn sgr_bg_bright_colors() {
        let mut t = Terminal::new(3, 20);
        t.process_bytes(b"\x1b[103mX"); // bright yellow bg (100 + 3)
        let cell = t.grid.get_cell(0, 0).unwrap();
        assert_eq!(cell.bg, ANSI_16[11]); // bright yellow = 8 + 3
    }

    // -- CSI 'f' (HVP) works like 'H' ----------------------------------------

    #[test]
    fn cursor_hvp_f() {
        let mut t = Terminal::new(10, 20);
        t.process_bytes(b"\x1b[4;12f"); // HVP: row 4, col 12
        assert_eq!(t.cursor_position(), (3, 11)); // 1-based to 0-based
    }

    // -- Scroll edge cases ----------------------------------------------------

    #[test]
    fn scroll_down_on_zero_size_terminal() {
        let mut t = Terminal::new(0, 0);
        t.scroll_down(); // should not panic
    }

    // -- Erase display mode 1 (erase from start to cursor) --------------------

    #[test]
    fn erase_display_to_cursor() {
        let mut t = Terminal::new(4, 20);
        t.process_bytes(b"aaaaaaaaaa\r\nbbbbbbbbbb\r\ncccccccccc\r\ndddddddddd");
        // Cursor after writing is at row 3, col 10.
        t.process_bytes(b"\x1b[2;5H"); // row 2, col 5 (0-indexed: 1, 4)
        t.process_bytes(b"\x1b[1J"); // erase from start of display to cursor
                                     // Row 0 should be fully erased
        assert_eq!(row_text(&t, 0), "");
        // Row 1: cols 0..4 erased, col 5 onward preserved
        let cell_erased = t.grid.get_cell(1, 0).unwrap();
        assert_eq!(cell_erased.ch, ' ');
        let cell_kept = t.grid.get_cell(1, 5).unwrap();
        assert_eq!(cell_kept.ch, 'b');
        // Row 2 and 3 should be untouched
        assert_eq!(row_text(&t, 2), "cccccccccc");
        assert_eq!(row_text(&t, 3), "dddddddddd");
    }

    // -- Erase display mode 3 (same as 2, entire display) ---------------------

    #[test]
    fn erase_display_mode_3() {
        let mut t = Terminal::new(3, 10);
        t.process_bytes(b"aaaaaaaaaa\r\nbbbbbbbbbb\r\ncccccccccc");
        t.process_bytes(b"\x1b[3J"); // erase entire display (with scrollback)
        assert_eq!(row_text(&t, 0), "");
        assert_eq!(row_text(&t, 1), "");
        assert_eq!(row_text(&t, 2), "");
    }

    // -- Basic terminal operations (PR #13) -----------------------------------

    #[test]
    fn basic_terminal_creation() {
        let term = Terminal::new(24, 80);
        assert_eq!(term.rows, 24);
        assert_eq!(term.cols, 80);
        assert_eq!(term.cursor_position(), (0, 0));
    }

    #[test]
    fn cursor_wraps_at_end_of_line() {
        let mut term = Terminal::new(2, 5);
        term.process_bytes(b"ABCDE");
        // After writing 5 chars in a 5-col terminal, cursor wraps to next row.
        assert_eq!(term.cursor_position(), (1, 0));
    }

    #[test]
    fn linefeed_advances_row() {
        let mut term = Terminal::new(24, 80);
        // LF (\n) only advances the row; it does NOT reset the column.
        // After "A" cursor is at (0,1). LF moves to (1,1). "B" prints at (1,1).
        term.process_bytes(b"A\nB");
        assert_eq!(term.cursor_position(), (1, 2));
        let cell_a = term.grid().get_cell(0, 0).unwrap();
        assert_eq!(cell_a.ch, 'A');
        let cell_b = term.grid().get_cell(1, 1).unwrap();
        assert_eq!(cell_b.ch, 'B');
    }

    #[test]
    fn carriage_return_resets_column() {
        let mut term = Terminal::new(24, 80);
        term.process_bytes(b"Hello\r");
        assert_eq!(term.cursor_position(), (0, 0));
    }

    #[test]
    fn resize_clamps_cursor() {
        let mut term = Terminal::new(24, 80);
        term.process_bytes(b"\x1b[20;70H"); // move cursor to row 19, col 69
        assert_eq!(term.cursor_position(), (19, 69));
        term.resize(10, 40);
        assert_eq!(term.cursor_position(), (9, 39));
    }

    // -- Character spacing regression tests (PR #13) --------------------------

    /// Regression test: "Windows" must occupy consecutive columns with no gaps.
    ///
    /// The renderer previously used `Attrs::new()` (SansSerif) for shaping
    /// individual characters but `Attrs::new().family(Monospace)` to measure
    /// cell width. Narrow sans-serif glyphs (i, l, r) rendered visually
    /// narrower than the measured cell_w, producing visible gaps like
    /// "Wi ndows" or "Mi crosoft". The fix ensures both shaping and
    /// measurement use the Monospace family.
    #[test]
    fn windows_string_occupies_consecutive_columns() {
        let mut term = Terminal::new(24, 80);
        term.process_bytes(b"Windows");

        let expected: Vec<(usize, char)> = "Windows".chars().enumerate().collect();
        for (col, expected_ch) in &expected {
            let cell = term.grid().get_cell(0, *col).expect("cell should exist");
            assert_eq!(
                cell.ch, *expected_ch,
                "column {} should contain '{}' but got '{}'",
                col, expected_ch, cell.ch,
            );
        }

        let (row, col) = term.cursor_position();
        assert_eq!(row, 0);
        assert_eq!(col, 7, "cursor should be at column 7 after 7-char string");
    }

    /// Regression test: "Microsoft" must occupy consecutive columns.
    #[test]
    fn microsoft_string_occupies_consecutive_columns() {
        let mut term = Terminal::new(24, 80);
        term.process_bytes(b"Microsoft");

        for (col, expected_ch) in "Microsoft".chars().enumerate() {
            let cell = term.grid().get_cell(0, col).expect("cell should exist");
            assert_eq!(
                cell.ch, expected_ch,
                "column {} should contain '{}' but got '{}'",
                col, expected_ch, cell.ch,
            );
        }

        let (row, col) = term.cursor_position();
        assert_eq!(row, 0);
        assert_eq!(col, 9);
    }

    /// Every printable ASCII character (0x20..=0x7E) must occupy exactly one
    /// cell and advance the cursor by one column.
    #[test]
    fn all_printable_ascii_occupy_one_cell_each() {
        let mut term = Terminal::new(24, 96);
        let printable: String = (0x20u8..=0x7Eu8).map(|b| b as char).collect();
        term.process_bytes(printable.as_bytes());

        for (col, expected_ch) in printable.chars().enumerate() {
            let cell = term.grid().get_cell(0, col).expect("cell should exist");
            assert_eq!(
                cell.ch, expected_ch,
                "column {} should contain {:?} (0x{:02X}) but got {:?}",
                col, expected_ch, expected_ch as u32, cell.ch,
            );
        }

        let (row, col) = term.cursor_position();
        assert_eq!(row, 0);
        assert_eq!(col, 95, "cursor should advance by one per printable char");
    }

    /// Narrow characters (i, l, r, t, f, j) must each advance the cursor by
    /// exactly one column, same as wide characters (M, W, m, w).
    #[test]
    fn narrow_and_wide_chars_advance_cursor_equally() {
        let narrow_chars = "ilrtfj";
        let wide_chars = "MWmw";

        for &ch in narrow_chars.as_bytes() {
            let mut term = Terminal::new(24, 80);
            term.process_bytes(&[ch]);
            let (_, col) = term.cursor_position();
            assert_eq!(
                col, 1,
                "narrow char '{}' should advance cursor to col 1, got {}",
                ch as char, col,
            );
        }

        for &ch in wide_chars.as_bytes() {
            let mut term = Terminal::new(24, 80);
            term.process_bytes(&[ch]);
            let (_, col) = term.cursor_position();
            assert_eq!(
                col, 1,
                "wide char '{}' should advance cursor to col 1, got {}",
                ch as char, col,
            );
        }
    }

    /// Mixed narrow and wide characters in a string must produce a
    /// contiguous sequence with no gaps.
    #[test]
    fn mixed_narrow_wide_string_no_gaps() {
        let mut term = Terminal::new(24, 80);
        let input = "File listing";
        term.process_bytes(input.as_bytes());

        for (col, expected_ch) in input.chars().enumerate() {
            let cell = term.grid().get_cell(0, col).expect("cell should exist");
            assert_eq!(
                cell.ch, expected_ch,
                "column {} should contain '{}' but got '{}'",
                col, expected_ch, cell.ch,
            );
        }

        let (_, col) = term.cursor_position();
        assert_eq!(col, input.len());
    }

    // -- Glyph verification tests (PR #15) ------------------------------------

    #[test]
    fn put_char_preserves_exact_character() {
        let mut term = Terminal::new(4, 80);
        let chars = ['i', 'l', '1', '|', ';', '!', 'I', 'O', '0'];
        for &c in &chars {
            term.cursor_row = 0;
            term.cursor_col = 0;
            term.put_char(c);
            let stored = cell_char(&term, 0, 0);
            assert_eq!(
                stored, c,
                "put_char({:?}) should store exactly {:?}, got {:?}",
                c, c, stored
            );
        }
    }

    #[test]
    fn all_ascii_printable_stored_correctly() {
        let mut term = Terminal::new(2, 96);
        let bytes: Vec<u8> = (0x20u8..=0x7E).collect();
        term.process_bytes(&bytes);

        for (i, byte) in (0x20u8..=0x7E).enumerate() {
            let expected = byte as char;
            let stored = cell_char(&term, 0, i);
            assert_eq!(
                stored, expected,
                "ASCII 0x{:02X} ({:?}) at col {} stored as {:?}",
                byte, expected, i, stored
            );
        }
    }

    #[test]
    fn visually_confusable_chars_are_distinct() {
        let confusable_pairs: &[(char, char)] = &[
            ('i', ';'),
            ('l', '!'),
            ('1', 'l'),
            ('|', '!'),
            ('O', '0'),
            ('I', 'l'),
        ];

        for &(a, b) in confusable_pairs {
            let mut term = Terminal::new(1, 80);
            term.process_bytes(&[a as u8]);
            let stored = cell_char(&term, 0, 0);
            assert_eq!(stored, a, "{:?} must not become {:?}", a, b);
            assert_ne!(stored, b, "{:?} must not become {:?}", a, b);
        }
    }

    #[test]
    fn word_corporation_stored_char_by_char() {
        let word = "Corporation";
        let mut term = Terminal::new(1, 80);
        term.process_bytes(word.as_bytes());

        for (col, expected) in word.chars().enumerate() {
            let stored = cell_char(&term, 0, col);
            assert_eq!(
                stored, expected,
                "\"Corporation\"[{}] should be {:?}, got {:?}",
                col, expected, stored
            );
        }
        assert_eq!(read_row_str(&term, 0, 0, word.len()), word);
    }

    #[test]
    fn word_windows_stored_char_by_char() {
        let word = "Windows";
        let mut term = Terminal::new(1, 80);
        term.process_bytes(word.as_bytes());

        for (col, expected) in word.chars().enumerate() {
            let stored = cell_char(&term, 0, col);
            assert_eq!(
                stored, expected,
                "\"Windows\"[{}] should be {:?}, got {:?}",
                col, expected, stored
            );
        }
        assert_eq!(read_row_str(&term, 0, 0, word.len()), word);
    }

    #[test]
    fn word_microsoft_stored_char_by_char() {
        let word = "Microsoft";
        let mut term = Terminal::new(1, 80);
        term.process_bytes(word.as_bytes());

        for (col, expected) in word.chars().enumerate() {
            let stored = cell_char(&term, 0, col);
            assert_eq!(
                stored, expected,
                "\"Microsoft\"[{}] should be {:?}, got {:?}",
                col, expected, stored
            );
        }
        assert_eq!(read_row_str(&term, 0, 0, word.len()), word);
    }

    #[test]
    fn multibyte_utf8_characters_stored_correctly() {
        let test_chars: &[char] = &[
            '\u{00E9}',  // e-acute (2 bytes)
            '\u{00F1}',  // n-tilde (2 bytes)
            '\u{00FC}',  // u-diaeresis (2 bytes)
            '\u{4E16}',  // CJK "world" (3 bytes)
            '\u{1F600}', // grinning face emoji (4 bytes)
        ];

        for &ch in test_chars {
            let mut term = Terminal::new(1, 80);
            let mut buf = [0u8; 4];
            let encoded = ch.encode_utf8(&mut buf);
            term.process_bytes(encoded.as_bytes());
            let stored = cell_char(&term, 0, 0);
            assert_eq!(
                stored, ch,
                "UTF-8 char U+{:04X} ({:?}) should be stored exactly, got {:?}",
                ch as u32, ch, stored
            );
        }
    }

    #[test]
    fn powershell_greeting_characters_preserved() {
        let greeting = concat!(
            "PowerShell 7.4.1\r\n",
            "Copyright (c) Microsoft Corporation.\r\n",
            "\r\n",
            "https://aka.ms/powershell\r\n",
            "Type 'help' to get help.\r\n",
        );

        let mut term = Terminal::new(24, 80);
        term.process_bytes(greeting.as_bytes());

        assert_eq!(read_row_str(&term, 0, 0, 16), "PowerShell 7.4.1");

        let row1 = read_row_str(&term, 1, 0, 36);
        assert_eq!(row1, "Copyright (c) Microsoft Corporation.");

        // 'i' in "Microsoft" at col 15 must NOT be ';'
        assert_eq!(cell_char(&term, 1, 15), 'i');
        // 'i' in "Corporation" at col 32 must NOT be ';'
        assert_eq!(cell_char(&term, 1, 32), 'i');

        assert_eq!(read_row_str(&term, 3, 0, 25), "https://aka.ms/powershell");
        assert_eq!(read_row_str(&term, 4, 0, 24), "Type 'help' to get help.");
    }

    #[test]
    fn powershell_greeting_with_ansi_escapes() {
        let ansi_greeting = concat!(
            "\x1b[0m",
            "\x1b[32mPowerShell 7.4.1",
            "\x1b[0m\r\n",
            "\x1b[90mCopyright (c) Microsoft Corporation.\x1b[0m\r\n",
            "\r\n",
            "\x1b[36mhttps://aka.ms/powershell\x1b[0m\r\n",
            "\x1b[90mType 'help' to get help.\x1b[0m\r\n",
        );

        let mut term = Terminal::new(24, 80);
        term.process_bytes(ansi_greeting.as_bytes());

        assert_eq!(read_row_str(&term, 0, 0, 16), "PowerShell 7.4.1");
        assert_eq!(
            read_row_str(&term, 1, 0, 36),
            "Copyright (c) Microsoft Corporation."
        );
        assert_eq!(cell_char(&term, 1, 15), 'i');
        assert_eq!(cell_char(&term, 1, 32), 'i');
    }

    // Regression: issue #17. Capital 'I' in "Instale" must be at col 0
    // with 'n' immediately at col 1. An ANSI parsing bug could consume
    // 'I' as a CSI action byte, inserting an empty cell.
    #[test]
    fn portuguese_greeting_instale_positions() {
        let input = "\x1b[33mInstale o PowerShell\x1b[0m";
        let mut term = Terminal::new(24, 80);
        term.process_bytes(input.as_bytes());

        assert_eq!(cell_char(&term, 0, 0), 'I', "col 0 must be 'I'");
        assert_eq!(cell_char(&term, 0, 1), 'n', "col 1 must be 'n'");
        assert_eq!(cell_char(&term, 0, 2), 's', "col 2 must be 's'");
        assert_eq!(read_row_str(&term, 0, 0, 20), "Instale o PowerShell");
    }

    // Regression: issue #17. Full Portuguese greeting pattern with
    // preceding lines and CR/LF. 'I' must land at col 0 of row 3.
    #[test]
    fn portuguese_greeting_with_preceding_lines() {
        let greeting = concat!(
            "\x1b[93mO Windows PowerShell\x1b[0m\r\n",
            "Copyright (C) Microsoft Corporation. Todos os direitos reservados.\r\n",
            "\r\n",
            "\x1b[33mInstale o PowerShell mais recente\x1b[0m\r\n",
        );
        let mut term = Terminal::new(24, 80);
        term.process_bytes(greeting.as_bytes());

        assert_eq!(cell_char(&term, 3, 0), 'I', "row 3 col 0 must be 'I'");
        assert_eq!(cell_char(&term, 3, 1), 'n', "row 3 col 1 must be 'n'");
        assert_eq!(read_row_str(&term, 3, 0, 8), "Instale ");
    }

    #[test]
    fn cursor_advances_consecutively() {
        let mut term = Terminal::new(1, 80);
        term.process_bytes(b"abcde");
        assert_eq!(term.cursor_position(), (0, 5));
        for (col, expected) in "abcde".chars().enumerate() {
            assert_eq!(cell_char(&term, 0, col), expected);
        }
    }

    #[test]
    fn line_wrap_preserves_characters() {
        let mut term = Terminal::new(4, 10);
        term.process_bytes(b"abcdefghijklmno");

        assert_eq!(read_row_str(&term, 0, 0, 10), "abcdefghij");
        assert_eq!(read_row_str(&term, 1, 0, 5), "klmno");
    }

    #[test]
    fn carriage_return_overwrites_preserve_chars() {
        let mut term = Terminal::new(1, 80);
        term.process_bytes(b"Hello\rHi!!");
        assert_eq!(cell_char(&term, 0, 0), 'H');
        assert_eq!(cell_char(&term, 0, 1), 'i');
        assert_eq!(cell_char(&term, 0, 2), '!');
        assert_eq!(cell_char(&term, 0, 3), '!');
        assert_eq!(cell_char(&term, 0, 4), 'o');
    }

    #[test]
    fn tab_does_not_corrupt_adjacent_cells() {
        let mut term = Terminal::new(1, 80);
        term.process_bytes(b"A\tB");
        assert_eq!(cell_char(&term, 0, 0), 'A');
        assert_eq!(cell_char(&term, 0, 8), 'B');
    }

    #[test]
    fn sentence_with_confusable_characters() {
        let sentence = "Bill filled 100 oil pills.";
        let mut term = Terminal::new(1, 80);
        term.process_bytes(sentence.as_bytes());

        for (col, expected) in sentence.chars().enumerate() {
            let stored = cell_char(&term, 0, col);
            assert_eq!(
                stored, expected,
                "sentence[{}] should be {:?}, got {:?}",
                col, expected, stored
            );
        }
    }

    #[test]
    fn process_bytes_is_deterministic() {
        let input = b"Microsoft Corporation 2024\r\nWindows PowerShell";
        let mut t1 = Terminal::new(24, 80);
        let mut t2 = Terminal::new(24, 80);
        t1.process_bytes(input);
        t2.process_bytes(input);

        for row in 0..2 {
            for col in 0..46 {
                let c1 = cell_char(&t1, row, col);
                let c2 = cell_char(&t2, row, col);
                assert_eq!(
                    c1, c2,
                    "determinism: ({},{}) differs between runs",
                    row, col
                );
            }
        }
    }

    // -- Scrollback buffer tests -----------------------------------------------

    /// Helper: fill a small terminal until it scrolls, returning the terminal.
    fn term_with_scrollback() -> Terminal {
        // 3-row, 5-col terminal. Write 5 lines to force 2 into scrollback.
        let mut term = Terminal::new(3, 5);
        term.process_bytes(b"AAAA\r\n");
        term.process_bytes(b"BBBB\r\n");
        term.process_bytes(b"CCCC\r\n");
        term.process_bytes(b"DDDD\r\n");
        term.process_bytes(b"EEEE");
        term
    }

    #[test]
    fn scroll_up_saves_top_row_to_scrollback() {
        let term = term_with_scrollback();
        // 5 lines in a 3-row terminal: first 2 lines scroll off.
        assert_eq!(term.scrollback_len(), 2, "expected 2 lines in scrollback");
    }

    #[test]
    fn scrollback_preserves_cell_content() {
        let term = term_with_scrollback();
        // The first line that scrolled off was "AAAA".
        let first_line = &term.scrollback[0];
        assert_eq!(first_line[0].ch, 'A', "first scrollback line should be 'A'");
        // The second line was "BBBB".
        let second_line = &term.scrollback[1];
        assert_eq!(
            second_line[0].ch, 'B',
            "second scrollback line should be 'B'"
        );
    }

    #[test]
    fn scrollback_max_limit_enforced() {
        let mut term = Terminal::new(2, 3);
        // Write MAX_SCROLLBACK + 10 lines to overflow the buffer.
        for i in 0..MAX_SCROLLBACK + 10 {
            let ch = (b'A' + (i % 26) as u8) as char;
            let line = format!("{}\r\n", ch);
            term.process_bytes(line.as_bytes());
        }
        assert!(
            term.scrollback_len() <= MAX_SCROLLBACK,
            "scrollback length {} exceeds MAX_SCROLLBACK {}",
            term.scrollback_len(),
            MAX_SCROLLBACK,
        );
    }

    #[test]
    fn scroll_offset_starts_at_zero() {
        let term = term_with_scrollback();
        assert_eq!(term.scroll_offset(), 0);
    }

    #[test]
    fn scroll_view_up_increases_offset() {
        let mut term = term_with_scrollback();
        term.scroll_view_up(1);
        assert_eq!(term.scroll_offset(), 1);
    }

    #[test]
    fn scroll_view_down_decreases_offset() {
        let mut term = term_with_scrollback();
        term.scroll_view_up(2);
        term.scroll_view_down(1);
        assert_eq!(term.scroll_offset(), 1);
    }

    #[test]
    fn scroll_view_down_clamps_at_zero() {
        let mut term = term_with_scrollback();
        term.scroll_view_up(1);
        term.scroll_view_down(100);
        assert_eq!(term.scroll_offset(), 0);
    }

    #[test]
    fn scroll_view_up_clamped_to_scrollback_len() {
        let mut term = term_with_scrollback();
        let max = term.scrollback_len();
        term.scroll_view_up(max + 100);
        assert_eq!(term.scroll_offset(), max);
    }

    #[test]
    fn process_bytes_resets_scroll_offset() {
        let mut term = term_with_scrollback();
        term.scroll_view_up(2);
        assert!(term.scroll_offset() > 0);
        term.process_bytes(b"X");
        assert_eq!(
            term.scroll_offset(),
            0,
            "new output should snap scroll to bottom"
        );
    }

    #[test]
    fn reset_scroll_snaps_to_bottom() {
        let mut term = term_with_scrollback();
        term.scroll_view_up(2);
        term.reset_scroll();
        assert_eq!(term.scroll_offset(), 0);
    }

    #[test]
    fn display_grid_at_bottom_matches_live_grid() {
        let term = term_with_scrollback();
        let live = term.grid().clone();
        let display = term.display_grid();
        for row in 0..term.rows {
            for col in 0..term.cols {
                let live_cell = live.get_cell(row, col).unwrap();
                let disp_cell = display.get_cell(row, col).unwrap();
                assert_eq!(
                    live_cell.ch, disp_cell.ch,
                    "display_grid at bottom should match live grid at ({},{})",
                    row, col
                );
            }
        }
    }

    #[test]
    fn display_grid_scrolled_shows_scrollback_content() {
        let mut term = term_with_scrollback();
        // Scroll all the way back (2 lines of scrollback).
        term.scroll_view_up(2);
        let display = term.display_grid();

        // Row 0 should show the first scrollback line (AAAA).
        let ch = display.get_cell(0, 0).unwrap().ch;
        assert_eq!(ch, 'A', "scrolled-back row 0 should be 'A', got '{}'", ch);

        // Row 1 should show the second scrollback line (BBBB).
        let ch = display.get_cell(1, 0).unwrap().ch;
        assert_eq!(ch, 'B', "scrolled-back row 1 should be 'B', got '{}'", ch);

        // Row 2 should show the first screen row (CCCC).
        let ch = display.get_cell(2, 0).unwrap().ch;
        assert_eq!(ch, 'C', "scrolled-back row 2 should be 'C', got '{}'", ch);
    }

    #[test]
    fn display_grid_hides_cursor_when_scrolled() {
        let mut term = term_with_scrollback();
        term.scroll_view_up(1);
        let display = term.display_grid();
        assert!(
            !display.cursor_visible(),
            "cursor should be hidden when scrolled back"
        );
    }

    #[test]
    fn display_grid_shows_cursor_at_bottom() {
        let term = term_with_scrollback();
        let display = term.display_grid();
        // At scroll_offset 0, display_grid is just a clone so cursor
        // visibility is whatever the live grid has.
        assert!(
            display.cursor_visible(),
            "cursor should be visible when at bottom"
        );
    }

    #[test]
    fn csi_ed_3_clears_scrollback() {
        let mut term = term_with_scrollback();
        assert!(term.scrollback_len() > 0);
        // CSI 3 J: erase display + clear scrollback.
        term.process_bytes(b"\x1b[3J");
        assert_eq!(
            term.scrollback_len(),
            0,
            "CSI 3 J should clear the scrollback buffer"
        );
    }

    // -- Line-damage regression tests (issue #63) -----------------------------
    //
    // After PR #62 patched CellGrid::scroll_up to full-damage every row, the
    // symmetric terminal-level ops that also shift which row an index points
    // to must do the same. Reference emulators (Alacritty, WezTerm, Kitty)
    // full-damage every affected row on scroll_down, insert-lines, and
    // delete-lines so the retained line quad cache rebuilds against the
    // post-shift row indices.

    /// Populate every cell on every row with distinct content, then clear
    /// damage so the starting line_damage state is fully clean.
    fn fill_and_clean_damage(term: &mut Terminal) {
        for r in 0..term.rows {
            let ch = (b'A' + (r as u8 % 26)) as char;
            for c in 0..term.cols {
                let cell = Cell {
                    ch,
                    fg: DEFAULT_FG,
                    bg: DEFAULT_BG,
                    attrs: CellAttrs::empty(),
                    wide_continuation: false,
                };
                term.grid.set_cell(r, c, cell);
            }
        }
        term.grid.clear_dirty();
        assert!(
            term.grid.line_damage().iter().all(|ld| ld.is_clean()),
            "precondition: every row must be clean after clear_dirty",
        );
    }

    fn assert_rows_fully_damaged(term: &Terminal, rows: std::ops::Range<usize>) {
        let last_col = term.cols.saturating_sub(1) as u16;
        for row in rows {
            let ld = term.grid.line_damage()[row];
            assert!(!ld.is_clean(), "row {row} must be damaged");
            assert_eq!(ld.first_dirty_col, 0, "row {row} first_dirty_col");
            assert_eq!(
                ld.last_dirty_col, last_col,
                "row {row} last_dirty_col must equal cols - 1",
            );
        }
    }

    #[test]
    fn scroll_down_marks_every_row_fully_damaged_so_line_cache_reemits() {
        // Regression: Terminal::scroll_down shifts content into every row
        // index. The retained line quad cache is keyed by row index and
        // content hash, so every row must be marked fully damaged and its
        // seqno bumped so the renderer re-emits and rebuilds the cache.
        let mut term = Terminal::new(4, 5);
        fill_and_clean_damage(&mut term);
        let seqs_before: Vec<u64> = term.grid.line_damage().iter().map(|ld| ld.seqno).collect();

        term.scroll_down();

        assert_rows_fully_damaged(&term, 0..term.rows);
        for (row, ld) in term.grid.line_damage().iter().enumerate() {
            assert!(
                ld.seqno > seqs_before[row],
                "row {row} seqno must advance after scroll_down",
            );
        }
    }

    #[test]
    fn insert_lines_marks_affected_rows_fully_damaged() {
        // Regression: CSI L (Insert Lines) shifts rows down starting at the
        // cursor. The cursor row and every row below must be marked fully
        // damaged, not merely per-column-dirtied with stale cache state.
        let mut term = Terminal::new(5, 6);
        // Cursor row 1: rows 1..=4 must be full-damaged.
        term.process_bytes(b"\x1b[2;1H"); // move to row 1, col 0 (1-indexed)
        fill_and_clean_damage(&mut term);
        let seqs_before: Vec<u64> = term.grid.line_damage().iter().map(|ld| ld.seqno).collect();

        term.process_bytes(b"\x1b[2L"); // Insert 2 blank lines

        let cursor_row = term.cursor_row;
        let rows = term.rows;
        assert_rows_fully_damaged(&term, cursor_row..rows);
        for (row, ld) in term
            .grid
            .line_damage()
            .iter()
            .enumerate()
            .take(rows)
            .skip(cursor_row)
        {
            assert!(
                ld.seqno > seqs_before[row],
                "row {row} seqno must advance after insert lines",
            );
        }
    }

    #[test]
    fn delete_lines_marks_affected_rows_fully_damaged() {
        // Regression: CSI M (Delete Lines) shifts rows up starting at the
        // cursor. The cursor row and every row below must be marked fully
        // damaged, not merely per-column-dirtied with stale cache state.
        let mut term = Terminal::new(5, 6);
        // Cursor row 1: rows 1..=4 must be full-damaged.
        term.process_bytes(b"\x1b[2;1H"); // move to row 1, col 0 (1-indexed)
        fill_and_clean_damage(&mut term);
        let seqs_before: Vec<u64> = term.grid.line_damage().iter().map(|ld| ld.seqno).collect();

        term.process_bytes(b"\x1b[2M"); // Delete 2 lines

        let cursor_row = term.cursor_row;
        let rows = term.rows;
        assert_rows_fully_damaged(&term, cursor_row..rows);
        for (row, ld) in term
            .grid
            .line_damage()
            .iter()
            .enumerate()
            .take(rows)
            .skip(cursor_row)
        {
            assert!(
                ld.seqno > seqs_before[row],
                "row {row} seqno must advance after delete lines",
            );
        }
    }

    #[test]
    fn reverse_index_at_top_marks_rows_fully_damaged() {
        // Regression: ESC M (Reverse Index) at the top of the screen
        // piggybacks on scroll_down, so it inherits the full-damage fix.
        let mut term = Terminal::new(4, 5);
        term.process_bytes(b"\x1b[1;1H"); // move cursor to row 0
        fill_and_clean_damage(&mut term);
        let seqs_before: Vec<u64> = term.grid.line_damage().iter().map(|ld| ld.seqno).collect();

        term.process_bytes(b"\x1bM"); // Reverse Index

        assert_rows_fully_damaged(&term, 0..term.rows);
        for (row, ld) in term.grid.line_damage().iter().enumerate() {
            assert!(
                ld.seqno > seqs_before[row],
                "row {row} seqno must advance after reverse index at top",
            );
        }
    }
}
