use vte::{Params, Perform};

use crate::cell::{Cell, CellAttrs};
use crate::color::{color_256, Color, ANSI_16};
use crate::grid::Grid;
use crate::scrollback::Scrollback;
use crate::snapshot::Snapshot;

const ENV_PARITY_WINDOWS_TERMINAL_COLORS: &str = "TM_PARITY_WINDOWS_TERMINAL_COLORS";
const DEFAULT_FG: Color = Color::WHITE;
// Windows Terminal's Campbell foreground setting is #cccccc, but this
// renderer's atlas/blending path matches WT captures more closely with #c4c4c4.
const WINDOWS_TERMINAL_PARITY_DEFAULT_FG: Color = Color::rgb(196, 196, 196);
const WINDOWS_TERMINAL_ANSI_16: [Color; 16] = [
    Color::rgb(12, 12, 12),
    Color::rgb(197, 15, 31),
    Color::rgb(19, 161, 14),
    Color::rgb(193, 156, 0),
    Color::rgb(0, 55, 218),
    Color::rgb(136, 23, 152),
    Color::rgb(58, 150, 221),
    Color::rgb(204, 204, 204),
    Color::rgb(118, 118, 118),
    Color::rgb(231, 72, 86),
    Color::rgb(22, 198, 12),
    Color::rgb(249, 241, 165),
    Color::rgb(59, 120, 255),
    Color::rgb(180, 0, 158),
    Color::rgb(97, 214, 214),
    Color::rgb(242, 242, 242),
];
const DEFAULT_BG: Color = Color::TRANSPARENT;

fn parity_windows_terminal_colors_enabled() -> bool {
    std::env::var_os(ENV_PARITY_WINDOWS_TERMINAL_COLORS)
        .filter(|v| !v.is_empty())
        .map(|v| {
            let normalized = v.to_string_lossy().trim().to_ascii_lowercase();
            !matches!(normalized.as_str(), "0" | "false" | "off" | "no")
        })
        .unwrap_or(false)
}

fn default_fg_for_parity(enabled: bool) -> Color {
    if enabled {
        WINDOWS_TERMINAL_PARITY_DEFAULT_FG
    } else {
        DEFAULT_FG
    }
}

fn default_fg() -> Color {
    default_fg_for_parity(parity_windows_terminal_colors_enabled())
}

fn default_bg() -> Color {
    DEFAULT_BG
}

fn ansi_16_color_for_parity(index: usize, enabled: bool) -> Color {
    if enabled {
        WINDOWS_TERMINAL_ANSI_16[index]
    } else {
        ANSI_16[index]
    }
}

fn ansi_16_fg_color_for_parity(index: usize, enabled: bool) -> Color {
    let color = ansi_16_color_for_parity(index, enabled);
    if !enabled {
        return color;
    }

    match index {
        7 => WINDOWS_TERMINAL_PARITY_DEFAULT_FG,
        8 => Color::rgb(114, 114, 114),
        _ => color,
    }
}

fn ansi_16_fg_color(index: usize) -> Color {
    ansi_16_fg_color_for_parity(index, parity_windows_terminal_colors_enabled())
}

fn ansi_16_color(index: usize) -> Color {
    ansi_16_color_for_parity(index, parity_windows_terminal_colors_enabled())
}

fn fg_color_256_for_parity_with_profile(index: u8, parity_enabled: bool) -> Color {
    if index < 16 {
        ansi_16_fg_color_for_parity(index as usize, parity_enabled)
    } else {
        color_256(index)
    }
}

fn bg_color_256_for_parity_with_profile(index: u8, parity_enabled: bool) -> Color {
    if index < 16 {
        ansi_16_color_for_parity(index as usize, parity_enabled)
    } else {
        color_256(index)
    }
}

fn fg_color_256_for_parity(index: u8) -> Color {
    fg_color_256_for_parity_with_profile(index, parity_windows_terminal_colors_enabled())
}

fn bg_color_256_for_parity(index: u8) -> Color {
    bg_color_256_for_parity_with_profile(index, parity_windows_terminal_colors_enabled())
}

pub struct Terminal {
    grid: Grid,
    scrollback: Scrollback,
    parser: vte::Parser,
    rows: usize,
    cols: usize,
    cursor_row: usize,
    cursor_col: usize,
    wrap_pending: bool,
    saved_cursor: (usize, usize),
    fg: Color,
    bg: Color,
    attrs: CellAttrs,
    title: String,
    alt_grid: Option<Grid>,
    alt_saved_cursor: (usize, usize),
    alt_saved_fg: Color,
    alt_saved_bg: Color,
    alt_saved_attrs: CellAttrs,
    scroll_top: usize,
    scroll_bot: usize,
    pending_response: Vec<u8>,
    synchronized_output_active: bool,
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
            wrap_pending: false,
            saved_cursor: (0, 0),
            fg: default_fg(),
            bg: default_bg(),
            attrs: CellAttrs::empty(),
            title: String::new(),
            alt_grid: None,
            alt_saved_cursor: (0, 0),
            alt_saved_fg: default_fg(),
            alt_saved_bg: default_bg(),
            alt_saved_attrs: CellAttrs::empty(),
            scroll_top: 0,
            scroll_bot: rows,
            pending_response: Vec::new(),
            synchronized_output_active: false,
        }
    }

    pub fn take_pending_response(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.pending_response)
    }

    pub fn synchronized_output_active(&self) -> bool {
        self.synchronized_output_active
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
        let alt_active = self.alt_grid.is_some();
        let scroll_region_was_full_screen = self.region_is_full_screen();

        if alt_active {
            if let Some(main) = self.alt_grid.take() {
                let mut previous = main;
                std::mem::swap(&mut self.grid, &mut previous);
                self.alt_grid = Some(previous);
            }
        }

        if rows > old_rows {
            let k = rows - old_rows;
            let lifted = self.scrollback.pop_back_n(k);
            self.grid.grow_rows_at_top(k, lifted);
            if alt_active {
                self.alt_saved_cursor.0 += k;
            } else {
                self.cursor_row += k;
            }
        } else if rows < old_rows {
            let k = old_rows - rows;
            // Only evict rows that sit above the cursor, so the live
            // prompt row is never pushed into scrollback.
            let cursor_for_eviction = if alt_active {
                self.alt_saved_cursor.0
            } else {
                self.cursor_row
            };
            let evict_above = k.min(cursor_for_eviction);
            if evict_above > 0 {
                let evicted = self.grid.shrink_rows_from_top(evict_above);
                for line in evicted {
                    self.scrollback.push(line);
                }
                if alt_active {
                    self.alt_saved_cursor.0 -= evict_above;
                } else {
                    self.cursor_row -= evict_above;
                }
            }
            // Any remaining shrink (k > cursor_row) trims the blank tail
            // below the cursor; grid.resize handles it by clipping.
        }

        self.grid.resize(rows, cols);
        if alt_active {
            if let Some(mut main) = self.alt_grid.take() {
                std::mem::swap(&mut self.grid, &mut main);
                self.grid.resize(rows, cols);
                self.alt_grid = Some(main);
            }
        }
        self.rows = rows;
        self.cols = cols;

        self.clamp_scroll_region_after_resize(rows, scroll_region_was_full_screen);

        self.cursor_row = self.cursor_row.min(rows.saturating_sub(1));
        self.cursor_col = self.cursor_col.min(cols.saturating_sub(1));
        self.wrap_pending = false;
        if alt_active {
            self.alt_saved_cursor.0 = self.alt_saved_cursor.0.min(rows.saturating_sub(1));
            self.alt_saved_cursor.1 = self.alt_saved_cursor.1.min(cols.saturating_sub(1));
        }
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

    fn clear_pending_wrap(&mut self) {
        self.wrap_pending = false;
    }

    fn wrap_to_next_line(&mut self) {
        self.cursor_col = 0;
        if self.cursor_row + 1 == self.scroll_bot {
            self.scroll_up();
        } else if self.cursor_row + 1 < self.rows {
            self.cursor_row += 1;
        } else {
            self.cursor_row = self.rows.saturating_sub(1);
        }
        self.wrap_pending = false;
    }

    fn prepare_for_printable(&mut self) {
        if self.wrap_pending {
            self.wrap_to_next_line();
        }
    }

    fn region_is_full_screen(&self) -> bool {
        self.scroll_top == 0 && self.scroll_bot == self.rows
    }

    fn clamp_scroll_region_after_resize(&mut self, rows: usize, was_full_screen: bool) {
        if was_full_screen
            || self.scroll_top >= rows
            || self.scroll_bot > rows
            || self.scroll_top >= self.scroll_bot
        {
            self.scroll_top = 0;
            self.scroll_bot = rows;
        }
    }

    fn copy_row(&mut self, dst: usize, src: usize) {
        if dst >= self.rows || src >= self.rows {
            return;
        }
        for col in 0..self.cols {
            let cell = self.grid.get(src, col).copied().unwrap_or(Cell::BLANK);
            self.grid.set(dst, col, cell);
        }
    }

    fn clear_row(&mut self, row: usize) {
        if row >= self.rows {
            return;
        }
        let blank = Cell {
            ch: ' ',
            fg: self.fg,
            bg: self.bg,
            attrs: CellAttrs::empty(),
        };
        for col in 0..self.cols {
            self.grid.set(row, col, blank);
        }
    }

    fn scroll_up(&mut self) {
        if self.rows == 0 || self.scroll_bot <= self.scroll_top {
            return;
        }
        if self.region_is_full_screen() {
            self.scroll_up_and_capture();
            return;
        }

        let top = self.scroll_top;
        let bot = self.scroll_bot;
        for row in top..bot.saturating_sub(1) {
            self.copy_row(row, row + 1);
        }
        self.clear_row(bot - 1);
    }

    fn scroll_down(&mut self) {
        if self.rows == 0 || self.scroll_bot <= self.scroll_top {
            return;
        }
        let top = self.scroll_top;
        let bot = self.scroll_bot;
        for row in (top + 1..bot).rev() {
            self.copy_row(row, row - 1);
        }
        self.clear_row(top);
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
        self.prepare_for_printable();
        let cell = Cell {
            ch: c,
            fg: self.fg,
            bg: self.bg,
            attrs: self.attrs,
        };
        self.grid.set(self.cursor_row, self.cursor_col, cell);
        if self.cursor_col + 1 >= self.cols {
            self.cursor_col = self.cols - 1;
            self.wrap_pending = true;
        } else {
            self.cursor_col += 1;
            self.wrap_pending = false;
        }
    }

    fn enter_alt_screen(&mut self) {
        if self.alt_grid.is_some() {
            return;
        }
        self.clear_pending_wrap();
        self.alt_saved_cursor = (self.cursor_row, self.cursor_col);
        self.alt_saved_fg = self.fg;
        self.alt_saved_bg = self.bg;
        self.alt_saved_attrs = self.attrs;

        let mut fresh = Grid::new(self.rows, self.cols);
        std::mem::swap(&mut self.grid, &mut fresh);
        self.alt_grid = Some(fresh);

        self.cursor_row = 0;
        self.cursor_col = 0;
        self.grid.set_cursor(0, 0);
    }

    fn exit_alt_screen(&mut self) {
        let Some(mut main) = self.alt_grid.take() else {
            return;
        };
        std::mem::swap(&mut self.grid, &mut main);
        self.cursor_row = self.alt_saved_cursor.0.min(self.rows.saturating_sub(1));
        self.cursor_col = self.alt_saved_cursor.1.min(self.cols.saturating_sub(1));
        self.clear_pending_wrap();
        self.fg = self.alt_saved_fg;
        self.bg = self.alt_saved_bg;
        self.attrs = self.alt_saved_attrs;
        self.grid.set_cursor(self.cursor_row, self.cursor_col);
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
                t.clear_pending_wrap();
                if t.cursor_row + 1 == t.scroll_bot {
                    t.scroll_up();
                } else if t.cursor_row + 1 < t.rows {
                    t.cursor_row += 1;
                } else {
                    t.cursor_row = t.rows.saturating_sub(1);
                }
            }
            0x0D => {
                t.clear_pending_wrap();
                t.cursor_col = 0;
            }
            0x09 => {
                t.clear_pending_wrap();
                let next_tab = (t.cursor_col / 8 + 1) * 8;
                t.cursor_col = next_tab.min(t.cols.saturating_sub(1));
            }
            0x08 => {
                t.clear_pending_wrap();
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
                        match *code {
                            25 => t.grid.set_cursor_visible(on),
                            2026 => t.synchronized_output_active = on,
                            47 | 1047 | 1049 if on => t.enter_alt_screen(),
                            47 | 1047 | 1049 => t.exit_alt_screen(),
                            _ => {}
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

        if !matches!(action, 'm' | 'c' | 'n' | 'q') {
            t.clear_pending_wrap();
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
            'L' => {
                let n = p0();
                let cursor_row = t.cursor_row;
                if cursor_row >= t.scroll_top && cursor_row < t.scroll_bot {
                    let bot = t.scroll_bot;
                    let n = n.min(bot.saturating_sub(cursor_row));
                    if n > 0 {
                        for row in (cursor_row + n..bot).rev() {
                            t.copy_row(row, row - n);
                        }
                        for row in cursor_row..cursor_row + n {
                            t.clear_row(row);
                        }
                    }
                }
            }
            'M' if intermediates.is_empty() => {
                let n = p0();
                let cursor_row = t.cursor_row;
                if cursor_row >= t.scroll_top && cursor_row < t.scroll_bot {
                    let bot = t.scroll_bot;
                    let n = n.min(bot.saturating_sub(cursor_row));
                    if n > 0 {
                        for row in cursor_row..bot.saturating_sub(n) {
                            t.copy_row(row, row + n);
                        }
                        for row in bot.saturating_sub(n)..bot {
                            t.clear_row(row);
                        }
                    }
                }
            }
            'S' => {
                for _ in 0..p0() {
                    t.scroll_up();
                }
            }
            'T' => {
                for _ in 0..p0() {
                    t.scroll_down();
                }
            }
            '@' => {
                let n = p0().min(t.cols.saturating_sub(t.cursor_col));
                for col in (t.cursor_col + n..t.cols).rev() {
                    if let Some(cell) = t.grid.get(t.cursor_row, col - n).copied() {
                        t.grid.set(t.cursor_row, col, cell);
                    }
                }
                let blank = Cell {
                    ch: ' ',
                    fg: t.fg,
                    bg: t.bg,
                    attrs: CellAttrs::empty(),
                };
                for col in t.cursor_col..t.cursor_col + n {
                    t.grid.set(t.cursor_row, col, blank);
                }
            }
            'P' => {
                let n = p0().min(t.cols.saturating_sub(t.cursor_col));
                for col in t.cursor_col..t.cols.saturating_sub(n) {
                    if let Some(cell) = t.grid.get(t.cursor_row, col + n).copied() {
                        t.grid.set(t.cursor_row, col, cell);
                    }
                }
                let blank = Cell {
                    ch: ' ',
                    fg: t.fg,
                    bg: t.bg,
                    attrs: CellAttrs::empty(),
                };
                for col in t.cols.saturating_sub(n)..t.cols {
                    t.grid.set(t.cursor_row, col, blank);
                }
            }
            'X' => {
                let n = p0().min(t.cols.saturating_sub(t.cursor_col));
                let blank = Cell {
                    ch: ' ',
                    fg: t.fg,
                    bg: t.bg,
                    attrs: CellAttrs::empty(),
                };
                for col in t.cursor_col..t.cursor_col + n {
                    t.grid.set(t.cursor_row, col, blank);
                }
            }
            'r' if intermediates.is_empty() => {
                let rows = t.rows;
                let top_1 = pv.first().copied().unwrap_or(0);
                let bot_1 = pv.get(1).copied().unwrap_or(0);
                let top = if top_1 == 0 { 1 } else { top_1 as usize };
                let bot = if bot_1 == 0 { rows } else { bot_1 as usize };
                let new_top = top.saturating_sub(1);
                let new_bot = bot.min(rows);
                if new_top < new_bot && new_bot <= rows {
                    t.scroll_top = new_top;
                    t.scroll_bot = new_bot;
                    t.cursor_row = 0;
                    t.cursor_col = 0;
                }
            }
            'c' if intermediates.is_empty() => {
                t.pending_response.extend_from_slice(b"\x1b[?1;2c");
            }
            'c' if intermediates == [b'>'] => {
                t.pending_response.extend_from_slice(b"\x1b[>0;95;0c");
            }
            'n' if intermediates.is_empty() && pv.first() == Some(&5) => {
                t.pending_response.extend_from_slice(b"\x1b[0n");
            }
            'n' if intermediates.is_empty() && pv.first() == Some(&6) => {
                let reply = format!("\x1b[{};{}R", t.cursor_row + 1, t.cursor_col + 1);
                t.pending_response.extend_from_slice(reply.as_bytes());
            }
            'q' if intermediates == [b'>'] => {
                t.pending_response
                    .extend_from_slice(b"\x1bP>|godly-terminal 0.1\x1b\\");
            }
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
                t.clear_pending_wrap();
                t.cursor_row = t.saved_cursor.0.min(t.rows.saturating_sub(1));
                t.cursor_col = t.saved_cursor.1.min(t.cols.saturating_sub(1));
            }
            b'M' => {
                t.clear_pending_wrap();
                if t.cursor_row == 0 {
                    t.scroll_down();
                } else {
                    t.cursor_row -= 1;
                }
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
            30..=37 => t.fg = ansi_16_fg_color((code - 30) as usize),
            38 => {
                i += 1;
                if i < pv.len() {
                    match pv[i] {
                        5 => {
                            i += 1;
                            if i < pv.len() {
                                t.fg = fg_color_256_for_parity(pv[i] as u8);
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
            39 => t.fg = default_fg(),
            40..=47 => t.bg = ansi_16_color((code - 40) as usize),
            48 => {
                i += 1;
                if i < pv.len() {
                    match pv[i] {
                        5 => {
                            i += 1;
                            if i < pv.len() {
                                t.bg = bg_color_256_for_parity(pv[i] as u8);
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
            49 => t.bg = default_bg(),
            90..=97 => t.fg = ansi_16_fg_color((code - 90 + 8) as usize),
            100..=107 => t.bg = ansi_16_color((code - 100 + 8) as usize),
            _ => {}
        }
        i += 1;
    }
}

fn reset_attrs(t: &mut Terminal) {
    t.fg = default_fg();
    t.bg = default_bg();
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
    fn full_width_line_delays_wrap_until_next_printable() {
        let mut t = Terminal::new(2, 5, 100);
        t.process_bytes(b"ABCDE");

        assert_eq!(t.grid().cursor(), (0, 4));
        assert_eq!(row_text(&t, 0), "ABCDE");
        assert_eq!(row_text(&t, 1), "");

        t.process_bytes(b"F");

        assert_eq!(t.grid().cursor(), (1, 1));
        assert_eq!(row_text(&t, 0), "ABCDE");
        assert_eq!(row_text(&t, 1), "F");
    }

    #[test]
    fn carriage_return_clears_pending_wrap_after_full_width_line() {
        let mut t = Terminal::new(2, 5, 100);
        t.process_bytes(b"ABCDE\rZ");

        assert_eq!(t.grid().cursor(), (0, 1));
        assert_eq!(row_text(&t, 0), "ZBCDE");
        assert_eq!(row_text(&t, 1), "");
    }

    #[test]
    fn terminal_query_preserves_pending_wrap() {
        let mut t = Terminal::new(2, 5, 100);
        t.process_bytes(b"ABCDE\x1b[6nF");

        assert_eq!(t.grid().cursor(), (1, 1));
        assert_eq!(row_text(&t, 0), "ABCDE");
        assert_eq!(row_text(&t, 1), "F");
    }

    #[test]
    fn host_queries_queue_capability_responses() {
        let mut t = Terminal::new(3, 5, 100);

        t.process_bytes(b"\x1b[c");
        assert_eq!(t.take_pending_response(), b"\x1b[?1;2c");

        t.process_bytes(b"\x1b[>c");
        assert_eq!(t.take_pending_response(), b"\x1b[>0;95;0c");

        t.process_bytes(b"\x1b[5n");
        assert_eq!(t.take_pending_response(), b"\x1b[0n");

        t.process_bytes(b"\x1b[2;3H\x1b[6n");
        assert_eq!(t.take_pending_response(), b"\x1b[2;3R");

        t.process_bytes(b"\x1b[>q");
        assert_eq!(
            t.take_pending_response(),
            b"\x1bP>|godly-terminal 0.1\x1b\\"
        );
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
    fn parity_default_fg_uses_capture_calibrated_windows_terminal_value() {
        assert_eq!(default_fg_for_parity(true), Color::rgb(196, 196, 196));
        assert_eq!(default_fg_for_parity(false), DEFAULT_FG);
    }

    #[test]
    fn parity_ansi_palette_matches_windows_terminal_campbell() {
        assert_eq!(ansi_16_color_for_parity(1, true), Color::rgb(197, 15, 31));
        assert_eq!(ansi_16_color_for_parity(1, false), ANSI_16[1]);
    }

    #[test]
    fn parity_neutral_foregrounds_are_capture_calibrated_separately_from_backgrounds() {
        assert_eq!(
            ansi_16_fg_color_for_parity(7, true),
            Color::rgb(196, 196, 196)
        );
        assert_eq!(
            ansi_16_fg_color_for_parity(8, true),
            Color::rgb(114, 114, 114)
        );
        assert_eq!(
            ansi_16_color_for_parity(7, true),
            Color::rgb(204, 204, 204),
            "background swatches should keep the literal Campbell palette"
        );
    }

    #[test]
    fn parity_256_color_low_indices_follow_ansi_profile_mapping() {
        assert_eq!(
            fg_color_256_for_parity_with_profile(7, true),
            Color::rgb(196, 196, 196)
        );
        assert_eq!(
            bg_color_256_for_parity_with_profile(7, true),
            Color::rgb(204, 204, 204)
        );
        assert_eq!(
            fg_color_256_for_parity_with_profile(8, true),
            Color::rgb(114, 114, 114)
        );
        assert_eq!(fg_color_256_for_parity_with_profile(7, false), ANSI_16[7]);
        assert_eq!(bg_color_256_for_parity_with_profile(7, false), ANSI_16[7]);
    }

    #[test]
    fn parity_256_color_high_indices_stay_in_256_color_cube() {
        assert_eq!(
            fg_color_256_for_parity_with_profile(196, true),
            color_256(196)
        );
        assert_eq!(
            bg_color_256_for_parity_with_profile(82, true),
            color_256(82)
        );
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
    fn synchronized_output_mode_tracks_dec_private_2026() {
        let mut t = Terminal::new(3, 5, 10);
        assert!(!t.synchronized_output_active());

        t.process_bytes(b"\x1b[?2026h");
        assert!(t.synchronized_output_active());

        t.process_bytes(b"\x1b[?2026l");
        assert!(!t.synchronized_output_active());
    }

    #[test]
    fn combined_private_modes_update_cursor_and_sync_output() {
        let mut t = Terminal::new(3, 5, 10);

        t.process_bytes(b"\x1b[?25;2026l");
        assert!(!t.grid().cursor_visible());
        assert!(!t.synchronized_output_active());

        t.process_bytes(b"\x1b[?25;2026h");
        assert!(t.grid().cursor_visible());
        assert!(t.synchronized_output_active());
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
    fn csi_ech_blanks_without_moving_cursor_or_shifting() {
        let mut t = Terminal::new(1, 10, 10);
        t.process_bytes(b"> abcdef\x1b[1;3H\x1b[4X");

        assert_eq!(t.grid().cursor(), (0, 2));
        assert_eq!(row_text(&t, 0), ">     ef");
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

    #[test]
    fn alt_screen_exit_restores_main_screen_and_cursor() {
        let mut t = Terminal::new(4, 6, 100);
        t.process_bytes(b"hello\r\nworld");
        let cursor_before = t.grid().cursor();

        t.process_bytes(b"\x1b[?1049h");
        t.process_bytes(b"ALT");
        assert_eq!(row_text(&t, 0), "ALT");

        t.process_bytes(b"\x1b[?1049l");

        assert_eq!(row_text(&t, 0), "hello");
        assert_eq!(row_text(&t, 1), "world");
        assert_eq!(t.grid().cursor(), cursor_before);
        assert!(t.alt_grid.is_none());
    }

    #[test]
    fn alt_screen_resize_scales_both_buffers() {
        let mut t = Terminal::new(4, 6, 100);
        t.process_bytes(b"main");
        t.process_bytes(b"\x1b[?1049h");
        t.process_bytes(b"alt");

        t.resize(6, 8);

        assert_eq!(t.grid().rows(), 6);
        assert_eq!(t.grid().cols(), 8);
        let main = t.alt_grid.as_ref().expect("main grid should be parked");
        assert_eq!(main.rows(), 6);
        assert_eq!(main.cols(), 8);

        t.process_bytes(b"\x1b[?1049l");
        assert_eq!(t.grid().rows(), 6);
        assert_eq!(t.grid().cols(), 8);
        assert_eq!(row_text(&t, 2), "main");
    }

    #[test]
    fn reverse_index_at_top_scrolls_down() {
        let mut t = Terminal::new(3, 10, 100);
        t.process_bytes(b"line1\r\nline2\r\nline3");
        t.process_bytes(b"\x1b[1;1H");
        t.process_bytes(b"\x1bM");

        assert_eq!(row_text(&t, 0), "");
        assert_eq!(row_text(&t, 1), "line1");
    }

    #[test]
    fn decstbm_lf_at_region_bottom_scrolls_region_only() {
        let mut t = Terminal::new(5, 4, 100);
        t.process_bytes(b"AA\r\nBB\r\nCC\r\nDD\r\nEE");
        t.process_bytes(b"\x1b[2;4r");
        t.process_bytes(b"\x1b[4;1H");
        t.process_bytes(b"\n");

        assert_eq!(t.grid().cursor(), (3, 0));
        assert_eq!(row_text(&t, 0), "AA");
        assert_eq!(row_text(&t, 1), "CC");
        assert_eq!(row_text(&t, 2), "DD");
        assert_eq!(row_text(&t, 3), "");
        assert_eq!(row_text(&t, 4), "EE");
    }

    #[test]
    fn decstbm_il_inside_region_preserves_pinned_rows() {
        let mut t = Terminal::new(5, 4, 100);
        t.process_bytes(b"AA\r\nBB\r\nCC\r\nDD\r\nEE");
        t.process_bytes(b"\x1b[2;4r");
        t.process_bytes(b"\x1b[2;1H");
        t.process_bytes(b"\x1b[1L");

        assert_eq!(row_text(&t, 0), "AA");
        assert_eq!(row_text(&t, 1), "");
        assert_eq!(row_text(&t, 2), "BB");
        assert_eq!(row_text(&t, 3), "CC");
        assert_eq!(row_text(&t, 4), "EE");
    }

    #[test]
    fn resize_expands_full_screen_scroll_region_for_conpty_redraw() {
        let mut t = Terminal::new(2, 10, 100);

        t.resize(4, 10);

        assert_eq!((t.scroll_top, t.scroll_bot), (0, 4));
        t.process_bytes(b"\x1b[Hone\x1b[K\r\ntwo\x1b[K\r\n\x1b[K\r\n\x1b[K\x1b[2;4H");

        assert_eq!(row_text(&t, 0), "one");
        assert_eq!(row_text(&t, 1), "two");
        assert_eq!(row_text(&t, 2), "");
        assert_eq!(row_text(&t, 3), "");
    }

    #[test]
    fn resize_preserves_valid_narrow_scroll_region() {
        let mut t = Terminal::new(5, 4, 100);
        t.process_bytes(b"\x1b[2;4r");

        t.resize(6, 4);

        assert_eq!((t.scroll_top, t.scroll_bot), (1, 4));
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
