//! VTE-based terminal emulator that drives a `CellGrid`.
//!
//! Parses ANSI escape sequences from PTY output using the `vte` crate (0.13)
//! and renders them into a `CellGrid` from the unshit framework. Supports
//! cursor movement, scrolling, text attributes (bold, italic, underline, etc.),
//! 256-color and true-color SGR, erase operations, and window title (OSC).

use unshit::core::cell_grid::{color_256, Cell, CellAttrs, CellGrid, ANSI_16};
use unshit::core::style::types::Color;
use vte::{Params, Perform};

pub mod keys;

/// Terminal emulator state.
///
/// Holds a `CellGrid` plus cursor position, saved cursor, current text
/// attributes, and the VTE parser. Feed PTY output through `process_bytes`
/// to update the grid.
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
        }
    }

    /// Feed raw bytes (from PTY output) through the VTE parser.
    ///
    /// The parser is temporarily moved out of `self` so that a `Performer`
    /// helper can borrow `&mut self` without conflicting with the parser's
    /// own `&mut self` requirement.
    pub fn process_bytes(&mut self, bytes: &[u8]) {
        let mut parser = std::mem::take(&mut self.parser);
        for &byte in bytes {
            let mut performer = Performer { terminal: self };
            parser.advance(&mut performer, byte);
        }
        self.parser = parser;
        // Sync cursor position to the grid so the renderer can draw it.
        self.grid.set_cursor(self.cursor_row, self.cursor_col);
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

    // -- helpers --------------------------------------------------------------

    /// Scroll the grid up by one line, clearing the new bottom row.
    fn scroll_up(&mut self) {
        self.grid.scroll_up(1);
        self.clear_row(self.rows.saturating_sub(1));
    }

    /// Scroll the grid down by one line. Moves all rows down and clears the
    /// top row.
    fn scroll_down(&mut self) {
        if self.rows == 0 {
            return;
        }
        // Shift rows down manually: copy row N-1 into row N, from bottom up.
        for row in (1..self.rows).rev() {
            for col in 0..self.cols {
                if let Some(cell) = self.grid.get_cell(row - 1, col).copied() {
                    self.grid.set_cell(row, col, cell);
                }
            }
        }
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
                    // 2 or 3: erase entire display (3 also clears scrollback, but
                    // we have none)
                    2 | 3 => {
                        t.clear_region(0, 0, t.rows.saturating_sub(1), t.cols.saturating_sub(1));
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
            'L' => {
                let n = p0();
                for _ in 0..n {
                    // Shift rows down starting from cursor row.
                    for row in (t.cursor_row + 1..t.rows).rev() {
                        for col in 0..t.cols {
                            if let Some(cell) = t.grid.get_cell(row - 1, col).copied() {
                                t.grid.set_cell(row, col, cell);
                            }
                        }
                    }
                    t.clear_row(t.cursor_row);
                }
            }
            // DL: Delete Lines
            'M' if intermediates.is_empty() => {
                let n = p0();
                for _ in 0..n {
                    // Shift rows up starting from cursor row.
                    for row in t.cursor_row..t.rows.saturating_sub(1) {
                        for col in 0..t.cols {
                            if let Some(cell) = t.grid.get_cell(row + 1, col).copied() {
                                t.grid.set_cell(row, col, cell);
                            }
                        }
                    }
                    t.clear_row(t.rows.saturating_sub(1));
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

#[cfg(test)]
mod tests {
    use super::*;

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

        // Each character should be placed at consecutive columns on row 0.
        let expected: Vec<(usize, char)> = "Windows".chars().enumerate().collect();

        for (col, expected_ch) in &expected {
            let cell = term.grid().get_cell(0, *col).expect("cell should exist");
            assert_eq!(
                cell.ch, *expected_ch,
                "column {} should contain '{}' but got '{}'",
                col, expected_ch, cell.ch,
            );
        }

        // Cursor should be right after the last character.
        let (row, col) = term.cursor_position();
        assert_eq!(row, 0);
        assert_eq!(col, 7, "cursor should be at column 7 after 7-char string");
    }

    /// Regression test: "Microsoft" must occupy consecutive columns.
    ///
    /// Same root cause as the "Windows" gap bug. "Microsoft" contains
    /// multiple narrow glyphs (i, c, r, o) that exposed the font family
    /// mismatch in the renderer.
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
        // 95 printable chars need 96 cols to avoid wrap (cursor sits at col 95).
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
}
