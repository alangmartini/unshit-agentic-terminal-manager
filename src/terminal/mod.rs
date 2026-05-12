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
    /// Delayed autowrap state. When a printable lands in the last
    /// column, real terminals leave the cursor on that cell and wrap
    /// only before the next printable character. Full-width TUI rules
    /// depend on this to avoid creating phantom rows.
    wrap_pending: bool,
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
    /// When the alternate screen buffer is active, this holds the
    /// previously-active main screen so it can be swapped back on exit
    /// (DEC private mode 1049 / 47 / 1047). `None` means the main
    /// screen is currently in `grid`.
    alt_grid: Option<CellGrid>,
    /// Cursor position saved by the alt-screen entry sequence. Kept
    /// separate from `saved_cursor` (DECSC) so a TUI nesting `ESC 7` /
    /// `ESC 8` inside the alt screen cannot trample the slot used to
    /// restore the cursor on `?1049l` exit.
    alt_saved_cursor: (usize, usize),
    /// SGR foreground saved on alt-screen entry; restored on exit.
    alt_saved_fg: Color,
    /// SGR background saved on alt-screen entry; restored on exit.
    alt_saved_bg: Color,
    /// SGR attribute flags saved on alt-screen entry; restored on exit.
    alt_saved_attrs: CellAttrs,
    /// Top of the active scroll region (inclusive), zero-indexed.
    /// Half-open with `scroll_bot`: the region is `[scroll_top, scroll_bot)`.
    /// Default is 0; DECSTBM (`CSI <top>;<bot> r`) overrides it.
    scroll_top: usize,
    /// Bottom of the active scroll region (exclusive), zero-indexed.
    /// Default is `rows` (full screen). DECSTBM updates it; resize
    /// clamps it back when the previous region no longer fits.
    scroll_bot: usize,
    /// Bytes the parser produced as a reply to a host query (DA1, DA2,
    /// DSR, CPR, XTVERSION, ...). The bridge subscription drains this
    /// after every `process_bytes` and writes it back to the PTY so
    /// TUIs that probe terminal capabilities (Claude Code, vim, fzf)
    /// see a real terminal and pick their full-feature rendering path
    /// instead of falling back to a defensive minimal layout.
    pending_response: Vec<u8>,
}

const ENV_PARITY_WINDOWS_TERMINAL_COLORS: &str = "TM_PARITY_WINDOWS_TERMINAL_COLORS";

/// Default foreground: warm amber.
const DEFAULT_FG: Color = Color {
    r: 212,
    g: 163,
    b: 72,
    a: 255,
};

/// Calibrated foreground for Windows Terminal parity screenshots.
///
/// Windows Terminal's Campbell foreground setting is `#cccccc`, but this
/// renderer's atlas/blending path lands closer to WT captures with `#c4c4c4`.
const WINDOWS_TERMINAL_PARITY_DEFAULT_FG: Color = Color {
    r: 196,
    g: 196,
    b: 196,
    a: 255,
};

const WINDOWS_TERMINAL_ANSI_16: [Color; 16] = [
    Color {
        r: 12,
        g: 12,
        b: 12,
        a: 255,
    },
    Color {
        r: 197,
        g: 15,
        b: 31,
        a: 255,
    },
    Color {
        r: 19,
        g: 161,
        b: 14,
        a: 255,
    },
    Color {
        r: 193,
        g: 156,
        b: 0,
        a: 255,
    },
    Color {
        r: 0,
        g: 55,
        b: 218,
        a: 255,
    },
    Color {
        r: 136,
        g: 23,
        b: 152,
        a: 255,
    },
    Color {
        r: 58,
        g: 150,
        b: 221,
        a: 255,
    },
    Color {
        r: 204,
        g: 204,
        b: 204,
        a: 255,
    },
    Color {
        r: 118,
        g: 118,
        b: 118,
        a: 255,
    },
    Color {
        r: 231,
        g: 72,
        b: 86,
        a: 255,
    },
    Color {
        r: 22,
        g: 198,
        b: 12,
        a: 255,
    },
    Color {
        r: 249,
        g: 241,
        b: 165,
        a: 255,
    },
    Color {
        r: 59,
        g: 120,
        b: 255,
        a: 255,
    },
    Color {
        r: 180,
        g: 0,
        b: 158,
        a: 255,
    },
    Color {
        r: 97,
        g: 214,
        b: 214,
        a: 255,
    },
    Color {
        r: 242,
        g: 242,
        b: 242,
        a: 255,
    },
];

/// Default background: fully transparent black.
const DEFAULT_BG: Color = Color {
    r: 0,
    g: 0,
    b: 0,
    a: 0,
};

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
        // Foreground text goes through glyph coverage/blending; keep neutral
        // foregrounds calibrated separately from literal background swatches.
        7 => WINDOWS_TERMINAL_PARITY_DEFAULT_FG,
        8 => Color {
            r: 114,
            g: 114,
            b: 114,
            a: 255,
        },
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
            wrap_pending: false,
            saved_cursor: (0, 0),
            fg: default_fg(),
            bg: default_bg(),
            attrs: CellAttrs::empty(),
            parser: vte::Parser::new(),
            rows,
            cols,
            title: String::new(),
            scrollback: VecDeque::new(),
            scroll_offset: 0,
            alt_grid: None,
            alt_saved_cursor: (0, 0),
            alt_saved_fg: default_fg(),
            alt_saved_bg: default_bg(),
            alt_saved_attrs: CellAttrs::empty(),
            scroll_top: 0,
            scroll_bot: rows,
            pending_response: Vec::new(),
        }
    }

    /// Drain bytes the parser queued as a host-query reply. The bridge
    /// calls this after `process_bytes` to forward the reply over the
    /// PTY back to the running TUI.
    pub fn take_pending_response(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.pending_response)
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

    /// Bottom-anchored reflow on row resize (issue #129). Growing rows
    /// lifts scrollback lines into the new top rows so the live prompt
    /// stays anchored at the bottom; shrinking rows evicts the rows
    /// above the cursor into scrollback so they survive the resize.
    /// Mirrors `unshit_terminal_core::Terminal::resize` so the UI's local
    /// emulator stays consistent with the daemon during a resize
    /// round-trip. Column-only resizes do not touch scrollback.
    pub fn resize(&mut self, rows: usize, cols: usize) {
        let old_rows = self.rows;
        let scroll_region_was_full_screen = self.region_is_full_screen();

        // The bottom-anchored reflow (lift scrollback / evict to
        // scrollback) only applies to the *main* grid: alt screens have
        // no scrollback and no concept of reflow. While the alt screen
        // is active, swap the parked main back in so the reflow runs on
        // the right grid, then swap the alt back into place and resize
        // it as a plain buffer.
        let alt_active = self.alt_grid.is_some();
        if alt_active {
            if let Some(alt) = self.alt_grid.take() {
                let mut prev = alt;
                std::mem::swap(&mut self.grid, &mut prev);
                // `prev` now holds the alt-screen contents.
                self.alt_grid = Some(prev);
            }
        }

        if rows > old_rows {
            let k = rows - old_rows;
            self.grow_rows_lifting_scrollback(k);
            // The cursor on the parked main was saved into
            // `alt_saved_cursor` on entry; bump it so it stays anchored
            // to the same content row after the lift.
            if alt_active {
                self.alt_saved_cursor.0 += k;
            } else {
                self.cursor_row += k;
            }
        } else if rows < old_rows {
            let k = old_rows - rows;
            let cursor_for_eviction = if alt_active {
                self.alt_saved_cursor.0
            } else {
                self.cursor_row
            };
            let evict_above = k.min(cursor_for_eviction);
            if evict_above > 0 {
                self.shrink_rows_evicting_to_scrollback(evict_above);
                if alt_active {
                    self.alt_saved_cursor.0 -= evict_above;
                } else {
                    self.cursor_row -= evict_above;
                }
            }
        }

        self.grid.resize(rows, cols);
        // Swap the alt buffer back on top and resize it plainly.
        if alt_active {
            if let Some(mut main) = self.alt_grid.take() {
                std::mem::swap(&mut self.grid, &mut main);
                // `main` now holds the resized main; `self.grid` holds
                // the alt buffer at its old size. Resize alt and stash
                // the resized main.
                self.grid.resize(rows, cols);
                self.alt_grid = Some(main);
            }
        }

        self.rows = rows;
        self.cols = cols;

        self.clamp_scroll_region_after_resize(rows, scroll_region_was_full_screen);

        // Clamp both the live cursor (alt-screen cursor when alt active,
        // main-screen cursor otherwise) and the parked save slot.
        self.cursor_row = self.cursor_row.min(rows.saturating_sub(1));
        self.cursor_col = self.cursor_col.min(cols.saturating_sub(1));
        self.wrap_pending = false;
        if alt_active {
            self.alt_saved_cursor.0 = self.alt_saved_cursor.0.min(rows.saturating_sub(1));
            self.alt_saved_cursor.1 = self.alt_saved_cursor.1.min(cols.saturating_sub(1));
        }
        self.grid.set_cursor(self.cursor_row, self.cursor_col);
    }

    /// Resize for viewport-driven growth without lifting existing screen
    /// rows downward. During window snap the compositor can deliver several
    /// intermediate sizes and the shell may redraw between them; using the
    /// bottom-anchored scrollback reflow for the final grow leaves that
    /// intermediate redraw floating in the middle of the new viewport.
    ///
    /// This keeps the existing top rows at the top on growth and falls back
    /// to the normal scrollback-preserving resize for shrink/column changes.
    pub fn resize_viewport_growth(&mut self, rows: usize, cols: usize) {
        if rows <= self.rows {
            self.resize(rows, cols);
            return;
        }

        let alt_active = self.alt_grid.is_some();
        let scroll_region_was_full_screen = self.region_is_full_screen();
        self.grid.resize(rows, cols);
        if let Some(alt) = self.alt_grid.as_mut() {
            alt.resize(rows, cols);
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

    /// Lift up to `k` newest scrollback rows into the top of the grid
    /// after extending the row count by `k`. Existing rows shift down
    /// so the bottom of the grid stays anchored to its previous content.
    fn grow_rows_lifting_scrollback(&mut self, k: usize) {
        let cols = self.cols;
        let old_rows = self.rows;
        let split_at = self.scrollback.len().saturating_sub(k);
        let lifted: Vec<Vec<Cell>> = self.scrollback.split_off(split_at).into();

        self.grid.resize(old_rows + k, cols);
        self.grid.shift_rows(k, 0, old_rows);
        for r in 0..k {
            for c in 0..cols {
                self.grid.set_cell(r, c, Cell::default());
            }
        }
        let blank_top = k - lifted.len();
        for (i, row) in lifted.iter().enumerate() {
            let copy = row.len().min(cols);
            for (c, cell) in row.iter().take(copy).enumerate() {
                self.grid.set_cell(blank_top + i, c, *cell);
            }
        }
    }

    /// Push the top `n` grid rows into scrollback (oldest first) and
    /// shift the remaining rows up so the bottom of the grid keeps its
    /// content. `CellGrid::shift_rows` is a `copy_within` and does not
    /// blank the source range, so the duplicated tail rows are clipped
    /// by the subsequent `grid.resize` call in `resize`.
    fn shrink_rows_evicting_to_scrollback(&mut self, n: usize) {
        let cols = self.cols;
        for r in 0..n {
            let row: Vec<Cell> = (0..cols)
                .map(|c| self.grid.get_cell(r, c).copied().unwrap_or_default())
                .collect();
            self.scrollback.push_back(row);
            if self.scrollback.len() > MAX_SCROLLBACK {
                self.scrollback.pop_front();
            }
        }
        self.grid.shift_rows(0, n, self.rows.saturating_sub(n));
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

    /// Overwrite this terminal's rendered state with `snapshot`.
    ///
    /// Used by the daemon attach path: after pulling a snapshot from the
    /// daemon the client rebuilds its local grid so the first frame it
    /// renders matches the authoritative daemon view. The VTE parser,
    /// current SGR state, saved cursor, and title are not touched since
    /// the daemon does not hand those back in slice 4d.
    pub fn apply_snapshot(&mut self, snapshot: &unshit_terminal_core::Snapshot) {
        let rows = snapshot.grid.rows();
        let cols = snapshot.grid.cols();
        if rows != self.rows || cols != self.cols {
            let scroll_region_was_full_screen = self.region_is_full_screen();
            self.rows = rows;
            self.cols = cols;
            self.grid.resize(rows, cols);
            if let Some(alt) = self.alt_grid.as_mut() {
                alt.resize(rows, cols);
            }
            self.clamp_scroll_region_after_resize(rows, scroll_region_was_full_screen);
        }
        for r in 0..rows {
            for c in 0..cols {
                if let Some(cell) = snapshot.grid.get(r, c) {
                    self.grid.set_cell(r, c, core_cell_to_ui(*cell));
                }
            }
        }
        let (cur_row, cur_col) = snapshot.grid.cursor();
        self.cursor_row = cur_row.min(rows.saturating_sub(1));
        self.cursor_col = cur_col.min(cols.saturating_sub(1));
        self.wrap_pending = false;
        self.grid.set_cursor(self.cursor_row, self.cursor_col);
        self.grid.set_cursor_visible(snapshot.grid.cursor_visible());

        self.scrollback.clear();
        self.scrollback.reserve(snapshot.scrollback.len());
        for line in &snapshot.scrollback {
            let converted: Vec<Cell> = line.iter().map(|c| core_cell_to_ui(*c)).collect();
            self.scrollback.push_back(converted);
        }
        self.scroll_offset = 0;
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

    /// `true` when the active scroll region covers the full screen.
    /// Used by `scroll_up` to decide whether the evicted top row should
    /// be pushed into scrollback. A DECSTBM-narrowed region is part of
    /// a TUI's redraw machinery (vim's status line, htop's header), so
    /// rows scrolled off it must NOT pollute scrollback.
    fn region_is_full_screen(&self) -> bool {
        self.scroll_top == 0 && self.scroll_bot == self.rows
    }

    /// Scroll the active region up by one line. When the region covers
    /// the whole screen, the top row is also saved to scrollback before
    /// it scrolls off; with a narrowed region scrollback is left alone.
    fn scroll_up(&mut self) {
        if self.rows == 0 || self.scroll_bot <= self.scroll_top {
            return;
        }
        let top = self.scroll_top;
        let bot = self.scroll_bot;

        // Only the full-screen region feeds scrollback.
        if self.region_is_full_screen() {
            let mut row = Vec::with_capacity(self.cols);
            for col in 0..self.cols {
                row.push(self.grid.get_cell(top, col).copied().unwrap_or_default());
            }
            self.scrollback.push_back(row);
            if self.scrollback.len() > MAX_SCROLLBACK {
                self.scrollback.pop_front();
            }
        }

        // Shift rows [top+1, bot) up into [top, bot-1) and blank row bot-1.
        let move_count = bot - top - 1;
        if move_count > 0 {
            self.grid.shift_rows(top, top + 1, move_count);
        }
        self.clear_row(bot - 1);
    }

    /// Scroll the active region down by one line. The top row of the
    /// region is blanked and the rest of the region shifts down by one;
    /// content above and below the region is untouched.
    ///
    /// Uses `CellGrid::shift_rows` which rotates content, stable line_ids,
    /// and per-row damage together. The line quad cache is keyed on
    /// `(NodeId, line_id)` so the shifted lines replay at their new row
    /// indices without re-emission. `clear_row` blanks the vacated top
    /// row and also resets its `line_id` because the old logical line is
    /// gone, so the cache misses against the blanked content on the next
    /// emit pass (preventing id reuse after the caller rotates content).
    fn scroll_down(&mut self) {
        if self.rows == 0 || self.scroll_bot <= self.scroll_top {
            return;
        }
        let top = self.scroll_top;
        let bot = self.scroll_bot;
        let move_count = bot - top - 1;
        if move_count > 0 {
            // Shift rows [top, bot-1) down into [top+1, bot), then blank top.
            self.grid.shift_rows(top + 1, top, move_count);
        }
        self.clear_row(top);
    }

    /// Fill an entire row with blank cells using the current background
    /// color and assign the row a fresh stable `line_id`. The id reset
    /// keeps caches keyed on line identity honest: the row's logical
    /// line has been discarded, so it should not share identity with the
    /// line that previously occupied it (or with any line that rotated
    /// into a neighboring row via `shift_rows`, which would otherwise
    /// duplicate the vacated row's id).
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
        self.grid.reset_line_identity(row);
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

    /// Enter the alternate screen buffer (DEC private mode 1049 / 47 /
    /// 1047). The current main grid is swapped into `alt_grid` and a
    /// fresh blank grid takes its place. The cursor and SGR state are
    /// captured into dedicated alt-screen save slots so a `?1049l`
    /// later can restore them, regardless of any DECSC/DECRC the TUI
    /// performs while it owns the alt screen.
    ///
    /// Calling this while the alt screen is already active is a no-op:
    /// the original main-screen save slot is preserved.
    fn enter_alt_screen(&mut self) {
        if self.alt_grid.is_some() {
            return;
        }
        self.clear_pending_wrap();
        self.alt_saved_cursor = (self.cursor_row, self.cursor_col);
        self.alt_saved_fg = self.fg;
        self.alt_saved_bg = self.bg;
        self.alt_saved_attrs = self.attrs;

        let mut fresh = CellGrid::new(self.rows, self.cols);
        std::mem::swap(&mut self.grid, &mut fresh);
        self.alt_grid = Some(fresh);

        self.cursor_row = 0;
        self.cursor_col = 0;
        self.clear_pending_wrap();
        self.grid.set_cursor(0, 0);
    }

    /// Exit the alternate screen buffer. Swaps the saved main grid back
    /// into place, restores the cursor and SGR state captured on entry,
    /// and discards everything drawn into the alt buffer (it's the
    /// *alt* buffer; that's the whole point).
    ///
    /// No-op when the alt screen is not active.
    fn exit_alt_screen(&mut self) {
        let Some(mut main) = self.alt_grid.take() else {
            return;
        };
        std::mem::swap(&mut self.grid, &mut main);
        // `main` now holds the discarded alt-buffer contents and is
        // dropped here.

        self.cursor_row = self.alt_saved_cursor.0.min(self.rows.saturating_sub(1));
        self.cursor_col = self.alt_saved_cursor.1.min(self.cols.saturating_sub(1));
        self.clear_pending_wrap();
        self.fg = self.alt_saved_fg;
        self.bg = self.alt_saved_bg;
        self.attrs = self.alt_saved_attrs;
        self.grid.set_cursor(self.cursor_row, self.cursor_col);
    }

    /// Write a character at the current cursor position with the current
    /// attributes, then advance the cursor.
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
            wide_continuation: false,
        };
        self.grid.set_cell(self.cursor_row, self.cursor_col, cell);
        if self.cursor_col + 1 >= self.cols {
            self.cursor_col = self.cols - 1;
            self.wrap_pending = true;
        } else {
            self.cursor_col += 1;
            self.wrap_pending = false;
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
                t.clear_pending_wrap();
                // If the cursor is sitting on the last row of the active
                // scroll region, LF scrolls the region instead of moving
                // past it. Outside the region, LF just advances the cursor
                // and clamps to the screen bottom (no scroll, since DECSTBM
                // pins the rest of the screen).
                if t.cursor_row + 1 == t.scroll_bot {
                    t.scroll_up();
                } else if t.cursor_row + 1 < t.rows {
                    t.cursor_row += 1;
                } else {
                    // Outside the region and at the absolute bottom: stay
                    // put. The region pins everything below it.
                    t.cursor_row = t.rows.saturating_sub(1);
                }
            }
            // Carriage Return
            0x0D => {
                t.clear_pending_wrap();
                t.cursor_col = 0;
            }
            // Horizontal Tab
            0x09 => {
                t.clear_pending_wrap();
                let next_tab = (t.cursor_col / 8 + 1) * 8;
                t.cursor_col = next_tab.min(t.cols.saturating_sub(1));
            }
            // Backspace
            0x08 => {
                t.clear_pending_wrap();
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

        let preserves_pending_wrap = matches!(action, 'm' | 'c' | 'n' | 'q')
            || (intermediates == [b'?'] && matches!(action, 'h' | 'l'));
        if !preserves_pending_wrap {
            t.clear_pending_wrap();
        }

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
            // down by `n`, then blanks the newly exposed rows. Because
            // `shift_rows` rotates stable `line_id`s alongside the cells
            // (issue #52 Step 3), the retained line quad cache replays the
            // shifted lines at their new row indices without re-emission.
            // `clear_row` resets line identity for the blanked rows so the
            // cache misses against the empty content.
            'L' => {
                let n = p0();
                let cursor_row = t.cursor_row;
                // IL is a no-op when the cursor is outside the active
                // scroll region. Inside the region, it scrolls the
                // sub-range `[cursor_row, scroll_bot)` down by `n`.
                if cursor_row < t.scroll_top || cursor_row >= t.scroll_bot {
                    // ignored
                } else {
                    let bot = t.scroll_bot;
                    let n = n.min(bot.saturating_sub(cursor_row));
                    if n > 0 {
                        let move_count = bot.saturating_sub(cursor_row + n);
                        if move_count > 0 {
                            t.grid.shift_rows(cursor_row + n, cursor_row, move_count);
                        }
                        for row in cursor_row..cursor_row + n {
                            t.clear_row(row);
                        }
                    }
                }
            }
            // DL: Delete Lines
            //
            // Uses `CellGrid::shift_rows` to move rows below the cursor up
            // by `n`, then blanks the newly exposed rows at the bottom.
            // Because `shift_rows` rotates stable `line_id`s alongside the
            // cells (issue #52 Step 3), the retained line quad cache
            // replays the shifted lines at their new row indices without
            // re-emission. `clear_row` resets line identity for the
            // blanked rows so the cache misses against the empty content.
            'M' if intermediates.is_empty() => {
                let n = p0();
                let cursor_row = t.cursor_row;
                // DL is a no-op when the cursor is outside the active
                // scroll region. Inside the region, it scrolls the
                // sub-range `[cursor_row, scroll_bot)` up by `n`.
                if cursor_row < t.scroll_top || cursor_row >= t.scroll_bot {
                    // ignored
                } else {
                    let bot = t.scroll_bot;
                    let n = n.min(bot.saturating_sub(cursor_row));
                    if n > 0 {
                        let move_count = bot.saturating_sub(cursor_row + n);
                        if move_count > 0 {
                            t.grid.shift_rows(cursor_row, cursor_row + n, move_count);
                        }
                        for row in bot.saturating_sub(n)..bot {
                            t.clear_row(row);
                        }
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
            // ECH: Erase Characters (blank cells in place; cursor does not move)
            'X' => {
                let n = p0().min(t.cols.saturating_sub(t.cursor_col));
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

            // -- SGR: Select Graphic Rendition ---------------------------------
            'm' => {
                handle_sgr(t, &pv);
            }

            // -- DECSTBM: Set Top/Bottom Scroll Margins ------------------------
            //
            // `CSI <top>;<bot> r`. Params are 1-based, half-open after
            // conversion: the active region is rows `[top-1, bot)`. With
            // no params (or zero), the region resets to the full screen.
            // Origin mode (DECOM) is intentionally not implemented; the
            // cursor is moved to absolute (0, 0) on success regardless,
            // matching xterm's "non-origin" behaviour.
            'r' if intermediates.is_empty() => {
                let rows = t.rows;
                let top_1 = pv.first().copied().unwrap_or(0);
                let bot_1 = pv.get(1).copied().unwrap_or(0);
                let top = if top_1 == 0 { 1 } else { top_1 as usize };
                let bot = if bot_1 == 0 { rows } else { bot_1 as usize };
                // Convert to half-open [scroll_top, scroll_bot).
                let new_top = top.saturating_sub(1);
                let new_bot = bot.min(rows);
                // Spec: must satisfy `top < bot` and both within bounds.
                // Invalid params leave the existing region intact.
                if new_top < new_bot && new_bot <= rows {
                    t.scroll_top = new_top;
                    t.scroll_bot = new_bot;
                    t.cursor_row = 0;
                    t.cursor_col = 0;
                }
            }

            // -- DEC private mode set/reset (CSI ? Pn h / l) -------------------
            //
            // We currently care about the alt-screen modes only:
            //   1049: save cursor + switch to alt screen (combined op)
            //   1047: switch to alt screen without explicit save (legacy)
            //     47: ditto, the original DEC alt-screen mode
            //
            // `?1049h/l` is the canonical "this is a TUI app" sequence
            // emitted by xterm-derived clients. We forward all three
            // variants to the same handler since many TUIs still send
            // the older aliases. Other private modes (mouse reporting,
            // bracketed paste, application keypad, etc.) are ignored
            // here; the daemon owns those semantics.
            'h' if intermediates == [b'?'] => {
                for &mode in &pv {
                    if matches!(mode, 47 | 1047 | 1049) {
                        t.enter_alt_screen();
                    }
                }
            }
            'l' if intermediates == [b'?'] => {
                for &mode in &pv {
                    if matches!(mode, 47 | 1047 | 1049) {
                        t.exit_alt_screen();
                    }
                }
            }

            // -- Host queries --------------------------------------------------
            //
            // TUIs (Claude Code, fzf, etc) probe the terminal to decide
            // which rendering path to use. With no reply they assume a
            // primitive terminal and fall back to a minimal layout (e.g.
            // Claude renders a 4-row bordered input box with the prompt
            // glyph on a row of its own instead of a single `> ABC|`
            // line). Replies are queued in `pending_response`; the
            // bridge drains and writes them back to the PTY.
            //
            // DA1 - Primary Device Attributes: `CSI c` or `CSI 0 c`.
            // Reply `CSI ? 1 ; 2 c` advertises VT100 with advanced video
            // option, the same baseline xterm reports.
            'c' if intermediates.is_empty() => {
                t.pending_response.extend_from_slice(b"\x1b[?1;2c");
            }
            // DA2 - Secondary Device Attributes: `CSI > c` or `CSI > 0 c`.
            // Reply `CSI > 0 ; 95 ; 0 c` mirrors xterm patch 95.
            'c' if intermediates == [b'>'] => {
                t.pending_response.extend_from_slice(b"\x1b[>0;95;0c");
            }
            // DSR - Device Status Report: `CSI 5 n` "are you ok?".
            // Reply `CSI 0 n` = ok.
            'n' if intermediates.is_empty() && pv.first() == Some(&5) => {
                t.pending_response.extend_from_slice(b"\x1b[0n");
            }
            // CPR - Cursor Position Report: `CSI 6 n`. Reply with the
            // current 1-indexed cursor position.
            'n' if intermediates.is_empty() && pv.first() == Some(&6) => {
                let row = t.cursor_row + 1;
                let col = t.cursor_col + 1;
                let reply = format!("\x1b[{};{}R", row, col);
                t.pending_response.extend_from_slice(reply.as_bytes());
            }
            // XTVERSION - terminal name and version: `CSI > q` or
            // `CSI > 0 q`. Claude Code sends this before deciding
            // between compact and bordered layouts. Reply with a DCS
            // string `DCS > | name version ST` so the probe succeeds
            // and Claude picks the rich path.
            'q' if intermediates == [b'>'] => {
                t.pending_response
                    .extend_from_slice(b"\x1bP>|godly-terminal 0.1\x1b\\");
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
                t.clear_pending_wrap();
                t.cursor_row = t.saved_cursor.0.min(t.rows.saturating_sub(1));
                t.cursor_col = t.saved_cursor.1.min(t.cols.saturating_sub(1));
            }
            // RI: Reverse Index (move cursor up; scroll down if at top)
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
            5 => t.attrs |= CellAttrs::BLINK,
            7 => t.attrs |= CellAttrs::INVERSE,
            9 => t.attrs |= CellAttrs::STRIKETHROUGH,

            // Unset attribute flags.
            22 => t.attrs &= !(CellAttrs::BOLD | CellAttrs::DIM),
            23 => t.attrs &= !CellAttrs::ITALIC,
            24 => t.attrs &= !CellAttrs::UNDERLINE,
            25 => t.attrs &= !CellAttrs::BLINK,
            27 => t.attrs &= !CellAttrs::INVERSE,
            29 => t.attrs &= !CellAttrs::STRIKETHROUGH,

            // Standard foreground colors (30..37).
            30..=37 => {
                t.fg = ansi_16_fg_color((code - 30) as usize);
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
                                t.fg = fg_color_256_for_parity(pv[i] as u8);
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
                t.fg = default_fg();
            }

            // Standard background colors (40..47).
            40..=47 => {
                t.bg = ansi_16_color((code - 40) as usize);
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
                                t.bg = bg_color_256_for_parity(pv[i] as u8);
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
                t.bg = default_bg();
            }

            // Bright foreground colors (90..97).
            90..=97 => {
                t.fg = ansi_16_fg_color((code - 90 + 8) as usize);
            }
            // Bright background colors (100..107).
            100..=107 => {
                t.bg = ansi_16_color((code - 100 + 8) as usize);
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
    t.fg = default_fg();
    t.bg = default_bg();
    t.attrs = CellAttrs::empty();
}

/// Convert a cell from the shared `unshit-terminal-core` shape to the
/// UI framework's `cell_grid::Cell`. Both types carry identical
/// per-channel color bytes and the same attribute bit layout, so this
/// is a field-for-field copy. `wide_continuation` is always reset to
/// `false` because the core snapshot does not carry that flag in slice
/// 4d; a future slice may propagate it.
fn core_cell_to_ui(core: unshit_terminal_core::Cell) -> Cell {
    Cell {
        ch: core.ch,
        fg: Color {
            r: core.fg.r,
            g: core.fg.g,
            b: core.fg.b,
            a: core.fg.a,
        },
        bg: Color {
            r: core.bg.r,
            g: core.bg.g,
            b: core.bg.b,
            a: core.bg.a,
        },
        attrs: CellAttrs::from_bits_truncate(core.attrs.bits()),
        wide_continuation: false,
    }
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
    fn erase_characters_blanks_without_moving_cursor_or_shifting() {
        let mut t = Terminal::new(1, 10);
        t.process_bytes(b"> abcdef");
        t.process_bytes(b"\x1b[1;3H"); // editable input starts after "> "
        t.process_bytes(b"\x1b[4X"); // ECH: blank four cells in place

        assert_eq!(t.cursor_position(), (0, 2));
        assert_eq!(row_text(&t, 0), ">     ef");
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
        // Bottom-anchored (issue #129): the cursor's distance from the
        // bottom is preserved across shrink, so row 7 of 10 (distance 2)
        // becomes row 2 of 5 (distance 2). Col is clamped to new bounds.
        assert_eq!(t.cursor_position(), (2, 9));
    }

    // -- Resize reflow (issue #129) -------------------------------------------
    //
    // Bottom-anchored: grow lifts scrollback into the new top rows; shrink
    // evicts top rows into scrollback. Mirrors the daemon-side behavior so
    // the local UI replay stays consistent with the authoritative grid.

    #[test]
    fn resize_grow_lifts_scrollback_into_new_top_rows_ui() {
        // Issue #129 regression. Short content (2 chars in 4-col grid) so
        // we don't trigger the eager wrap-scroll path.
        let mut t = Terminal::new(2, 4);
        t.process_bytes(b"L1\r\nL2\r\nL3\r\nL4\r\n>>");
        assert_eq!(t.scrollback_len(), 3);
        assert_eq!(row_text(&t, 0), "L4");
        assert_eq!(row_text(&t, 1), ">>");
        let cursor_before = t.cursor_position();

        t.resize(4, 4);

        assert_eq!(t.rows, 4);
        assert_eq!(row_text(&t, 0), "L2");
        assert_eq!(row_text(&t, 1), "L3");
        assert_eq!(row_text(&t, 2), "L4");
        assert_eq!(row_text(&t, 3), ">>");
        assert_eq!(t.scrollback_len(), 1);
        assert_eq!(t.cursor_position().0, cursor_before.0 + 2);
    }

    #[test]
    fn resize_grow_with_empty_scrollback_blanks_top_and_advances_cursor_ui() {
        let mut t = Terminal::new(2, 3);
        t.process_bytes(b"aa\r\nbb");
        assert_eq!(t.cursor_position().0, 1);
        assert_eq!(t.scrollback_len(), 0);

        t.resize(4, 3);

        assert_eq!(row_text(&t, 0), "");
        assert_eq!(row_text(&t, 1), "");
        assert_eq!(row_text(&t, 2), "aa");
        assert_eq!(row_text(&t, 3), "bb");
        assert_eq!(t.cursor_position().0, 3);
    }

    #[test]
    fn resize_viewport_growth_keeps_existing_rows_at_top_ui() {
        let mut t = Terminal::new(2, 3);
        t.process_bytes(b"aa\r\nbb");
        let cursor_before = t.cursor_position();

        t.resize_viewport_growth(4, 3);

        assert_eq!(t.rows, 4);
        assert_eq!(row_text(&t, 0), "aa");
        assert_eq!(row_text(&t, 1), "bb");
        assert_eq!(row_text(&t, 2), "");
        assert_eq!(row_text(&t, 3), "");
        assert_eq!(t.cursor_position(), cursor_before);
    }

    #[test]
    fn resize_viewport_growth_expands_full_screen_scroll_region_for_redraw_ui() {
        let mut t = Terminal::new(2, 10);

        t.resize_viewport_growth(4, 10);

        assert_eq!((t.scroll_top, t.scroll_bot), (0, 4));
        t.process_bytes(b"\x1b[Hone\x1b[K\r\ntwo\x1b[K\r\n\x1b[K\r\n\x1b[K\x1b[2;4H");

        assert_eq!(row_text(&t, 0), "one");
        assert_eq!(row_text(&t, 1), "two");
        assert_eq!(row_text(&t, 2), "");
        assert_eq!(row_text(&t, 3), "");
    }

    #[test]
    fn resize_shrink_pushes_top_rows_to_scrollback_ui() {
        let mut t = Terminal::new(4, 3);
        t.process_bytes(b"aa\r\nbb\r\ncc\r\ndd");
        assert_eq!(t.cursor_position().0, 3);
        assert_eq!(t.scrollback_len(), 0);

        t.resize(2, 3);

        assert_eq!(t.rows, 2);
        assert_eq!(row_text(&t, 0), "cc");
        assert_eq!(row_text(&t, 1), "dd");
        assert_eq!(t.cursor_position().0, 1);
        assert_eq!(t.scrollback_len(), 2);
        assert_eq!(t.scrollback[0][0].ch, 'a');
        assert_eq!(t.scrollback[1][0].ch, 'b');
    }

    #[test]
    fn resize_shrink_when_k_exceeds_cursor_row_evicts_only_above_cursor_ui() {
        let mut t = Terminal::new(5, 3);
        t.process_bytes(b"aa\r\nXX");
        assert_eq!(t.cursor_position().0, 1);

        t.resize(1, 3);

        assert_eq!(t.rows, 1);
        assert_eq!(row_text(&t, 0), "XX");
        assert_eq!(t.cursor_position().0, 0);
        assert_eq!(t.scrollback_len(), 1);
        assert_eq!(t.scrollback[0][0].ch, 'a');
    }

    #[test]
    fn resize_round_trip_is_stable_ui() {
        let mut t = Terminal::new(4, 3);
        t.process_bytes(b"aa\r\nbb\r\ncc\r\ndd");
        let cursor_before = t.cursor_position();

        t.resize(2, 3);
        t.resize(4, 3);

        assert_eq!(row_text(&t, 0), "aa");
        assert_eq!(row_text(&t, 1), "bb");
        assert_eq!(row_text(&t, 2), "cc");
        assert_eq!(row_text(&t, 3), "dd");
        assert_eq!(t.cursor_position(), cursor_before);
    }

    #[test]
    fn resize_column_only_does_not_touch_scrollback_ui() {
        let mut t = Terminal::new(2, 3);
        t.process_bytes(b"aa\r\nbb\r\ncc");
        let scrollback_before = t.scrollback_len();

        t.resize(2, 5);

        assert_eq!(t.scrollback_len(), scrollback_before);
        assert_eq!(t.cols, 5);
    }

    #[test]
    fn resize_shrink_respects_max_scrollback_ui() {
        // The UI mirror's eviction loop uses its own VecDeque cap check
        // (MAX_SCROLLBACK = 10_000) instead of the daemon's `Scrollback`
        // type. We can't shrink that cap from the test, so just verify
        // overflow handling does the right thing on a small synthetic
        // case by pre-filling scrollback up to the cap and then evicting
        // one more row via resize.
        let mut t = Terminal::new(2, 4);
        // Pre-load scrollback to MAX_SCROLLBACK by directly pushing.
        for _ in 0..MAX_SCROLLBACK {
            t.scrollback.push_back(vec![Cell::default(); 4]);
        }
        // Now write a real row so the grid has identifiable content.
        t.process_bytes(b"AB\r\nCD");
        assert_eq!(t.scrollback_len(), MAX_SCROLLBACK);

        t.resize(1, 4);

        // The shrink evicted one row to scrollback; cap forces an
        // equivalent pop_front so length stays at MAX_SCROLLBACK.
        assert_eq!(t.scrollback_len(), MAX_SCROLLBACK);
        // The newest scrollback entry is the row we just evicted ("AB").
        let newest = t.scrollback.back().unwrap();
        assert_eq!(newest[0].ch, 'A');
        assert_eq!(newest[1].ch, 'B');
    }

    #[test]
    fn resize_to_zero_rows_does_not_panic_ui() {
        let mut t = Terminal::new(2, 3);
        t.process_bytes(b"aa\r\nbb");
        t.resize(0, 3);
        assert_eq!(t.rows, 0);
        assert_eq!(t.cursor_position().0, 0);
    }

    #[test]
    fn resize_no_blank_gap_after_split_unsplit_round_trip_ui() {
        let mut t = Terminal::new(6, 8);
        t.process_bytes(b"row1__\r\nrow2__\r\nrow3__\r\nrow4__\r\nrow5__\r\n>>>>>");
        let cursor_before = t.cursor_position();

        t.resize(4, 8);
        assert_eq!(row_text(&t, 3), ">>>>>");

        t.resize(6, 8);

        assert_eq!(row_text(&t, 0), "row1__");
        assert_eq!(row_text(&t, 1), "row2__");
        assert_eq!(row_text(&t, 2), "row3__");
        assert_eq!(row_text(&t, 3), "row4__");
        assert_eq!(row_text(&t, 4), "row5__");
        assert_eq!(row_text(&t, 5), ">>>>>");
        assert_eq!(t.cursor_position(), cursor_before);
        for r in 0..5 {
            assert_ne!(row_text(&t, r), "", "row {r} unexpectedly blank");
        }
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
    fn parity_default_fg_uses_capture_calibrated_windows_terminal_value() {
        assert_eq!(
            default_fg_for_parity(true),
            Color {
                r: 196,
                g: 196,
                b: 196,
                a: 255,
            }
        );
        assert_eq!(default_fg_for_parity(false), DEFAULT_FG);
    }

    #[test]
    fn parity_ansi_palette_matches_windows_terminal_campbell() {
        assert_eq!(
            ansi_16_color_for_parity(1, true),
            Color {
                r: 197,
                g: 15,
                b: 31,
                a: 255,
            }
        );
        assert_eq!(ansi_16_color_for_parity(1, false), ANSI_16[1]);
    }

    #[test]
    fn parity_neutral_foregrounds_are_capture_calibrated_separately_from_backgrounds() {
        assert_eq!(
            ansi_16_fg_color_for_parity(7, true),
            Color {
                r: 196,
                g: 196,
                b: 196,
                a: 255,
            }
        );
        assert_eq!(
            ansi_16_fg_color_for_parity(8, true),
            Color {
                r: 114,
                g: 114,
                b: 114,
                a: 255,
            }
        );
        assert_eq!(
            ansi_16_color_for_parity(7, true),
            Color {
                r: 204,
                g: 204,
                b: 204,
                a: 255,
            },
            "background swatches should keep the literal Campbell palette"
        );
    }

    #[test]
    fn parity_256_color_low_indices_follow_ansi_profile_mapping() {
        assert_eq!(
            fg_color_256_for_parity_with_profile(7, true),
            Color {
                r: 196,
                g: 196,
                b: 196,
                a: 255,
            }
        );
        assert_eq!(
            bg_color_256_for_parity_with_profile(7, true),
            Color {
                r: 204,
                g: 204,
                b: 204,
                a: 255,
            }
        );
        assert_eq!(
            fg_color_256_for_parity_with_profile(8, true),
            Color {
                r: 114,
                g: 114,
                b: 114,
                a: 255,
            }
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
    fn sgr_blink_toggles_blink_attr() {
        let mut t = Terminal::new(3, 20);
        t.process_bytes(b"\x1b[5mB");
        let blinking = t.grid.get_cell(0, 0).unwrap();
        assert!(blinking.attrs.contains(CellAttrs::BLINK));

        t.process_bytes(b"\x1b[25mX");
        let steady = t.grid.get_cell(0, 1).unwrap();
        assert!(!steady.attrs.contains(CellAttrs::BLINK));
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
    fn full_width_line_delays_wrap_until_next_printable() {
        let mut term = Terminal::new(2, 5);
        term.process_bytes(b"ABCDE");
        // Real terminals use delayed autowrap: a character written into
        // the last column leaves the cursor there until the next printable
        // character arrives. Claude Code draws full-width rules/status
        // rows; eager wrapping creates phantom rows that make input drift
        // into previously-rendered output.
        assert_eq!(term.cursor_position(), (0, 4));
        assert_eq!(row_text(&term, 0), "ABCDE");
        assert_eq!(row_text(&term, 1), "");

        term.process_bytes(b"F");

        assert_eq!(term.cursor_position(), (1, 1));
        assert_eq!(row_text(&term, 0), "ABCDE");
        assert_eq!(row_text(&term, 1), "F");
    }

    #[test]
    fn carriage_return_clears_pending_wrap_after_full_width_line() {
        let mut term = Terminal::new(2, 5);
        term.process_bytes(b"ABCDE\rZ");

        assert_eq!(term.cursor_position(), (0, 1));
        assert_eq!(row_text(&term, 0), "ZBCDE");
        assert_eq!(row_text(&term, 1), "");
    }

    #[test]
    fn terminal_query_preserves_pending_wrap() {
        let mut term = Terminal::new(2, 5);
        term.process_bytes(b"ABCDE\x1b[6nF");

        assert_eq!(term.cursor_position(), (1, 1));
        assert_eq!(row_text(&term, 0), "ABCDE");
        assert_eq!(row_text(&term, 1), "F");
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
        // Bottom-anchored (issue #129): row 19 of 24 (distance 4 from
        // bottom) becomes row 5 of 10 (distance 4). Col is clamped.
        assert_eq!(term.cursor_position(), (5, 39));
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

    // -- Line-damage / line-identity regression tests (issues #62, #63) ------
    //
    // PR #62 initially full-damaged every row on scroll_up because the line
    // quad cache was keyed on `(NodeId, row_index)`: a scrolled row's
    // content moved to a different index and the cache at that index went
    // stale. PR #70 extended the same full-damage invariant to scroll_down,
    // Insert Lines, and Delete Lines.
    //
    // Issue #52 Step 3 re-keys the line quad cache on stable `line_id`
    // (Kitty `linebuf_index`, Ghostty `PageList`, WezTerm Line appdata).
    // The shifted lines keep their identity, so the cache replays them at
    // their new row indices without re-emission. The PR #62 / #70 full-
    // damage invariants are therefore relaxed: only the rows whose
    // logical line has actually been discarded (the vacated row that the
    // caller blanks via `clear_row`) receive fresh damage and a new id.

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

    #[test]
    fn scroll_down_rotates_line_ids_and_damages_only_new_row_issue_63_non_regression() {
        // Non-regression guard for issue #63. After Step 3, Terminal::
        // scroll_down rotates stable `line_id`s and per-row damage with
        // the content. Only the newly cleared top row (row 0) gets a
        // fresh id and fully damaged state; the shifted lines keep their
        // identities and their clean damage. The original #63 symptom
        // (cache replay at stale row index) cannot return because the
        // cache is no longer keyed on row index.
        let mut term = Terminal::new(4, 5);
        fill_and_clean_damage(&mut term);
        let ids_before: Vec<u64> = term.grid.line_ids().to_vec();

        term.scroll_down();

        // Shifted rows 1..rows inherit identity from source rows 0..rows-1.
        for row in 1..term.rows {
            assert_eq!(
                term.grid.line_id(row),
                Some(ids_before[row - 1]),
                "row {row} must inherit line_id from source row {}",
                row - 1,
            );
            // Shifted rows are clean: content is unchanged from the line's
            // perspective (same logical line, different row index).
            assert!(
                term.grid.line_damage()[row].is_clean(),
                "row {row} must remain clean; line content did not change",
            );
        }
        // Row 0 gets a fresh id (the cleared vacated row).
        let new_top = term.grid.line_id(0).unwrap();
        assert!(
            !ids_before.contains(&new_top),
            "new top row id {new_top} must not collide with pre-shift ids {ids_before:?}",
        );
    }

    #[test]
    fn insert_lines_rotates_line_ids_and_damages_only_new_rows() {
        // Step 3: CSI L (Insert Lines) at cursor row 1 rotates rows 1..3
        // down into rows 3..5 and blanks rows 1..3 (the freshly inserted
        // blank lines). Shifted rows keep their identity; inserted blank
        // rows get fresh ids.
        let mut term = Terminal::new(5, 6);
        term.process_bytes(b"\x1b[2;1H"); // move to row 1, col 0 (1-indexed)
        fill_and_clean_damage(&mut term);
        let ids_before: Vec<u64> = term.grid.line_ids().to_vec();

        term.process_bytes(b"\x1b[2L"); // Insert 2 blank lines at cursor

        // Rows 3 and 4 inherit identity from rows 1 and 2 (shifted down
        // by 2). Rows 1 and 2 (the inserted blanks) have fresh ids.
        assert_eq!(term.grid.line_id(3), Some(ids_before[1]));
        assert_eq!(term.grid.line_id(4), Some(ids_before[2]));
        for inserted_row in 1..=2 {
            let new_id = term.grid.line_id(inserted_row).unwrap();
            assert!(
                !ids_before.contains(&new_id),
                "inserted row {inserted_row} must have a fresh id",
            );
        }
    }

    #[test]
    fn delete_lines_rotates_line_ids_and_damages_only_new_rows() {
        // Step 3: CSI M (Delete Lines) at cursor row 1 rotates rows 3..5
        // up into rows 1..3 and blanks rows 3..5 (the freshly vacated
        // bottom lines). Shifted rows keep their identity; vacated rows
        // get fresh ids.
        let mut term = Terminal::new(5, 6);
        term.process_bytes(b"\x1b[2;1H"); // move to row 1, col 0 (1-indexed)
        fill_and_clean_damage(&mut term);
        let ids_before: Vec<u64> = term.grid.line_ids().to_vec();

        term.process_bytes(b"\x1b[2M"); // Delete 2 lines at cursor

        // Rows 1 and 2 inherit identity from rows 3 and 4.
        assert_eq!(term.grid.line_id(1), Some(ids_before[3]));
        assert_eq!(term.grid.line_id(2), Some(ids_before[4]));
        // Rows 3 and 4 (vacated) have fresh ids.
        for vacated_row in 3..=4 {
            let new_id = term.grid.line_id(vacated_row).unwrap();
            assert!(
                !ids_before.contains(&new_id),
                "vacated row {vacated_row} must have a fresh id",
            );
        }
    }

    #[test]
    fn reverse_index_at_top_rotates_line_ids_and_damages_only_new_row() {
        // Step 3: ESC M (Reverse Index) at the top of the screen piggybacks
        // on Terminal::scroll_down, so it inherits the line-identity
        // rotation: only the new top row gets a fresh id.
        let mut term = Terminal::new(4, 5);
        term.process_bytes(b"\x1b[1;1H"); // move cursor to row 0
        fill_and_clean_damage(&mut term);
        let ids_before: Vec<u64> = term.grid.line_ids().to_vec();

        term.process_bytes(b"\x1bM"); // Reverse Index

        for row in 1..term.rows {
            assert_eq!(
                term.grid.line_id(row),
                Some(ids_before[row - 1]),
                "row {row} must inherit line_id from source row {}",
                row - 1,
            );
        }
        let new_top = term.grid.line_id(0).unwrap();
        assert!(
            !ids_before.contains(&new_top),
            "new top row id {new_top} must not collide with pre-shift ids",
        );
    }

    // -- apply_snapshot -------------------------------------------------------

    #[test]
    fn apply_snapshot_replaces_grid_and_cursor() {
        use unshit_terminal_core::Terminal as CoreTerminal;

        let mut core = CoreTerminal::new(3, 5, 10);
        core.process_bytes(b"hi\r\nyo");
        let snap = core.snapshot(10);

        let mut ui = Terminal::new(3, 5);
        ui.apply_snapshot(&snap);

        assert_eq!(cell_char(&ui, 0, 0), 'h');
        assert_eq!(cell_char(&ui, 0, 1), 'i');
        assert_eq!(cell_char(&ui, 1, 0), 'y');
        assert_eq!(cell_char(&ui, 1, 1), 'o');
        let (core_row, core_col) = snap.grid.cursor();
        assert_eq!(ui.cursor_position(), (core_row, core_col));
    }

    #[test]
    fn apply_snapshot_replaces_scrollback() {
        use unshit_terminal_core::Terminal as CoreTerminal;

        let mut core = CoreTerminal::new(2, 4, 100);
        core.process_bytes(b"aaaa\r\nbbbb\r\ncccc\r\ndddd");
        let snap = core.snapshot(100);
        assert!(!snap.scrollback.is_empty(), "fixture should have scrolled");

        let mut ui = Terminal::new(2, 4);
        ui.apply_snapshot(&snap);

        assert_eq!(ui.scrollback_len(), snap.scrollback.len());
        let first_line = &snap.scrollback[0];
        let first_ch = first_line[0].ch;
        assert_eq!(ui.scrollback[0][0].ch, first_ch);
    }

    #[test]
    fn apply_snapshot_translates_colors_and_attrs() {
        use unshit_terminal_core::Terminal as CoreTerminal;

        let mut core = CoreTerminal::new(1, 3, 10);
        core.process_bytes(b"\x1b[31;1mA\x1b[0m");
        let snap = core.snapshot(0);

        let mut ui = Terminal::new(1, 3);
        ui.apply_snapshot(&snap);

        let cell = ui.grid().get_cell(0, 0).expect("cell (0,0) must exist");
        assert!(
            cell.attrs.contains(CellAttrs::BOLD),
            "bold attribute must survive translation, got {:?}",
            cell.attrs
        );
        assert_eq!(
            cell.fg, ANSI_16[1],
            "red (SGR 31) foreground must map to UI ANSI_16[1]"
        );
    }

    #[test]
    fn apply_snapshot_resizes_grid_if_dimensions_differ() {
        use unshit_terminal_core::Terminal as CoreTerminal;

        let core = CoreTerminal::new(3, 5, 10);
        let snap = core.snapshot(0);

        let mut ui = Terminal::new(5, 10);
        assert_eq!(ui.rows, 5);
        assert_eq!(ui.cols, 10);

        ui.apply_snapshot(&snap);

        assert_eq!(ui.rows, 3);
        assert_eq!(ui.cols, 5);
        assert_eq!(ui.grid().rows(), 3);
        assert_eq!(ui.grid().cols(), 5);
    }

    // -- Alt screen buffer (DEC private mode 1049 / 47 / 1047) ----------------
    //
    // These cover the entry/exit invariants documented in the issue:
    //   - cursor resets to (0, 0) on entry, original screen preserved
    //   - cursor restored, alt content discarded on exit
    //   - SGR state survives the round trip
    //   - resize while in alt screen sizes both buffers
    //   - the older `?47` and `?1047` aliases route to the same handler

    #[test]
    fn alt_screen_enter_resets_cursor_and_preserves_main_content() {
        let mut t = Terminal::new(4, 6);
        t.process_bytes(b"hello\r\nworld");
        let cursor_before = t.cursor_position();
        assert_eq!(row_text(&t, 0), "hello");

        t.process_bytes(b"\x1b[?1049h"); // enter alt screen

        // Cursor reset to (0, 0).
        assert_eq!(t.cursor_position(), (0, 0));
        // Alt grid is now active and is blank.
        for row in 0..t.rows {
            assert_eq!(row_text(&t, row), "");
        }
        // The original main grid is parked in alt_grid with its content.
        assert!(t.alt_grid.is_some());
        let stashed = t.alt_grid.as_ref().unwrap();
        assert_eq!(stashed.get_cell(0, 0).unwrap().ch, 'h');
        // The pre-entry cursor lives in the dedicated save slot.
        assert_eq!(t.alt_saved_cursor, cursor_before);
    }

    #[test]
    fn alt_screen_exit_restores_main_screen_and_cursor() {
        let mut t = Terminal::new(4, 6);
        t.process_bytes(b"hello\r\nworld");
        let cursor_before = t.cursor_position();

        t.process_bytes(b"\x1b[?1049h");
        // Draw something into the alt screen.
        t.process_bytes(b"ALT");
        assert_eq!(row_text(&t, 0), "ALT");

        t.process_bytes(b"\x1b[?1049l"); // exit alt screen

        // Main screen content is back.
        assert_eq!(row_text(&t, 0), "hello");
        assert_eq!(row_text(&t, 1), "world");
        // Cursor restored to its pre-entry slot.
        assert_eq!(t.cursor_position(), cursor_before);
        // alt_grid slot is empty again.
        assert!(t.alt_grid.is_none());
    }

    #[test]
    fn alt_screen_discards_alt_buffer_drawing_on_exit() {
        let mut t = Terminal::new(3, 5);
        t.process_bytes(b"\x1b[?1049h");
        t.process_bytes(b"XXXXX");
        // A clean exit + re-entry should give a blank alt buffer.
        t.process_bytes(b"\x1b[?1049l"); // exit, alt content tossed
        t.process_bytes(b"\x1b[?1049h"); // re-enter

        // CellGrid::new fills with Cell::default() whose char is the
        // null byte (the framework's "uninitialized" marker), not space.
        // What matters is that the previous 'X's are gone.
        let blank_ch = Cell::default().ch;
        for col in 0..t.cols {
            let cell = t.grid.get_cell(0, col).unwrap();
            assert_eq!(
                cell.ch, blank_ch,
                "alt buffer must be blank on re-entry, found {:?}",
                cell.ch
            );
        }
    }

    #[test]
    fn alt_screen_preserves_sgr_state_across_round_trip() {
        let mut t = Terminal::new(3, 5);
        // Bold + red foreground on the main screen.
        t.process_bytes(b"\x1b[1;31m");
        let main_fg = t.fg;
        let main_attrs = t.attrs;
        assert!(main_attrs.contains(CellAttrs::BOLD));

        // Enter alt screen and clobber the SGR state in there.
        t.process_bytes(b"\x1b[?1049h");
        t.process_bytes(b"\x1b[0m\x1b[34m"); // reset + blue
        assert!(!t.attrs.contains(CellAttrs::BOLD));

        // Exit must restore the main-screen SGR slot byte-for-byte.
        t.process_bytes(b"\x1b[?1049l");
        assert_eq!(t.fg, main_fg);
        assert_eq!(t.attrs, main_attrs);
        assert!(t.attrs.contains(CellAttrs::BOLD));
    }

    #[test]
    fn alt_screen_resize_scales_both_buffers() {
        let mut t = Terminal::new(4, 6);
        t.process_bytes(b"main"); // some content on main
        t.process_bytes(b"\x1b[?1049h");
        t.process_bytes(b"alt");

        // Resize while inside the alt screen.
        t.resize(6, 8);
        assert_eq!(t.grid().rows(), 6);
        assert_eq!(t.grid().cols(), 8);
        let alt = t.alt_grid.as_ref().expect("alt_grid must still exist");
        assert_eq!(alt.rows(), 6);
        assert_eq!(alt.cols(), 8);

        // Exiting should now restore at the new size, not the old one.
        t.process_bytes(b"\x1b[?1049l");
        assert!(t.alt_grid.is_none());
        assert_eq!(t.grid().rows(), 6);
        assert_eq!(t.grid().cols(), 8);
        // The original "main" content (lifted into the new top via
        // resize) should still be visible on row 2 (we grew by 2 rows
        // from a 4-row main with no scrollback).
        assert_eq!(read_row_str(&t, 2, 0, 4), "main");
    }

    #[test]
    fn alt_screen_legacy_47_and_1047_aliases() {
        // ?47 and ?1047 should both route to the same alt-screen path.
        for seq in [&b"\x1b[?47h"[..], &b"\x1b[?1047h"[..]] {
            let mut t = Terminal::new(3, 5);
            t.process_bytes(b"main");
            t.process_bytes(seq);
            assert!(
                t.alt_grid.is_some(),
                "{} should switch to alt screen",
                std::str::from_utf8(seq).unwrap(),
            );
        }
        for seq in [&b"\x1b[?47l"[..], &b"\x1b[?1047l"[..]] {
            let mut t = Terminal::new(3, 5);
            t.process_bytes(b"\x1b[?1049h"); // enter via 1049
            t.process_bytes(b"alt");
            t.process_bytes(seq); // exit via legacy alias
            assert!(
                t.alt_grid.is_none(),
                "{} should leave the alt screen",
                std::str::from_utf8(seq).unwrap(),
            );
        }
    }

    #[test]
    fn alt_screen_double_enter_is_idempotent() {
        let mut t = Terminal::new(3, 5);
        t.process_bytes(b"main");
        let cursor_before = t.cursor_position();
        t.process_bytes(b"\x1b[?1049h");
        let saved_after_first = t.alt_saved_cursor;
        // Move the cursor inside the alt screen.
        t.process_bytes(b"\x1b[2;3H");
        // Re-entering must NOT clobber the original cursor save slot.
        t.process_bytes(b"\x1b[?1049h");
        assert_eq!(t.alt_saved_cursor, saved_after_first);
        assert_eq!(saved_after_first, cursor_before);
    }

    // -- DECSTBM (CSI <top>;<bot> r) ------------------------------------------

    #[test]
    fn decstbm_clamps_scroll_up_to_region() {
        let mut t = Terminal::new(6, 4);
        t.process_bytes(b"AA\r\nBB\r\nCC\r\nDD\r\nEE\r\nFF");
        // Set region rows 3..=5 (1-based) -> half-open [2, 5).
        t.process_bytes(b"\x1b[3;5r");
        assert_eq!(t.scroll_top, 2);
        assert_eq!(t.scroll_bot, 5);
        // Cursor is parked at home after DECSTBM.
        assert_eq!(t.cursor_position(), (0, 0));

        // Move cursor inside the region and trigger a scroll by emitting
        // CSI 1 S (scroll up). With my changes, this now operates only
        // on rows [2, 5).
        t.process_bytes(b"\x1b[1S");

        // Rows 0 and 1 are pinned by the region.
        assert_eq!(row_text(&t, 0), "AA");
        assert_eq!(row_text(&t, 1), "BB");
        // The region's top (row 2) was discarded; CC is gone, DD shifted up.
        assert_eq!(row_text(&t, 2), "DD");
        assert_eq!(row_text(&t, 3), "EE");
        // Region's last row (row 4 = 5-1) is now blank.
        assert_eq!(row_text(&t, 4), "");
        // Row 5 sits below the region and is untouched.
        assert_eq!(row_text(&t, 5), "FF");
    }

    #[test]
    fn decstbm_reset_restores_full_screen_scrolling() {
        let mut t = Terminal::new(4, 4);
        t.process_bytes(b"\x1b[2;3r"); // narrow region
        assert_eq!(t.scroll_top, 1);
        assert_eq!(t.scroll_bot, 3);
        // Reset to full screen with CSI r (no params).
        t.process_bytes(b"\x1b[r");
        assert_eq!(t.scroll_top, 0);
        assert_eq!(t.scroll_bot, 4);
        // Now a regular scroll affects the whole grid as before.
        t.process_bytes(b"L1\r\nL2\r\nL3\r\nL4\r\nL5");
        assert_eq!(row_text(&t, 0), "L2");
        assert_eq!(row_text(&t, 1), "L3");
        assert_eq!(row_text(&t, 2), "L4");
        assert_eq!(row_text(&t, 3), "L5");
    }

    #[test]
    fn decstbm_lf_at_region_bottom_scrolls_region_only() {
        // Issue Claude Code symptom: input prompt pinned below the region.
        let mut t = Terminal::new(5, 4);
        // Pre-fill the screen so we can see what shifts.
        t.process_bytes(b"AA\r\nBB\r\nCC\r\nDD\r\nEE");
        // Region [1, 4) — rows 0 and 4 are pinned.
        t.process_bytes(b"\x1b[2;4r");
        // Move cursor to last row of region (row 3 zero-indexed), col 0.
        t.process_bytes(b"\x1b[4;1H");
        assert_eq!(t.cursor_position(), (3, 0));
        // LF at the region's bottom must scroll the region up by one,
        // not move the cursor below the region.
        t.process_bytes(b"\n");
        assert_eq!(t.cursor_position(), (3, 0));
        // Pinned rows untouched.
        assert_eq!(row_text(&t, 0), "AA");
        assert_eq!(row_text(&t, 4), "EE");
        // Region content shifted: CC lost, DD moved to row 1, BB stayed
        // at row 1? Let's trace it carefully.
        // Before: row1=BB row2=CC row3=DD. Scroll up region [1,4):
        // row1=CC row2=DD row3=blank.
        assert_eq!(row_text(&t, 1), "CC");
        assert_eq!(row_text(&t, 2), "DD");
        assert_eq!(row_text(&t, 3), "");
    }

    #[test]
    fn decstbm_invalid_params_leave_region_unchanged() {
        let mut t = Terminal::new(5, 4);
        // Set a valid region first so we can verify it survives bad params.
        t.process_bytes(b"\x1b[2;4r");
        let (top_before, bot_before) = (t.scroll_top, t.scroll_bot);

        // top > bot — must be ignored. (1-based 4;2 means top=3, bot=2;
        // half-open: top=3, bot=2, fails the strict `top < bot` check.)
        t.process_bytes(b"\x1b[4;2r");
        assert_eq!((t.scroll_top, t.scroll_bot), (top_before, bot_before));

        // top out of range and bot equally invalid (top_1 > rows).
        // top_1 = 99 -> new_top = 98, new_bot clamped to 5: 98 >= 5,
        // strict `top < bot` fails -> ignored.
        t.process_bytes(b"\x1b[99;5r");
        assert_eq!((t.scroll_top, t.scroll_bot), (top_before, bot_before));

        // bot beyond rows clamps to rows. With rows = 5 the params
        // (1, 99) become [0, 5) -- xterm accepts this and so do we.
        t.process_bytes(b"\x1b[1;99r");
        assert_eq!((t.scroll_top, t.scroll_bot), (0, 5));
    }

    #[test]
    fn decstbm_resize_clamps_or_resets_region() {
        let mut t = Terminal::new(6, 4);
        t.process_bytes(b"\x1b[2;5r"); // region [1, 5) within 6 rows
        assert_eq!((t.scroll_top, t.scroll_bot), (1, 5));

        // Shrink so the previous region no longer fits.
        t.resize(3, 4);
        // Region must reset to full-screen [0, 3) since [1, 5) is now invalid.
        assert_eq!((t.scroll_top, t.scroll_bot), (0, 3));

        // Now set a region and resize within bounds: it should NOT reset.
        t.process_bytes(b"\x1b[1;2r"); // [0, 2) on a 3-row screen
        t.resize(4, 4);
        // Still within bounds (bot = 2 <= rows = 4), region preserved.
        assert_eq!((t.scroll_top, t.scroll_bot), (0, 2));
    }

    #[test]
    fn decstbm_il_dl_inside_region_only() {
        let mut t = Terminal::new(5, 4);
        t.process_bytes(b"AA\r\nBB\r\nCC\r\nDD\r\nEE");
        t.process_bytes(b"\x1b[2;4r"); // region [1, 4)

        // Cursor inside the region.
        t.process_bytes(b"\x1b[2;1H"); // (1, 0)

        // Insert one line at row 1: rows in [1, 4) shift down by one,
        // row 1 becomes blank, row 4 (below region) is untouched.
        t.process_bytes(b"\x1b[1L");
        assert_eq!(row_text(&t, 0), "AA"); // pinned above
        assert_eq!(row_text(&t, 1), ""); // inserted blank
        assert_eq!(row_text(&t, 2), "BB"); // shifted down
        assert_eq!(row_text(&t, 3), "CC"); // shifted down
        assert_eq!(row_text(&t, 4), "EE"); // pinned below
    }

    #[test]
    fn decstbm_region_does_not_pollute_scrollback() {
        // TUIs use scroll regions to redraw status lines; that scrolling
        // must NOT leak into scrollback (which would let the user scroll
        // up and see partial frames).
        let mut t = Terminal::new(4, 4);
        t.process_bytes(b"AA\r\nBB\r\nCC\r\nDD");
        let scrollback_before = t.scrollback_len();

        t.process_bytes(b"\x1b[2;3r"); // region [1, 3)
        t.process_bytes(b"\x1b[3S"); // scroll up 3 inside region

        assert_eq!(
            t.scrollback_len(),
            scrollback_before,
            "narrowed-region scrolls must not push to scrollback",
        );
    }
}
