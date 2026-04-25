use vte::{Params, Perform};

use crate::cell::{Cell, CellAttrs};
use crate::color::{color_256, Color, ANSI_16};
use crate::grid::Grid;
use crate::scrollback::Scrollback;
use crate::snapshot::Snapshot;

const DEFAULT_FG: Color = Color::WHITE;
const DEFAULT_BG: Color = Color::TRANSPARENT;

pub struct Terminal {
    grid: Grid,
    scrollback: Scrollback,
    parser: vte::Parser,
    rows: usize,
    cols: usize,
    cursor_row: usize,
    cursor_col: usize,
    saved_cursor: (usize, usize),
    fg: Color,
    bg: Color,
    attrs: CellAttrs,
    title: String,
}

impl Terminal {
    pub fn new(rows: usize, cols: usize, max_scrollback: usize) -> Self {
        Self {
            grid: Grid::new(rows, cols),
            scrollback: Scrollback::new(max_scrollback),
            parser: vte::Parser::new(),
            rows,
            cols,
            cursor_row: 0,
            cursor_col: 0,
            saved_cursor: (0, 0),
            fg: DEFAULT_FG,
            bg: DEFAULT_BG,
            attrs: CellAttrs::empty(),
            title: String::new(),
        }
    }

    pub fn process_bytes(&mut self, bytes: &[u8]) {
        let mut parser = std::mem::take(&mut self.parser);
        for &byte in bytes {
            let mut performer = Performer { terminal: self };
            parser.advance(&mut performer, byte);
        }
        self.parser = parser;
        self.grid.set_cursor(self.cursor_row, self.cursor_col);
    }

    /// Bottom-anchored reflow on row resize (issue #129). Growing rows
    /// lifts scrollback into the new top so the cursor stays anchored to
    /// its distance-from-bottom, rather than leaving a blank gap below
    /// the live prompt. Shrinking rows pushes the rows above the cursor
    /// into scrollback so they survive the resize. Column-only resizes
    /// do not touch scrollback.
    pub fn resize(&mut self, rows: usize, cols: usize) {
        let old_rows = self.rows;

        if rows > old_rows {
            let k = rows - old_rows;
            let lifted = self.scrollback.pop_back_n(k);
            self.grid.grow_rows_at_top(k, lifted);
            self.cursor_row += k;
        } else if rows < old_rows {
            let k = old_rows - rows;
            // Only evict rows that sit above the cursor, so the live
            // prompt row is never pushed into scrollback.
            let evict_above = k.min(self.cursor_row);
            if evict_above > 0 {
                let evicted = self.grid.shrink_rows_from_top(evict_above);
                for line in evicted {
                    self.scrollback.push(line);
                }
                self.cursor_row -= evict_above;
            }
            // Any remaining shrink (k > cursor_row) trims the blank tail
            // below the cursor; grid.resize handles it by clipping.
        }

        self.grid.resize(rows, cols);
        self.rows = rows;
        self.cols = cols;

        self.cursor_row = self.cursor_row.min(rows.saturating_sub(1));
        self.cursor_col = self.cursor_col.min(cols.saturating_sub(1));
        self.grid.set_cursor(self.cursor_row, self.cursor_col);
    }

    pub fn grid(&self) -> &Grid {
        &self.grid
    }

    pub fn grid_mut(&mut self) -> &mut Grid {
        &mut self.grid
    }

    pub fn scrollback(&self) -> &Scrollback {
        &self.scrollback
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn snapshot(&self, scrollback_lines: usize) -> Snapshot {
        Snapshot {
            grid: self.grid.clone(),
            scrollback: self.scrollback.tail(scrollback_lines),
        }
    }

    fn scroll_up_and_capture(&mut self) {
        let evicted = self.grid.scroll_up();
        if !evicted.is_empty() {
            self.scrollback.push(evicted);
        }
    }

    fn clear_region(&mut self, start_row: usize, start_col: usize, end_row: usize, end_col: usize) {
        let blank = Cell {
            ch: ' ',
            fg: self.fg,
            bg: self.bg,
            attrs: CellAttrs::empty(),
        };
        let er = end_row.min(self.rows.saturating_sub(1));
        let ec = end_col.min(self.cols.saturating_sub(1));
        for r in start_row..=er {
            for c in start_col..=ec {
                self.grid.set(r, c, blank);
            }
        }
    }

    fn put_char(&mut self, c: char) {
        if self.rows == 0 || self.cols == 0 {
            return;
        }
        let cell = Cell {
            ch: c,
            fg: self.fg,
            bg: self.bg,
            attrs: self.attrs,
        };
        self.grid.set(self.cursor_row, self.cursor_col, cell);
        self.cursor_col += 1;
        if self.cursor_col >= self.cols {
            self.cursor_col = 0;
            self.cursor_row += 1;
            if self.cursor_row >= self.rows {
                self.cursor_row = self.rows - 1;
                self.scroll_up_and_capture();
            }
        }
    }
}

impl Default for Terminal {
    fn default() -> Self {
        Self::new(24, 80, 10_000)
    }
}

struct Performer<'a> {
    terminal: &'a mut Terminal,
}

impl Perform for Performer<'_> {
    fn print(&mut self, c: char) {
        self.terminal.put_char(c);
    }

    fn execute(&mut self, byte: u8) {
        let t = &mut *self.terminal;
        match byte {
            0x0A => {
                t.cursor_row += 1;
                if t.cursor_row >= t.rows {
                    t.cursor_row = t.rows.saturating_sub(1);
                    t.scroll_up_and_capture();
                }
            }
            0x0D => {
                t.cursor_col = 0;
            }
            0x09 => {
                let next_tab = (t.cursor_col / 8 + 1) * 8;
                t.cursor_col = next_tab.min(t.cols.saturating_sub(1));
            }
            0x08 => {
                t.cursor_col = t.cursor_col.saturating_sub(1);
            }
            0x07 => {}
            _ => {}
        }
    }

    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], _ignore: bool, action: char) {
        let t = &mut *self.terminal;
        let pv: Vec<u16> = params.iter().map(|sub| sub[0]).collect();
        let p = |idx: usize, default: u16| -> u16 {
            let v = pv.get(idx).copied().unwrap_or(0);
            if v == 0 {
                default
            } else {
                v
            }
        };
        let p0 = || p(0, 1) as usize;
        let raw = |idx: usize| -> u16 { pv.get(idx).copied().unwrap_or(0) };

        // Private modes: `\x1b[?25h/l` etc. arrive with intermediate byte `?`.
        if intermediates == b"?" {
            match action {
                'h' | 'l' => {
                    let on = action == 'h';
                    for code in &pv {
                        if *code == 25 {
                            t.grid.set_cursor_visible(on);
                        }
                    }
                }
                _ => {}
            }
            return;
        }

        // DECSCUSR: `\x1b[<n> q`. The intermediate is a space.
        if intermediates == b" " && action == 'q' {
            return;
        }

        match action {
            'A' => {
                t.cursor_row = t.cursor_row.saturating_sub(p0());
            }
            'B' => {
                t.cursor_row = (t.cursor_row + p0()).min(t.rows.saturating_sub(1));
            }
            'C' => {
                t.cursor_col = (t.cursor_col + p0()).min(t.cols.saturating_sub(1));
            }
            'D' => {
                t.cursor_col = t.cursor_col.saturating_sub(p0());
            }
            'H' | 'f' => {
                let row = p(0, 1) as usize;
                let col = p(1, 1) as usize;
                t.cursor_row = row.saturating_sub(1).min(t.rows.saturating_sub(1));
                t.cursor_col = col.saturating_sub(1).min(t.cols.saturating_sub(1));
            }
            'd' => {
                let row = p0();
                t.cursor_row = row.saturating_sub(1).min(t.rows.saturating_sub(1));
            }
            'G' => {
                let col = p0();
                t.cursor_col = col.saturating_sub(1).min(t.cols.saturating_sub(1));
            }
            'J' => match raw(0) {
                0 => {
                    t.clear_region(
                        t.cursor_row,
                        t.cursor_col,
                        t.cursor_row,
                        t.cols.saturating_sub(1),
                    );
                    if t.cursor_row + 1 < t.rows {
                        t.clear_region(t.cursor_row + 1, 0, t.rows - 1, t.cols.saturating_sub(1));
                    }
                }
                1 => {
                    if t.cursor_row > 0 {
                        t.clear_region(0, 0, t.cursor_row - 1, t.cols.saturating_sub(1));
                    }
                    t.clear_region(t.cursor_row, 0, t.cursor_row, t.cursor_col);
                }
                2 => {
                    t.clear_region(0, 0, t.rows.saturating_sub(1), t.cols.saturating_sub(1));
                }
                3 => {
                    t.clear_region(0, 0, t.rows.saturating_sub(1), t.cols.saturating_sub(1));
                    t.scrollback.clear();
                }
                _ => {}
            },
            'K' => match raw(0) {
                0 => {
                    t.clear_region(
                        t.cursor_row,
                        t.cursor_col,
                        t.cursor_row,
                        t.cols.saturating_sub(1),
                    );
                }
                1 => {
                    t.clear_region(t.cursor_row, 0, t.cursor_row, t.cursor_col);
                }
                2 => {
                    t.clear_region(t.cursor_row, 0, t.cursor_row, t.cols.saturating_sub(1));
                }
                _ => {}
            },
            'm' => handle_sgr(t, &pv),
            _ => {}
        }
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        if params.len() >= 2 {
            let cmd = params[0];
            if cmd == b"0" || cmd == b"2" {
                if let Ok(title) = std::str::from_utf8(params[1]) {
                    self.terminal.title = title.to_string();
                }
            }
        }
    }

    fn esc_dispatch(&mut self, _intermediates: &[u8], _ignore: bool, byte: u8) {
        let t = &mut *self.terminal;
        match byte {
            b'7' => {
                t.saved_cursor = (t.cursor_row, t.cursor_col);
            }
            b'8' => {
                t.cursor_row = t.saved_cursor.0.min(t.rows.saturating_sub(1));
                t.cursor_col = t.saved_cursor.1.min(t.cols.saturating_sub(1));
            }
            _ => {}
        }
    }

    fn hook(&mut self, _params: &Params, _intermediates: &[u8], _ignore: bool, _action: char) {}
    fn put(&mut self, _byte: u8) {}
    fn unhook(&mut self) {}
}

fn handle_sgr(t: &mut Terminal, pv: &[u16]) {
    if pv.is_empty() {
        reset_attrs(t);
        return;
    }
    let mut i = 0;
    while i < pv.len() {
        let code = pv[i];
        match code {
            0 => reset_attrs(t),
            1 => t.attrs |= CellAttrs::BOLD,
            2 => t.attrs |= CellAttrs::DIM,
            3 => t.attrs |= CellAttrs::ITALIC,
            4 => t.attrs |= CellAttrs::UNDERLINE,
            5 => t.attrs |= CellAttrs::BLINK,
            7 => t.attrs |= CellAttrs::INVERSE,
            9 => t.attrs |= CellAttrs::STRIKETHROUGH,
            22 => t.attrs &= !(CellAttrs::BOLD | CellAttrs::DIM),
            23 => t.attrs &= !CellAttrs::ITALIC,
            24 => t.attrs &= !CellAttrs::UNDERLINE,
            25 => t.attrs &= !CellAttrs::BLINK,
            27 => t.attrs &= !CellAttrs::INVERSE,
            29 => t.attrs &= !CellAttrs::STRIKETHROUGH,
            30..=37 => t.fg = ANSI_16[(code - 30) as usize],
            38 => {
                i += 1;
                if i < pv.len() {
                    match pv[i] {
                        5 => {
                            i += 1;
                            if i < pv.len() {
                                t.fg = color_256(pv[i] as u8);
                            }
                        }
                        2 => {
                            if i + 3 < pv.len() {
                                t.fg =
                                    Color::rgb(pv[i + 1] as u8, pv[i + 2] as u8, pv[i + 3] as u8);
                                i += 3;
                            }
                        }
                        _ => {}
                    }
                }
            }
            39 => t.fg = DEFAULT_FG,
            40..=47 => t.bg = ANSI_16[(code - 40) as usize],
            48 => {
                i += 1;
                if i < pv.len() {
                    match pv[i] {
                        5 => {
                            i += 1;
                            if i < pv.len() {
                                t.bg = color_256(pv[i] as u8);
                            }
                        }
                        2 => {
                            if i + 3 < pv.len() {
                                t.bg =
                                    Color::rgb(pv[i + 1] as u8, pv[i + 2] as u8, pv[i + 3] as u8);
                                i += 3;
                            }
                        }
                        _ => {}
                    }
                }
            }
            49 => t.bg = DEFAULT_BG,
            90..=97 => t.fg = ANSI_16[(code - 90 + 8) as usize],
            100..=107 => t.bg = ANSI_16[(code - 100 + 8) as usize],
            _ => {}
        }
        i += 1;
    }
}

fn reset_attrs(t: &mut Terminal) {
    t.fg = DEFAULT_FG;
    t.bg = DEFAULT_BG;
    t.attrs = CellAttrs::empty();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row_text(t: &Terminal, row: usize) -> String {
        let cells = t.grid().row(row).unwrap();
        let mut s: String = cells.iter().map(|c| c.ch).collect();
        while s.ends_with(' ') {
            s.pop();
        }
        s
    }

    #[test]
    fn process_bytes_writes_at_cursor_and_advances() {
        let mut t = Terminal::new(3, 5, 100);
        t.process_bytes(b"hi");
        assert_eq!(t.grid().get(0, 0).unwrap().ch, 'h');
        assert_eq!(t.grid().get(0, 1).unwrap().ch, 'i');
        assert_eq!(t.grid().cursor(), (0, 2));
    }

    #[test]
    fn newline_moves_cursor_to_next_row_preserving_column() {
        let mut t = Terminal::new(3, 5, 100);
        t.process_bytes(b"ab\n");
        // Column is preserved; only `\r` resets it.
        assert_eq!(t.grid().cursor(), (1, 2));
    }

    #[test]
    fn carriage_return_resets_column() {
        let mut t = Terminal::new(3, 5, 100);
        t.process_bytes(b"ab\rc");
        assert_eq!(t.grid().get(0, 0).unwrap().ch, 'c');
        assert_eq!(t.grid().get(0, 1).unwrap().ch, 'b');
    }

    #[test]
    fn tab_moves_to_next_8_col_stop() {
        let mut t = Terminal::new(1, 40, 100);
        t.process_bytes(b"a\tb");
        assert_eq!(t.grid().get(0, 8).unwrap().ch, 'b');
        assert_eq!(t.grid().cursor(), (0, 9));
    }

    #[test]
    fn backspace_moves_cursor_left() {
        let mut t = Terminal::new(1, 10, 100);
        t.process_bytes(b"abc\x08X");
        assert_eq!(row_text(&t, 0), "abX");
    }

    #[test]
    fn bell_is_silently_ignored() {
        let mut t = Terminal::new(1, 10, 100);
        t.process_bytes(b"a\x07b");
        assert_eq!(row_text(&t, 0), "ab");
    }

    #[test]
    fn clear_and_home_empties_grid_and_cursor_at_origin() {
        let mut t = Terminal::new(3, 5, 100);
        t.process_bytes(b"hello\nworld");
        t.process_bytes(b"\x1b[2J\x1b[H");
        for r in 0..3 {
            for c in 0..5 {
                assert_eq!(t.grid().get(r, c).unwrap().ch, ' ');
            }
        }
        assert_eq!(t.grid().cursor(), (0, 0));
    }

    #[test]
    fn overflow_pushes_top_row_into_scrollback() {
        let mut t = Terminal::new(2, 10, 100);
        t.process_bytes(b"aaaaa\r\nbbbbb\r\nccccc");
        // After the third line the original `aaaaa` scrolls off.
        assert!(!t.scrollback().is_empty());
        let first: String = t
            .scrollback()
            .lines()
            .next()
            .unwrap()
            .iter()
            .map(|c| c.ch)
            .collect();
        assert!(first.starts_with("aaaaa"));
    }

    #[test]
    fn sgr_31_sets_red_foreground() {
        let mut t = Terminal::new(1, 2, 10);
        t.process_bytes(b"\x1b[31ma");
        assert_eq!(t.grid().get(0, 0).unwrap().fg, ANSI_16[1]);
    }

    #[test]
    fn sgr_256_color_foreground() {
        let mut t = Terminal::new(1, 2, 10);
        t.process_bytes(b"\x1b[38;5;196ma");
        assert_eq!(t.grid().get(0, 0).unwrap().fg, color_256(196));
    }

    #[test]
    fn sgr_truecolor_foreground() {
        let mut t = Terminal::new(1, 2, 10);
        t.process_bytes(b"\x1b[38;2;100;150;200ma");
        assert_eq!(t.grid().get(0, 0).unwrap().fg, Color::rgb(100, 150, 200));
    }

    #[test]
    fn sgr_reset_clears_attrs() {
        let mut t = Terminal::new(1, 3, 10);
        t.process_bytes(b"\x1b[1;31ma\x1b[0mb");
        assert_eq!(t.grid().get(0, 0).unwrap().attrs, CellAttrs::BOLD);
        assert_eq!(t.grid().get(0, 0).unwrap().fg, ANSI_16[1]);
        assert_eq!(t.grid().get(0, 1).unwrap().attrs, CellAttrs::empty());
        assert_eq!(t.grid().get(0, 1).unwrap().fg, DEFAULT_FG);
    }

    #[test]
    fn dectcem_toggles_cursor_visibility() {
        let mut t = Terminal::new(3, 5, 10);
        assert!(t.grid().cursor_visible());
        t.process_bytes(b"\x1b[?25l");
        assert!(!t.grid().cursor_visible());
        t.process_bytes(b"\x1b[?25h");
        assert!(t.grid().cursor_visible());
    }

    #[test]
    fn osc_0_sets_title() {
        let mut t = Terminal::new(3, 5, 10);
        t.process_bytes(b"\x1b]0;hello\x07");
        assert_eq!(t.title(), "hello");
    }

    #[test]
    fn osc_2_sets_title() {
        let mut t = Terminal::new(3, 5, 10);
        t.process_bytes(b"\x1b]2;world\x07");
        assert_eq!(t.title(), "world");
    }

    #[test]
    fn snapshot_includes_grid_and_scrollback_tail() {
        let mut t = Terminal::new(2, 3, 100);
        t.process_bytes(b"aaa\r\nbbb\r\nccc");
        let snap = t.snapshot(1000);
        assert_eq!(snap.grid, *t.grid());
        assert_eq!(snap.scrollback.len(), t.scrollback().len());
        assert!(!snap.scrollback.is_empty());
    }

    #[test]
    fn snapshot_caps_scrollback_window() {
        let mut t = Terminal::new(1, 2, 100);
        // Each "x\r\n" pushes one line into scrollback.
        for _ in 0..5 {
            t.process_bytes(b"x\r\n");
        }
        let snap = t.snapshot(2);
        assert!(snap.scrollback.len() <= 2);
    }

    #[test]
    fn resize_clamps_cursor() {
        let mut t = Terminal::new(4, 6, 10);
        t.process_bytes(b"\x1b[4;6H");
        assert_eq!(t.grid().cursor(), (3, 5));
        t.resize(2, 3);
        assert_eq!(t.grid().cursor(), (1, 2));
    }

    #[test]
    fn cursor_position_cup_is_one_based() {
        let mut t = Terminal::new(5, 5, 10);
        t.process_bytes(b"\x1b[3;2H");
        assert_eq!(t.grid().cursor(), (2, 1));
    }

    #[test]
    fn csi_cha_sets_column_absolute() {
        let mut t = Terminal::new(1, 10, 10);
        t.process_bytes(b"hello\x1b[3G");
        assert_eq!(t.grid().cursor(), (0, 2));
    }

    #[test]
    fn csi_vpa_sets_row_absolute() {
        let mut t = Terminal::new(5, 5, 10);
        t.process_bytes(b"\x1b[3d");
        assert_eq!(t.grid().cursor(), (2, 0));
    }

    #[test]
    fn csi_el_zero_erases_to_line_end() {
        let mut t = Terminal::new(1, 10, 10);
        t.process_bytes(b"hello\x1b[3G\x1b[0K");
        assert_eq!(t.grid().get(0, 0).unwrap().ch, 'h');
        assert_eq!(t.grid().get(0, 1).unwrap().ch, 'e');
        assert_eq!(t.grid().get(0, 2).unwrap().ch, ' ');
        assert_eq!(t.grid().get(0, 4).unwrap().ch, ' ');
    }

    #[test]
    fn decscusr_is_silently_accepted() {
        // We do not model cursor shape yet; `\x1b[2 q` must not break the parser.
        let mut t = Terminal::new(1, 2, 10);
        t.process_bytes(b"\x1b[2 qa");
        assert_eq!(t.grid().get(0, 0).unwrap().ch, 'a');
    }

    #[test]
    fn background_sgr_applies_to_cell() {
        let mut t = Terminal::new(1, 2, 10);
        t.process_bytes(b"\x1b[41ma");
        assert_eq!(t.grid().get(0, 0).unwrap().bg, ANSI_16[1]);
    }

    #[test]
    fn bright_fg_maps_to_upper_ansi_half() {
        let mut t = Terminal::new(1, 2, 10);
        t.process_bytes(b"\x1b[91ma");
        assert_eq!(t.grid().get(0, 0).unwrap().fg, ANSI_16[9]);
    }

    // -- Resize reflow (issue #129) ----------------------------------------
    //
    // Splitting a pane and unsplitting triggers two PTY resize calls. The
    // old behavior was a naive top-left clip: shrink discarded top rows
    // silently and grow appended blanks at the bottom, so the live prompt
    // ended up mid-grid with a blank gap below it and old narrow-width
    // prompts left behind in the visible rows. The new behavior is
    // bottom-anchored: grow lifts scrollback into the new top rows, shrink
    // evicts top rows into scrollback.

    #[test]
    fn resize_grow_lifts_scrollback_into_new_top_rows() {
        // Issue #129 regression: when the grid grows, scrollback content
        // must fill the new top rows so the live prompt stays at the
        // bottom and there is no blank gap. Input is short (2 chars in
        // 4-col terminal) so the eager wrap-scroll path is not exercised.
        let mut t = Terminal::new(2, 4, 100);
        // Five logical lines, last one is the "prompt".
        t.process_bytes(b"L1\r\nL2\r\nL3\r\nL4\r\n>>");
        assert_eq!(t.scrollback().len(), 3);
        assert_eq!(row_text(&t, 0), "L4");
        assert_eq!(row_text(&t, 1), ">>");
        let cursor_before = t.grid().cursor();

        t.resize(4, 4);

        assert_eq!(t.grid().rows(), 4);
        assert_eq!(row_text(&t, 0), "L2");
        assert_eq!(row_text(&t, 1), "L3");
        assert_eq!(row_text(&t, 2), "L4");
        assert_eq!(row_text(&t, 3), ">>");
        assert_eq!(t.scrollback().len(), 1);
        // Cursor advanced by 2 (the row count delta) so it is still on
        // the same logical line ">>".
        assert_eq!(t.grid().cursor().0, cursor_before.0 + 2);
    }

    #[test]
    fn resize_no_blank_gap_after_split_unsplit_round_trip() {
        // Issue #129 reproduction: simulate a wide pane that gets split
        // (shrink) and then unsplit (grow back). After the round-trip,
        // every visible row should have content (no blank gap) and the
        // bottom row should still be the live prompt.
        let mut t = Terminal::new(6, 8, 100);
        // Six logical lines of wide content (6 chars each, 8-col grid).
        t.process_bytes(b"row1__\r\nrow2__\r\nrow3__\r\nrow4__\r\nrow5__\r\n>>>>>");
        let cursor_before = t.grid().cursor();

        t.resize(4, 8);
        assert_eq!(row_text(&t, 3), ">>>>>");

        t.resize(6, 8);

        assert_eq!(row_text(&t, 0), "row1__");
        assert_eq!(row_text(&t, 1), "row2__");
        assert_eq!(row_text(&t, 2), "row3__");
        assert_eq!(row_text(&t, 3), "row4__");
        assert_eq!(row_text(&t, 4), "row5__");
        assert_eq!(row_text(&t, 5), ">>>>>");
        assert_eq!(t.grid().cursor(), cursor_before);
        for r in 0..5 {
            assert_ne!(row_text(&t, r), "", "row {r} unexpectedly blank");
        }
    }

    #[test]
    fn resize_grow_with_empty_scrollback_blanks_top_and_advances_cursor() {
        let mut t = Terminal::new(2, 3, 100);
        t.process_bytes(b"aa\r\nbb");
        assert_eq!(t.grid().cursor().0, 1);
        assert!(t.scrollback().is_empty());

        t.resize(4, 3);

        // No scrollback, so the new top rows are blank; existing content
        // shifted down by 2.
        assert_eq!(row_text(&t, 0), "");
        assert_eq!(row_text(&t, 1), "");
        assert_eq!(row_text(&t, 2), "aa");
        assert_eq!(row_text(&t, 3), "bb");
        assert_eq!(t.grid().cursor().0, 3);
    }

    #[test]
    fn resize_grow_when_scrollback_smaller_than_delta() {
        let mut t = Terminal::new(2, 3, 100);
        t.process_bytes(b"aa\r\nbb\r\ncc");
        assert_eq!(t.scrollback().len(), 1);

        t.resize(5, 3);

        // Only 1 line in scrollback to lift, but we grew by 3. The lifted
        // row sits adjacent to the original top so the bottom stays
        // anchored: top 2 rows blank, then "aa", then "bb", "cc".
        assert_eq!(row_text(&t, 0), "");
        assert_eq!(row_text(&t, 1), "");
        assert_eq!(row_text(&t, 2), "aa");
        assert_eq!(row_text(&t, 3), "bb");
        assert_eq!(row_text(&t, 4), "cc");
        assert!(t.scrollback().is_empty());
        assert_eq!(t.grid().cursor().0, 4);
    }

    #[test]
    fn resize_shrink_pushes_top_rows_to_scrollback_and_decreases_cursor() {
        let mut t = Terminal::new(4, 3, 100);
        t.process_bytes(b"aa\r\nbb\r\ncc\r\ndd");
        assert_eq!(t.grid().cursor().0, 3);
        assert!(t.scrollback().is_empty());

        t.resize(2, 3);

        assert_eq!(t.grid().rows(), 2);
        assert_eq!(row_text(&t, 0), "cc");
        assert_eq!(row_text(&t, 1), "dd");
        assert_eq!(t.grid().cursor().0, 1);
        let scrolled: Vec<String> = t
            .scrollback()
            .lines()
            .map(|l| {
                l.iter()
                    .map(|c| c.ch)
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect();
        assert_eq!(scrolled, vec!["aa".to_string(), "bb".to_string()]);
    }

    #[test]
    fn resize_shrink_when_k_exceeds_cursor_row_evicts_only_above_cursor() {
        let mut t = Terminal::new(5, 3, 100);
        // Cursor stays on row 1 ("XX") with three blank rows below it.
        t.process_bytes(b"aa\r\nXX");
        assert_eq!(t.grid().cursor().0, 1);

        // Shrink by 4 rows. Only 1 row above the cursor (row 0 "aa") can
        // be pushed to scrollback; remaining 3 rows of trim come from
        // the blank tail below the cursor.
        t.resize(1, 3);

        assert_eq!(t.grid().rows(), 1);
        assert_eq!(row_text(&t, 0), "XX");
        assert_eq!(t.grid().cursor().0, 0);
        let scrolled: Vec<String> = t
            .scrollback()
            .lines()
            .map(|l| {
                l.iter()
                    .map(|c| c.ch)
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect();
        assert_eq!(scrolled, vec!["aa".to_string()]);
    }

    #[test]
    fn resize_shrink_respects_max_scrollback() {
        // Cap scrollback at 1; the second eviction pushes the first out.
        let mut t = Terminal::new(3, 4, 1);
        t.process_bytes(b"L1\r\nL2\r\nL3");
        // Visible: ["L1", "L2", "L3"]; scrollback empty.
        assert!(t.scrollback().is_empty());

        t.resize(1, 4);

        // Evict "L1" then "L2" in that order; cap is 1 so only "L2"
        // survives.
        assert_eq!(t.scrollback().len(), 1);
        let last: String = t
            .scrollback()
            .lines()
            .next()
            .unwrap()
            .iter()
            .map(|c| c.ch)
            .collect::<String>()
            .trim_end()
            .to_string();
        assert_eq!(last, "L2");
    }

    #[test]
    fn resize_round_trip_is_stable() {
        let mut t = Terminal::new(4, 3, 100);
        t.process_bytes(b"aa\r\nbb\r\ncc\r\ndd");
        let cursor_before = t.grid().cursor();

        t.resize(2, 3);
        t.resize(4, 3);

        // After shrink-then-grow, the visible grid recovers and the
        // cursor returns to its pre-resize row.
        assert_eq!(row_text(&t, 0), "aa");
        assert_eq!(row_text(&t, 1), "bb");
        assert_eq!(row_text(&t, 2), "cc");
        assert_eq!(row_text(&t, 3), "dd");
        assert_eq!(t.grid().cursor(), cursor_before);
    }

    #[test]
    fn resize_column_only_does_not_touch_scrollback() {
        let mut t = Terminal::new(2, 3, 100);
        t.process_bytes(b"aa\r\nbb\r\ncc");
        let scrollback_before = t.scrollback().len();

        t.resize(2, 5);

        assert_eq!(t.scrollback().len(), scrollback_before);
        assert_eq!(t.grid().cols(), 5);
    }

    #[test]
    fn resize_to_zero_rows_does_not_panic() {
        let mut t = Terminal::new(2, 3, 10);
        t.process_bytes(b"aa\r\nbb");
        t.resize(0, 3);
        assert_eq!(t.grid().rows(), 0);
        // Cursor row is forced to 0 when rows == 0; cursor col is left
        // at whatever it was clamped to (the grid's own cursor clamp).
        assert_eq!(t.grid().cursor().0, 0);
    }
}
