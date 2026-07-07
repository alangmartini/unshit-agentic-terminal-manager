//! VTE-based terminal emulator that drives a `CellGrid`.
//!
//! Parses ANSI escape sequences from PTY output using the `vte` crate (0.13)
//! and renders them into a `CellGrid` from the unshit framework. Supports
//! cursor movement, scrolling, text attributes (bold, italic, underline, etc.),
//! 256-color and true-color SGR, erase operations, and window title (OSC).

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use unshit::app::scroll_motion::ScrollMotion;
use unshit::core::cell_grid::{color_256, Cell, CellAttrs, CellGrid, ANSI_16};
use unshit::core::style::types::Color;
use unshit::core::trace::{append_terminal_trace_line, terminal_trace_enabled};
use vte::{Params, Perform};

pub mod keys;
pub mod paste_image;

/// Maximum number of scrollback lines retained per terminal.
const MAX_SCROLLBACK: usize = 10_000;

/// High-bit namespace for the overscan row's stable line id in
/// [`Terminal::display_grid`] snapshots. The id is derived from the
/// overscan line's absolute index so an unchanged overscan row keeps its
/// identity across snapshots, and the namespace bit keeps it from
/// colliding with the grid's own monotonic line ids.
const OVERSCAN_LINE_ID_NAMESPACE: u64 = 1 << 63;

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
    /// Fractional wheel-scroll carry in lines. `scroll_view_by_lines`
    /// applies whole lines to `scroll_offset` and parks the sub-line
    /// remainder here so touchpads and high-resolution wheels track
    /// their input 1:1 instead of rounding every event to a full line.
    /// While no animation is in flight this carry is also the rendered
    /// sub-row fraction (see `fractional_view_position`).
    scroll_accum_lines: f32,
    /// Active wheel-scroll animation in bottom-anchored pixel space:
    /// `0.0` = live bottom, positive = scrolled back
    /// (`px = lines_scrolled_back * cell_h`). `None` when idle. Exactly
    /// one fractional-position owner exists at a time: while this is
    /// `Some`, `scroll_accum_lines` is zero and the rendered fraction
    /// lives in `scroll_view_fraction`; collapsing the animation hands
    /// the fraction back to the carry.
    scroll_anim: Option<ScrollMotion>,
    /// Cell height (physical px) the active animation's pixel space was
    /// built against. Sampling, retargeting, and PTY anchoring rescale
    /// the motion when the renderer's cell height changes mid-flight
    /// (Ctrl+wheel zoom, terminal font-size change), so the line-space
    /// position stays continuous instead of jumping by the zoom ratio.
    scroll_anim_cell_h: f32,
    /// Rendered sub-row fraction in `[0, 1)`: how far the viewport top
    /// has scrolled past `scroll_offset` toward `scroll_offset + 1`.
    /// Written by `sample_scroll_animation` each animation tick; zero
    /// whenever `scroll_anim` is `None` (the non-animating fraction is
    /// derived from `scroll_accum_lines` instead).
    scroll_view_fraction: f32,
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
    synchronized_output_active: bool,
    /// Whether the running program enabled bracketed paste mode via
    /// DECSET 2004 (`CSI ? 2004 h`). When set, pasted text should be
    /// wrapped in `ESC[200~` / `ESC[201~` so readline/editors can tell
    /// a paste from typed input. Reset by `CSI ? 2004 l`.
    bracketed_paste: bool,
    /// Count of scrollback lines permanently evicted off the top (when the
    /// buffer exceeds `MAX_SCROLLBACK`). Added to a line's index within the
    /// live `scrollback ++ screen` buffer to form a stable *absolute* line
    /// id that selections anchor to, so they survive scrolling and output.
    evicted_lines: u64,
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
            scroll_accum_lines: 0.0,
            scroll_anim: None,
            scroll_anim_cell_h: 0.0,
            scroll_view_fraction: 0.0,
            alt_grid: None,
            alt_saved_cursor: (0, 0),
            alt_saved_fg: default_fg(),
            alt_saved_bg: default_bg(),
            alt_saved_attrs: CellAttrs::empty(),
            scroll_top: 0,
            scroll_bot: rows,
            pending_response: Vec::new(),
            synchronized_output_active: false,
            bracketed_paste: false,
            evicted_lines: 0,
        }
    }

    /// Drain bytes the parser queued as a host-query reply. The bridge
    /// calls this after `process_bytes` to forward the reply over the
    /// PTY back to the running TUI.
    pub fn take_pending_response(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.pending_response)
    }

    pub fn synchronized_output_active(&self) -> bool {
        self.synchronized_output_active
    }

    /// Feed raw bytes (from PTY output) through the VTE parser.
    ///
    /// The parser is temporarily moved out of `self` so that a `Performer`
    /// helper can borrow `&mut self` without conflicting with the parser's
    /// own `&mut self` requirement.
    pub fn process_bytes(&mut self, bytes: &[u8]) {
        // Gate S3 (scroll-smoothness spec): new PTY output no longer
        // snaps a scrolled-back view to the live screen. At the live
        // bottom the view follows output exactly as before (only the
        // dead fractional carry is discarded, matching the old
        // `reset_scroll`); when the user is reading scrollback the view
        // stays anchored instead — `scroll_up` bumps `scroll_offset`
        // per pushed line so the same content stays on screen.
        if self.live_bottom_intent() {
            self.scroll_accum_lines = 0.0;
        }

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
        // A viewport reflow invalidates the animation's notion of "rows
        // from the bottom"; freeze the view at its current sample (the
        // resize forces a full rebuild anyway). Same-dimension calls
        // (the resize handler republishing unchanged metrics) must not
        // disturb an in-flight animation.
        if rows != self.rows || cols != self.cols {
            self.collapse_scroll_animation();
        }
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

        self.collapse_scroll_animation();
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
        self.reset_scroll();
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
    /// Cancels any in-flight wheel animation first (collapsing it to
    /// its last rendered sample) so a direct whole-line jump and the
    /// animation sampler never fight over `scroll_offset`.
    pub fn scroll_view_up(&mut self, n: usize) {
        self.collapse_scroll_animation();
        let max = self.scrollback.len();
        self.scroll_offset = (self.scroll_offset + n).min(max);
    }

    /// Scroll the view forward (toward live screen) by `n` lines.
    /// Cancels any in-flight wheel animation first, like
    /// [`Terminal::scroll_view_up`].
    pub fn scroll_view_down(&mut self, n: usize) {
        self.collapse_scroll_animation();
        self.scroll_offset = self.scroll_offset.saturating_sub(n);
    }

    /// Scroll the view by a possibly fractional number of lines,
    /// carrying the sub-line remainder over to the next call.
    ///
    /// Positive `delta_lines` scrolls up (toward older history),
    /// negative scrolls down (toward the live screen). Whole lines are
    /// applied immediately via `scroll_view_up` / `scroll_view_down`;
    /// the fraction accumulates so a sequence of small wheel deltas
    /// sums to exactly its total instead of rounding per event. When
    /// the view clamps at either end of the scrollback the carry is
    /// discarded so pent-up remainder cannot rubber-band a later
    /// gesture; likewise, sub-line carry pointing into a boundary the
    /// view is already pinned against is discarded so it cannot swallow
    /// or amplify a later reversal. Non-finite deltas are ignored.
    /// Returns the signed number of lines actually applied.
    pub fn scroll_view_by_lines(&mut self, delta_lines: f32) -> i32 {
        if !delta_lines.is_finite() {
            return 0;
        }
        // Precision-device takeover: a touchpad delta cancels any
        // in-flight wheel animation, folding its sampled fraction into
        // the carry so the rendered position is unchanged and the two
        // fractional-position owners can never double-apply.
        self.collapse_scroll_animation();
        self.scroll_accum_lines += delta_lines;
        // Defense in depth: a finite delta should never push the
        // accumulator non-finite, but never let a poisoned value persist.
        if !self.scroll_accum_lines.is_finite() {
            self.scroll_accum_lines = 0.0;
        }
        let whole = self.scroll_accum_lines.trunc() as i32;
        self.scroll_accum_lines -= whole as f32;
        let before = self.scroll_offset;
        if whole > 0 {
            self.scroll_view_up(whole as usize);
        } else if whole < 0 {
            self.scroll_view_down(whole.unsigned_abs() as usize);
        }
        let applied = self.scroll_offset as i64 - before as i64;
        if applied != whole as i64 {
            self.scroll_accum_lines = 0.0;
        }
        // Sub-line carry pressing into a pinned boundary is dead input:
        // negative carry at the bottom (offset 0) or positive carry at
        // the top (offset == scrollback len) can never be applied, so
        // discard it rather than let it offset the next gesture.
        if (self.scroll_offset == 0 && self.scroll_accum_lines < 0.0)
            || (self.scroll_offset == self.scrollback.len() && self.scroll_accum_lines > 0.0)
        {
            self.scroll_accum_lines = 0.0;
        }
        applied as i32
    }

    /// Snap the view back to the live screen, cancelling any in-flight
    /// wheel animation and discarding every fractional remainder.
    pub fn reset_scroll(&mut self) {
        self.scroll_offset = 0;
        self.scroll_accum_lines = 0.0;
        self.scroll_anim = None;
        self.scroll_view_fraction = 0.0;
    }

    // -- animated wheel scrolling (sub-row precision) -------------------------
    //
    // A wheel notch no longer teleports the view by whole rows: the
    // handler retargets a `ScrollMotion` in bottom-anchored pixel space
    // (`scroll_animate_by_px`) and the framework's animation tick
    // samples it (`sample_scroll_animation`) into the integer
    // `scroll_offset` plus a sub-row `scroll_view_fraction` that
    // `display_grid` renders as a fractional translation. Invariant:
    // exactly one fractional-position owner at a time — the animation
    // (`scroll_view_fraction`) or the touchpad carry
    // (`scroll_accum_lines`); collapse hands the value across.

    /// Cancel any in-flight wheel animation by collapsing it to its
    /// most recent sample: `scroll_offset` already holds the sampled
    /// whole rows, so the sampled fraction moves into the touchpad
    /// carry and the rendered position is unchanged. Idempotent.
    fn collapse_scroll_animation(&mut self) {
        if self.scroll_anim.take().is_some() {
            self.scroll_accum_lines = self.scroll_view_fraction;
            self.scroll_view_fraction = 0.0;
        }
    }

    /// Rescale the active animation's pixel space when the renderer's
    /// cell height changed mid-flight (zoom / font-size change). Both
    /// endpoints scale by the same ratio, so the sampled position is
    /// exactly preserved in line space.
    fn rescale_scroll_animation(&mut self, cell_h: f32) {
        if let Some(motion) = self.scroll_anim.as_mut() {
            if self.scroll_anim_cell_h > 0.0 && cell_h > 0.0 && self.scroll_anim_cell_h != cell_h {
                let ratio = cell_h / self.scroll_anim_cell_h;
                motion.start *= ratio;
                motion.target *= ratio;
                self.scroll_anim_cell_h = cell_h;
            }
        }
    }

    /// The rendered scroll position as `(whole_rows, fraction)` with
    /// `fraction` in `[0, 1)`: the viewport top sits `fraction` rows
    /// above the `whole_rows` viewport. While animating this is the
    /// last sample; otherwise the touchpad carry is normalized into a
    /// forward fraction (a negative carry borrows from `scroll_offset`)
    /// so precision devices get true sub-row tracking too.
    pub fn fractional_view_position(&self) -> (usize, f32) {
        if self.scroll_anim.is_some() {
            return (self.scroll_offset, self.scroll_view_fraction);
        }
        let carry = self.scroll_accum_lines;
        if carry > 0.0 {
            if self.scroll_offset >= self.scrollback.len() {
                // Positive carry pinned at the top of scrollback is dead
                // input (no line above exists to reveal).
                (self.scroll_offset, 0.0)
            } else {
                (self.scroll_offset, carry)
            }
        } else if carry < 0.0 && self.scroll_offset >= 1 {
            (self.scroll_offset - 1, 1.0 + carry)
        } else {
            (self.scroll_offset, 0.0)
        }
    }

    /// `true` when the rendered view is anywhere but the live bottom
    /// (whole rows, a sub-row fraction, or an in-flight animation).
    /// Drives snap-to-live-on-keypress.
    pub fn is_view_scrolled(&self) -> bool {
        if self.scroll_anim.is_some() {
            return true;
        }
        let (offset, fraction) = self.fractional_view_position();
        offset > 0 || fraction > 0.0
    }

    /// The rendered position quantized to whole device pixels:
    /// `(whole_rows, render_offset_px)`, where the second component is
    /// exactly the paint-time translation `display_grid` emits (see
    /// [`Self::render_offset_px`]). Two equal values paint identically,
    /// so this is the change detector for animation ticks and the
    /// instant wheel path.
    pub fn rendered_scroll_px(&self, cell_h: f32) -> (usize, i64) {
        let (offset, fraction) = self.fractional_view_position();
        (offset, Self::render_offset_px(fraction, cell_h))
    }

    /// The vertical paint-time translation for a sub-row `fraction`, in
    /// whole device pixels: `round((fraction - 1) * cell_h)`. The whole
    /// offset is rounded (not just the animated component) so the
    /// translated primitives stay pixel-aligned even when `cell_h`
    /// itself is fractional — pixel-snapped cell quads and the glyph
    /// rasterizer's subpixel bins are chosen at the pre-translation
    /// position, so a fractional residue in the offset would displace
    /// every snapped edge and glyph off its rasterized phase on every
    /// frame, idle included.
    fn render_offset_px(fraction: f32, cell_h: f32) -> i64 {
        ((fraction - 1.0) * cell_h).round() as i64
    }

    /// Start or retarget the wheel-scroll animation by `delta_px`
    /// pixels (positive scrolls toward older history, matching
    /// `scroll_view_by_lines`). Compounds off the previous target with
    /// velocity continuity (`ScrollMotion::retarget`), clamps the
    /// target to `[0, scrollback_len * cell_h]`, and folds any parked
    /// touchpad carry into the start position on takeover. Returns
    /// `true` when an animation is (still) in flight or a collapsed
    /// retarget needs one settling repaint; `false` means nothing will
    /// change. Non-finite parameters are rejected.
    pub fn scroll_animate_by_px(
        &mut self,
        delta_px: f32,
        now: Instant,
        duration: Duration,
        initial_slope: f32,
        cell_h: f32,
    ) -> bool {
        if !delta_px.is_finite()
            || !initial_slope.is_finite()
            || !cell_h.is_finite()
            || cell_h <= 0.0
        {
            return false;
        }
        self.rescale_scroll_animation(cell_h);
        let max_px = self.scrollback.len() as f32 * cell_h;
        let had_anim = self.scroll_anim.is_some();
        let current = match self.scroll_anim {
            Some(motion) => motion.sample(now).0.clamp(0.0, max_px),
            None => {
                // Takeover from the idle/touchpad state: normalize the
                // carry into the rendered (offset, fraction) pair so the
                // animation starts from exactly the position on screen,
                // then transfer ownership of the fraction to the motion.
                let (offset, fraction) = self.fractional_view_position();
                self.scroll_offset = offset;
                self.scroll_view_fraction = fraction;
                self.scroll_accum_lines = 0.0;
                ((offset as f32 + fraction) * cell_h).clamp(0.0, max_px)
            }
        };
        match ScrollMotion::retarget(
            self.scroll_anim,
            current,
            -delta_px,
            max_px,
            now,
            duration,
            initial_slope,
        ) {
            Some(motion) => {
                self.scroll_anim = Some(motion);
                self.scroll_anim_cell_h = cell_h;
                true
            }
            None => {
                // Epsilon no-op: the compounded target collapses onto
                // the current position. Settle there; an in-flight
                // animation still needs one repaint at the settled
                // position before it quiesces.
                self.scroll_anim = None;
                self.settle_scroll_position_px(current, cell_h);
                had_anim
            }
        }
    }

    /// Write a bottom-anchored pixel position into the non-animating
    /// state: whole rows go to `scroll_offset`, the sub-row remainder
    /// becomes the carry so `fractional_view_position` keeps rendering
    /// it. Caller must have cleared `scroll_anim`.
    fn settle_scroll_position_px(&mut self, pos_px: f32, cell_h: f32) {
        let sb_len = self.scrollback.len();
        let lines = pos_px / cell_h;
        let whole = (lines.floor().max(0.0) as usize).min(sb_len);
        self.scroll_offset = whole;
        self.scroll_accum_lines = if whole >= sb_len {
            0.0
        } else {
            (lines - whole as f32).clamp(0.0, 1.0 - f32::EPSILON)
        };
        self.scroll_view_fraction = 0.0;
    }

    /// Advance the wheel animation to `now`, mapping the eased pixel
    /// position to `(scroll_offset, scroll_view_fraction)`. Returns
    /// `None` when no animation is in flight (e.g. cancelled by a
    /// takeover or snap-to-live), else `Some((changed, finished))`
    /// where `changed` means the device-pixel rendering moved and
    /// `finished` means the motion completed and its resting sub-row
    /// fraction was handed to the carry (the final frame paints
    /// identical pixels from the non-animating state). Pure in the
    /// injected timestamp.
    pub fn sample_scroll_animation(&mut self, now: Instant, cell_h: f32) -> Option<(bool, bool)> {
        let motion = self.scroll_anim?;
        if !cell_h.is_finite() || cell_h <= 0.0 {
            return Some((false, false));
        }
        self.rescale_scroll_animation(cell_h);
        let motion = self.scroll_anim.unwrap_or(motion);
        let sb_len = self.scrollback.len();
        let max_px = sb_len as f32 * cell_h;
        let (raw_pos, _, finished) = motion.sample(now);
        let pos = raw_pos.clamp(0.0, max_px);
        let before = self.rendered_scroll_px(cell_h);
        if finished {
            self.scroll_anim = None;
            self.settle_scroll_position_px(pos, cell_h);
        } else {
            let lines = pos / cell_h;
            let whole = (lines.floor().max(0.0) as usize).min(sb_len);
            self.scroll_offset = whole;
            self.scroll_view_fraction = if whole >= sb_len {
                0.0
            } else {
                (lines - whole as f32).clamp(0.0, 1.0 - f32::EPSILON)
            };
        }
        let after = self.rendered_scroll_px(cell_h);
        Some((before != after, finished))
    }

    /// Whether the user's intent is "follow the live output": at the
    /// rendered live bottom with no animation, or an animation whose
    /// target is the live bottom (a downward fling must be able to land
    /// even while output streams). Anything else is reading intent and
    /// PTY output anchors the viewport instead of snapping it.
    ///
    /// "At the live bottom" is judged in device pixels, not exact
    /// floats: a landing that left a sub-half-pixel residue (float error
    /// in `settle_scroll_position_px`, an epsilon-collapsed retarget)
    /// renders identically to the live bottom, so it must not silently
    /// disable follow-output. The residue itself is dead input and is
    /// discarded by `process_bytes` on the next output chunk.
    fn live_bottom_intent(&self) -> bool {
        /// Anything below half a device pixel rounds to "no offset".
        const HALF_DEVICE_PX: f32 = 0.5;
        if let Some(motion) = self.scroll_anim {
            return motion.target.abs() < HALF_DEVICE_PX;
        }
        let (offset, fraction) = self.fractional_view_position();
        if offset != 0 {
            return false;
        }
        if fraction == 0.0 {
            return true;
        }
        // Convert the line-space fraction to pixels with the best cell
        // height available (the animation's, else the renderer's
        // published metric); without one, fall back to the exact check.
        let cell_h = if self.scroll_anim_cell_h > 0.0 {
            self.scroll_anim_cell_h
        } else {
            CellGrid::global_cell_h()
        };
        cell_h > 0.0 && fraction * cell_h < HALF_DEVICE_PX
    }

    /// Build a `CellGrid` representing what should be displayed.
    ///
    /// Snapshots are always `(rows + 1, cols)`: one overscan row above
    /// the viewport (declared via `overscan_rows`) plus a vertical
    /// paint-time translation `render_offset_y = -(1 - fraction) *
    /// cell_h` (snapped to whole device pixels), so sub-row scroll
    /// positions render as a fractional slide with the overscan row
    /// covering the gap at the top. The constant shape keeps the
    /// framework's paint-only grid patches and the reconciler's
    /// grid-content fast path dimension-stable across live/scrolled
    /// states.
    ///
    /// At the live bottom the snapshot is a clone of the live grid with
    /// the newest scrollback line prepended (preserving the
    /// damage-splice fast path); when scrolled back it is composed from
    /// scrollback lines and the upper portion of the live screen, with
    /// the cursor hidden.
    pub fn display_grid(&self) -> CellGrid {
        let (offset, fraction) = self.fractional_view_position();
        // Producer half of the renderer contract: the offset is in
        // physical pixels, and the WHOLE offset is snapped to a whole
        // device pixel (`render_offset_px`). Cell-background quads are
        // pixel-snapped and glyph subpixel bins are chosen at the
        // pre-translation position, so any fractional residue in the
        // offset (e.g. an exact `-cell_h` base with a fractional cell
        // height) would shift every snapped edge and glyph off its
        // rasterized phase — on every frame, idle included. A whole-pixel
        // offset keeps the final positions on the same pixel grid the
        // primitives were snapped/binned for; at a zero fraction the rows
        // land within half a device pixel of the offset-free layout. When
        // cell metrics have not been published yet (headless tests, the
        // instant before the first ever frame) the fraction is
        // necessarily zero and the offset degrades to 0.
        let cell_h = CellGrid::global_cell_h();
        let render_offset_y = if cell_h > 0.0 {
            Self::render_offset_px(fraction, cell_h) as f32
        } else {
            0.0
        };

        if offset == 0 && fraction == 0.0 {
            let mut view = self.grid.clone();
            let (overscan_cells, overscan_id) = match self.scrollback.back() {
                Some(row) => (
                    Some(row.as_slice()),
                    OVERSCAN_LINE_ID_NAMESPACE
                        | (self.evicted_lines + self.scrollback.len() as u64 - 1),
                ),
                None => (None, OVERSCAN_LINE_ID_NAMESPACE),
            };
            view.insert_overscan_row_top(overscan_cells, overscan_id);
            view.set_overscan_rows(1);
            view.set_render_offset_y(render_offset_y);
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

        let mut view = CellGrid::new(self.rows + 1, self.cols);
        let sb_len = self.scrollback.len();
        // Virtual index (into scrollback ++ screen) of the viewport's
        // top line. Display row 0 is the overscan line above it, blank
        // when the view is pinned at the oldest line.
        let base = sb_len.saturating_sub(offset);

        for display_row in 0..self.rows + 1 {
            let Some(virtual_line) = (base + display_row).checked_sub(1) else {
                continue;
            };

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
        view.set_overscan_rows(1);
        view.set_render_offset_y(render_offset_y);
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

    // -- selection / clipboard -----------------------------------------------
    //
    // Selections are stored in *absolute line* coordinates: an index into
    // the conceptual `scrollback ++ screen` buffer, offset by the number of
    // lines that have been permanently evicted off the top of scrollback.
    // A content line keeps its absolute index for life (until evicted), so a
    // selection stays pinned to its text as the view scrolls and as output
    // pushes lines into scrollback — no display-relative drift, no need to
    // clear the selection just because the screen moved.

    /// Whether the running program enabled bracketed paste mode via
    /// DECSET 2004. `terminal.paste` wraps the payload in
    /// `ESC[200~`/`ESC[201~` when this is set.
    pub fn bracketed_paste_active(&self) -> bool {
        self.bracketed_paste
    }

    /// Absolute index of the virtual line currently shown at display
    /// row 0 (the viewport top, not the overscan row). Uses the
    /// normalized rendered offset so it agrees with `display_grid` even
    /// when a negative touchpad carry borrows a row.
    fn top_abs_line(&self) -> u64 {
        let (offset, _) = self.fractional_view_position();
        let top_virtual = self.scrollback.len().saturating_sub(offset);
        self.evicted_lines + top_virtual as u64
    }

    /// Map an element-local pixel `y_px` to the absolute line rendered
    /// there, accounting for the sub-row fraction: the content is
    /// displaced down by `fraction * cell_h`, so
    /// `abs = top_abs_line() + floor(y/cell_h - fraction)`, which
    /// resolves to the partially visible overscan line for small `y`
    /// while scrolled to a fraction. Clamped to the rows actually on
    /// screen and to the addressable line range.
    pub fn view_pixel_to_abs_line(&self, y_px: f32, cell_h: f32) -> u64 {
        let top = self.top_abs_line();
        if !y_px.is_finite() || !cell_h.is_finite() || cell_h <= 0.0 {
            return top;
        }
        let (_, fraction) = self.fractional_view_position();
        let min_rel = if fraction > 0.0 { -1.0 } else { 0.0 };
        let max_rel = self.rows.saturating_sub(1) as f32;
        let rel = (y_px.max(0.0) / cell_h - fraction)
            .floor()
            .clamp(min_rel, max_rel);
        if rel < 0.0 {
            top.saturating_sub(1).max(self.first_abs_line())
        } else {
            top + rel as u64
        }
    }

    /// Smallest still-addressable absolute line (the oldest line retained in
    /// scrollback). Lines below this have been evicted and are gone.
    pub fn first_abs_line(&self) -> u64 {
        self.evicted_lines
    }

    /// One past the largest addressable absolute line.
    fn end_abs_line(&self) -> u64 {
        self.evicted_lines + (self.scrollback.len() + self.rows) as u64
    }

    /// Absolute line index for the cell currently shown at `display_row`.
    /// The same mapping [`Terminal::display_grid`] uses, lifted into absolute
    /// space so the result is stable as the buffer scrolls.
    pub fn abs_line_at_display(&self, display_row: usize) -> u64 {
        self.top_abs_line() + display_row as u64
    }

    /// The cell at absolute `(abs, col)`, reading from scrollback or the live
    /// screen as appropriate. `None` when the line was evicted or is out of
    /// bounds.
    fn cell_at_abs(&self, abs: u64, col: usize) -> Option<Cell> {
        if col >= self.cols || abs < self.evicted_lines {
            return None;
        }
        let virtual_line = (abs - self.evicted_lines) as usize;
        let sb_len = self.scrollback.len();
        if virtual_line < sb_len {
            self.scrollback[virtual_line].get(col).copied()
        } else {
            let screen_row = virtual_line - sb_len;
            if screen_row < self.rows {
                self.grid.get_cell(screen_row, col).copied()
            } else {
                None
            }
        }
    }

    /// True when `ch` counts as part of a word for double-click selection.
    /// Includes alphanumerics plus the punctuation that commonly appears in
    /// identifiers and paths so double-clicking a path or flag grabs the
    /// whole token rather than a fragment.
    fn is_word_char(ch: char) -> bool {
        ch.is_alphanumeric() || "_./-~+:@%".contains(ch)
    }

    /// Inclusive `[start_col, end_col]` column span of the word at absolute
    /// `(abs_line, col)`. A click on a non-word cell selects just that cell.
    pub fn word_bounds_at(&self, abs_line: u64, col: usize) -> (usize, usize) {
        if self.cols == 0 {
            return (0, 0);
        }
        let col = col.min(self.cols - 1);
        let ch_at = |c: usize| {
            self.cell_at_abs(abs_line, c)
                .map(|cell| if cell.ch == '\0' { ' ' } else { cell.ch })
                .unwrap_or(' ')
        };
        let here = ch_at(col);
        if !Self::is_word_char(here) {
            return (col, col);
        }
        let mut start = col;
        while start > 0 && Self::is_word_char(ch_at(start - 1)) {
            start -= 1;
        }
        let mut end = col;
        while end + 1 < self.cols && Self::is_word_char(ch_at(end + 1)) {
            end += 1;
        }
        (start, end)
    }

    /// Inclusive `[start_col, end_col]` span covering a whole line, used by
    /// triple-click line selection. Trailing blanks are trimmed by
    /// [`Terminal::selection_text`] at copy time.
    pub fn line_bounds_at(&self, _abs_line: u64) -> (usize, usize) {
        (0, self.cols.saturating_sub(1))
    }

    /// The `http`/`https` URL occupying absolute `(abs_line, col)`, or `None`
    /// when the clicked cell is not inside a recognizable link. Powers
    /// Ctrl+click-to-open. Detection is single-line: a URL that soft-wraps onto
    /// the next row is only recognized up to the row boundary. Only `http://`
    /// and `https://` are matched so a click can never hand an arbitrary
    /// protocol (`file:`, custom handlers) to the OS; scheme is re-validated at
    /// open time in [`crate::browser`].
    pub fn url_at(&self, abs_line: u64, col: usize) -> Option<String> {
        if self.cols == 0 || col >= self.cols {
            return None;
        }
        // Materialize the row as one char per column. URLs are ASCII, so every
        // URL character occupies exactly one cell and column == char index;
        // wide (CJK) continuation cells collapse to spaces and simply break a
        // run, which is correct because they can't appear inside a URL.
        let line: Vec<char> = (0..self.cols)
            .map(|c| {
                self.cell_at_abs(abs_line, c)
                    .map(|cell| if cell.ch == '\0' { ' ' } else { cell.ch })
                    .unwrap_or(' ')
            })
            .collect();
        find_url_at_col(&line, col)
    }

    /// Paint `bg` over every visible cell of the selection spanning absolute
    /// coordinates `anchor`..`focus` (order-independent) on `grid`, which
    /// must be the [`Terminal::display_grid`] clone for the current view.
    /// Lines scrolled out of the viewport are skipped; the selection itself
    /// is unchanged. The range is inclusive on both ends, so `anchor == focus`
    /// paints exactly one cell — whether a collapsed range should highlight is
    /// the caller's decision (`apply_selection_highlight` filters out an empty
    /// `Cell` selection via `is_empty`, while a single-character word selection
    /// legitimately reaches here and must paint).
    pub fn paint_selection(
        &self,
        grid: &mut CellGrid,
        anchor: (u64, usize),
        focus: (u64, usize),
        bg: Color,
    ) {
        if self.rows == 0 || self.cols == 0 {
            return;
        }
        let (start, end) = if anchor <= focus {
            (anchor, focus)
        } else {
            (focus, anchor)
        };
        let last_col = self.cols - 1;
        // Only the on-screen portion of the selection needs painting.
        // An overscan grid carries one extra (partially visible) line
        // above the viewport at row 0; paint it too so a sub-row scroll
        // position never clips the highlight at the top edge.
        let overscan = grid.overscan_rows() as u64;
        let top = self.top_abs_line();
        let vis_top = top.saturating_sub(overscan).max(self.first_abs_line());
        let vis_bot = top + (self.rows.saturating_sub(1)) as u64;
        let from = start.0.max(vis_top);
        let to = end.0.min(vis_bot);
        let mut abs = from;
        while abs <= to {
            // Grid row for `abs`: the viewport top renders at grid row
            // `overscan`, lines above it (only the overscan line) at
            // smaller indices.
            let row = (abs + overscan).checked_sub(top).map(|r| r as usize);
            if let Some(row) = row.filter(|&r| r < grid.rows()) {
                let c0 = if abs == start.0 {
                    start.1.min(last_col)
                } else {
                    0
                };
                let c1 = if abs == end.0 {
                    end.1.min(last_col)
                } else {
                    last_col
                };
                for col in c0..=c1 {
                    if let Some(mut cell) = grid.get_cell(row, col).copied() {
                        cell.bg = bg;
                        cell.attrs.remove(CellAttrs::INVERSE);
                        grid.set_cell(row, col, cell);
                    }
                }
            }
            abs += 1;
        }
    }

    /// Plain text for the selection spanning absolute coordinates `a`..`b`
    /// (order-independent), read straight from the buffer so it is correct
    /// regardless of the current scroll position. Linear (stream) selection:
    /// the first line runs from its start column to end of line, interior
    /// lines are full, the last runs to its end column. Wide-character
    /// continuation cells are skipped, empty cells become spaces, trailing
    /// whitespace is trimmed per line, and lines join with `\n`.
    pub fn selection_text(&self, a: (u64, usize), b: (u64, usize)) -> String {
        if self.rows == 0 || self.cols == 0 {
            return String::new();
        }
        let (start, end) = if a <= b { (a, b) } else { (b, a) };
        let last_col = self.cols - 1;
        // Clamp to the addressable range so an evicted top or an
        // out-of-range bottom does not emit blank padding lines.
        let start_abs = start.0.max(self.first_abs_line());
        let end_abs = end.0.min(self.end_abs_line().saturating_sub(1));
        if start_abs > end_abs {
            return String::new();
        }
        let mut lines: Vec<String> = Vec::new();
        let mut abs = start_abs;
        while abs <= end_abs {
            let c0 = if abs == start.0 {
                start.1.min(last_col)
            } else {
                0
            };
            let c1 = if abs == end.0 {
                end.1.min(last_col)
            } else {
                last_col
            };
            let mut s = String::new();
            for col in c0..=c1 {
                if let Some(cell) = self.cell_at_abs(abs, col) {
                    if cell.wide_continuation {
                        continue;
                    }
                    s.push(if cell.ch == '\0' { ' ' } else { cell.ch });
                }
            }
            lines.push(s.trim_end_matches(' ').to_string());
            abs += 1;
        }
        lines.join("\n")
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
                // A line left the buffer for good; bump the absolute-line
                // base so existing selection anchors stay pinned to the
                // right text (their absolute index is unaffected; indices
                // into the live buffer all shift down by one).
                self.evicted_lines += 1;
            }
            // Gate S3: a reader parked in scrollback keeps their view
            // anchored while output streams. Each pushed line moves the
            // viewport one line further from the live bottom, so bump
            // the offset (clamped at the top of scrollback, where
            // eviction is allowed to consume the view) and shift an
            // in-flight animation's pixel space by one row so its
            // sampled displacement stays continuous in content space.
            // A live-bottom intent (including a downward fling whose
            // target is the live edge) keeps today's follow behavior.
            if !self.live_bottom_intent() {
                self.scroll_offset = (self.scroll_offset + 1).min(self.scrollback.len());
                if let Some(motion) = self.scroll_anim.as_mut() {
                    let cell_h = self.scroll_anim_cell_h;
                    if cell_h > 0.0 {
                        let max_px = self.scrollback.len() as f32 * cell_h;
                        motion.start += cell_h;
                        motion.target = (motion.target + cell_h).min(max_px);
                    }
                }
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
        // A scrolled-back view (or an in-flight wheel animation) must not
        // survive into the alt screen: `display_grid` composes scrollback
        // above the live grid regardless of alt mode, so a TUI launched
        // while the user reads scrollback would render shifted down under
        // stale shell output. The gate-S3 anchoring in `process_bytes`
        // deliberately stopped snapping on output, so the snap happens
        // here, at the buffer switch, instead.
        self.reset_scroll();
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
        // Mirror `enter_alt_screen`: a view scrolled while the TUI owned
        // the screen (wheel over `less`, etc.) snaps back to the live
        // prompt on exit rather than dropping the user at a stale
        // scrollback position.
        self.reset_scroll();
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
// URL detection for Ctrl+click-to-open
// ---------------------------------------------------------------------------

/// Characters permitted in the body of a URL: the RFC 3986 unreserved,
/// gen-delim, sub-delim, and percent sets. Whitespace and control characters
/// terminate the run, so a link stops at the surrounding spaces.
fn is_url_char(ch: char) -> bool {
    matches!(ch,
        'a'..='z' | 'A'..='Z' | '0'..='9'
        // unreserved
        | '-' | '.' | '_' | '~'
        // gen-delims
        | ':' | '/' | '?' | '#' | '[' | ']' | '@'
        // sub-delims
        | '!' | '$' | '&' | '\'' | '(' | ')' | '*' | '+' | ',' | ';' | '='
        // percent-encoding
        | '%'
    )
}

/// Strip trailing characters that are far more likely to be sentence
/// punctuation than part of the link (e.g. the `).` in "(see http://x.com).").
/// A closing bracket is only dropped when it has no matching opener inside the
/// URL, so `http://en.wikipedia.org/wiki/Foo_(bar)` keeps its parenthesis.
fn trim_url_trailing(url: &str) -> &str {
    let mut end = url.len();
    let bytes = url.as_bytes();
    while end > 0 {
        let last = bytes[end - 1] as char;
        let drop = match last {
            '.' | ',' | ';' | ':' | '!' | '?' | '\'' | '"' | '<' | '>' => true,
            ')' | ']' | '}' => {
                let (open, close) = match last {
                    ')' => ('(', ')'),
                    ']' => ('[', ']'),
                    _ => ('{', '}'),
                };
                let slice = &url[..end];
                slice.matches(close).count() > slice.matches(open).count()
            }
            _ => false,
        };
        if drop {
            end -= 1;
        } else {
            break;
        }
    }
    &url[..end]
}

/// Find the `http`/`https` URL covering column `col` in a row rendered as one
/// `char` per column. Scans for each scheme, extends over the run of URL
/// characters, and returns the span (minus trailing punctuation) that contains
/// the click. `None` when the click is not on a link.
fn find_url_at_col(line: &[char], col: usize) -> Option<String> {
    if col >= line.len() {
        return None;
    }
    // Prefer the longer scheme first so "https" is never mis-split as "http".
    for scheme in ["https://", "http://"] {
        let sch: Vec<char> = scheme.chars().collect();
        let mut i = 0;
        while i + sch.len() <= line.len() {
            if line[i..i + sch.len()] == sch[..] {
                let mut end = i + sch.len(); // exclusive
                while end < line.len() && is_url_char(line[end]) {
                    end += 1;
                }
                // Require at least one host character after the scheme, and
                // that the click land within the matched span.
                if end > i + sch.len() && col >= i && col < end {
                    let run: String = line[i..end].iter().collect();
                    let url = trim_url_trailing(&run);
                    if url.len() > scheme.len() {
                        return Some(url.to_string());
                    }
                }
                i = end.max(i + 1);
            } else {
                i += 1;
            }
        }
    }
    None
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
                        t.reset_scroll();
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
            // We currently care about cursor visibility and the alt-screen modes:
            //     25: show/hide the terminal-owned cursor
            //   1049: save cursor + switch to alt screen (combined op)
            //   1047: switch to alt screen without explicit save (legacy)
            //     47: ditto, the original DEC alt-screen mode
            //
            // `?1049h/l` is the canonical "this is a TUI app" sequence
            // emitted by xterm-derived clients. We forward all three
            // variants to the same handler since many TUIs still send
            // the older aliases. `?25l` is equally important for TUI
            // prompts that hide the hardware cursor while drawing their
            // own input cursor; ignoring it produces a brief double cursor.
            // Other private modes (mouse reporting, application keypad,
            // etc.) are ignored here; the daemon owns those semantics.
            // Bracketed paste (2004) is tracked locally so `terminal.paste`
            // can wrap pasted bodies in `ESC[200~`/`ESC[201~`.
            'h' if intermediates == [b'?'] => {
                for &mode in &pv {
                    match mode {
                        25 => t.grid.set_cursor_visible(true),
                        2004 => t.bracketed_paste = true,
                        2026 => t.synchronized_output_active = true,
                        47 | 1047 | 1049 => t.enter_alt_screen(),
                        _ => {}
                    }
                }
            }
            'l' if intermediates == [b'?'] => {
                for &mode in &pv {
                    match mode {
                        25 => t.grid.set_cursor_visible(false),
                        2004 => t.bracketed_paste = false,
                        2026 => t.synchronized_output_active = false,
                        47 | 1047 | 1049 => t.exit_alt_screen(),
                        _ => {}
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
    fn process_bytes_at_bottom_stays_live() {
        let mut term = term_with_scrollback();
        assert_eq!(term.scroll_offset(), 0);
        term.process_bytes(b"FFFF\r\nGGGG");
        assert_eq!(
            term.scroll_offset(),
            0,
            "a live-bottom view must keep following output"
        );
        // The viewport shows the newest lines, exactly as before.
        assert_eq!(term.display_grid().get_cell(3, 0).unwrap().ch, 'G');
    }

    #[test]
    fn process_bytes_scrolled_back_anchors_viewport() {
        let mut term = term_with_scrollback();
        term.scroll_view_up(2);
        let top_before = term.abs_line_at_display(0);
        let top_content = term.display_grid().get_cell(1, 0).unwrap().ch;
        // Two more lines scroll off the live screen.
        term.process_bytes(b"\r\nFFFF\r\nGGGG");
        assert_eq!(
            term.abs_line_at_display(0),
            top_before,
            "output must not shift a scrolled-back viewport (gate S3)"
        );
        assert_eq!(
            term.display_grid().get_cell(1, 0).unwrap().ch,
            top_content,
            "the rendered top line must be unchanged across paints"
        );
        assert!(
            term.scroll_offset() > 2,
            "the offset grew to hold the anchor"
        );
    }

    #[test]
    fn enter_alt_screen_snaps_scrolled_back_view_to_live() {
        // Launching a TUI (?1049h) while reading scrollback must not
        // render scrollback above the alt screen: the anchoring gate
        // (S3) stopped snapping on output, so the buffer switch snaps.
        let mut term = term_with_scrollback();
        term.scroll_view_up(2);
        term.process_bytes(b"\x1b[?1049h");
        assert_eq!(term.scroll_offset(), 0, "alt-screen entry snaps to live");
        assert!(!term.is_view_scrolled());
        // The viewport shows the (blank) alt screen, not scrollback.
        assert_eq!(term.display_grid().get_cell(1, 0).unwrap().ch, '\0');
    }

    #[test]
    fn enter_alt_screen_cancels_in_flight_wheel_animation() {
        let mut term = term_with_scrollback();
        let t0 = Instant::now();
        assert!(term.scroll_animate_by_px(40.0, t0, Duration::from_millis(180), 0.25, 20.0));
        term.process_bytes(b"\x1b[?1049h");
        assert!(
            term.scroll_anim.is_none(),
            "the wheel animation is cancelled"
        );
        assert_eq!(term.fractional_view_position(), (0, 0.0));
    }

    #[test]
    fn exit_alt_screen_snaps_scrolled_view_to_live() {
        let mut term = term_with_scrollback();
        term.process_bytes(b"\x1b[?1049h");
        // Scroll back over the TUI (the main screen's scrollback is
        // still addressable), then exit: the view lands on the live
        // prompt, not a stale scrollback position.
        term.scroll_view_up(2);
        term.process_bytes(b"\x1b[?1049l");
        assert_eq!(term.scroll_offset(), 0, "alt-screen exit snaps to live");
        assert!(!term.is_view_scrolled());
    }

    #[test]
    fn reset_scroll_snaps_to_bottom() {
        let mut term = term_with_scrollback();
        term.scroll_view_up(2);
        term.reset_scroll();
        assert_eq!(term.scroll_offset(), 0);
    }

    #[test]
    fn scroll_view_by_lines_applies_exact_multiples_immediately() {
        let mut term = term_with_scrollback();
        assert_eq!(term.scroll_view_by_lines(2.0), 2);
        assert_eq!(term.scroll_offset(), 2);
        assert_eq!(term.scroll_view_by_lines(-2.0), -2);
        assert_eq!(term.scroll_offset(), 0);
    }

    #[test]
    fn scroll_view_by_lines_accumulates_sub_line_deltas() {
        let mut term = term_with_scrollback();
        assert_eq!(term.scroll_view_by_lines(0.3), 0);
        assert_eq!(term.scroll_view_by_lines(0.3), 0);
        // 0.3 + 0.3 + 0.5 crosses one whole line, leaving ~0.1 carry.
        assert_eq!(term.scroll_view_by_lines(0.5), 1);
        assert_eq!(term.scroll_offset(), 1);
        // The ~0.1 carry plus 0.5 stays below the next whole line.
        assert_eq!(term.scroll_view_by_lines(0.5), 0);
        assert_eq!(term.scroll_offset(), 1);
    }

    #[test]
    fn scroll_view_by_lines_sign_change_consumes_carry() {
        let mut term = term_with_scrollback();
        assert_eq!(term.scroll_view_by_lines(0.7), 0);
        // The opposite-direction nudge eats into the carry (now ~0.5)
        // without moving the view.
        assert_eq!(term.scroll_view_by_lines(-0.2), 0);
        assert_eq!(term.scroll_offset(), 0);
        // ~0.5 carry plus 0.6 crosses a whole line.
        assert_eq!(term.scroll_view_by_lines(0.6), 1);
        assert_eq!(term.scroll_offset(), 1);
    }

    #[test]
    fn scroll_view_by_lines_tracks_fractional_sums_over_random_sequence() {
        // Enough history that a 200-step walk bounded by +/-3 lines per
        // step (at most +/-600 total) can never clamp when started from
        // the midpoint of the scrollback.
        let mut term = Terminal::new(3, 5);
        let mut fill = String::new();
        for _ in 0..2000 {
            fill.push_str("x\r\n");
        }
        term.process_bytes(fill.as_bytes());
        let start = term.scrollback_len() / 2;
        assert!(start > 600, "walk needs head room, got {start}");
        term.scroll_view_up(start);

        // Deterministic LCG (Numerical Recipes constants) mapped to
        // [-3.0, 3.0) so the sequence is identical on every run.
        let mut state: u32 = 0xDEAD_BEEF;
        let mut next_delta = move || {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (state >> 8) as f32 / (1u32 << 24) as f32 * 6.0 - 3.0
        };

        // Mirror the accumulator math independently: every call must
        // apply exactly the whole part of the running fractional sum.
        let mut expected_carry = 0.0f32;
        let mut total_applied = 0i64;
        for step in 0..200 {
            let delta = next_delta();
            expected_carry += delta;
            let expected_whole = expected_carry.trunc() as i32;
            expected_carry -= expected_whole as f32;
            let applied = term.scroll_view_by_lines(delta);
            assert_eq!(applied, expected_whole, "step {step} delta {delta}");
            total_applied += i64::from(applied);
        }
        assert!(expected_carry.abs() < 1.0);
        assert_eq!(
            term.scroll_offset() as i64,
            start as i64 + total_applied,
            "offset must equal the start plus every applied whole line"
        );
    }

    #[test]
    fn scroll_view_by_lines_clamp_at_top_discards_carry() {
        let mut term = term_with_scrollback();
        let max = term.scrollback_len();
        // Requests 5 whole lines but only `max` exist: the view clamps
        // and the 0.9 carry is discarded.
        assert_eq!(term.scroll_view_by_lines(5.9), max as i32);
        assert_eq!(term.scroll_offset(), max);
        // Were the 0.9 carry still pending, two -0.5 nudges would sum
        // to -0.1 and the view would rubber-band in place.
        assert_eq!(term.scroll_view_by_lines(-0.5), 0);
        assert_eq!(term.scroll_view_by_lines(-0.5), -1);
        assert_eq!(term.scroll_offset(), max - 1);
    }

    #[test]
    fn scroll_view_by_lines_clamp_at_bottom_discards_carry() {
        let mut term = term_with_scrollback();
        // Already at the live screen: the down-scroll clamps at zero and
        // the -0.5 carry is discarded.
        assert_eq!(term.scroll_view_by_lines(-2.5), 0);
        assert_eq!(term.scroll_offset(), 0);
        // Were the -0.5 carry still pending, two +0.5 nudges would sum
        // to 0.5 and never cross a whole line.
        assert_eq!(term.scroll_view_by_lines(0.5), 0);
        assert_eq!(term.scroll_view_by_lines(0.5), 1);
        assert_eq!(term.scroll_offset(), 1);
    }

    #[test]
    fn reset_scroll_discards_fractional_carry() {
        let mut term = term_with_scrollback();
        assert_eq!(term.scroll_view_by_lines(0.9), 0);
        term.reset_scroll();
        // Were the 0.9 carry still pending, this 0.5 would cross a line.
        assert_eq!(term.scroll_view_by_lines(0.5), 0);
        assert_eq!(term.scroll_offset(), 0);
    }

    #[test]
    fn process_bytes_at_bottom_discards_dead_negative_carry() {
        let mut term = term_with_scrollback();
        // A negative carry at the live bottom renders as position 0
        // (dead input; only reachable defensively), and new output
        // discards it exactly like the old snap-to-live did.
        term.scroll_accum_lines = -0.4;
        assert_eq!(term.fractional_view_position(), (0, 0.0));
        term.process_bytes(b"FFFF\r\n");
        assert_eq!(term.scroll_offset(), 0);
        assert_eq!(term.scroll_accum_lines, 0.0);
        // Were the -0.4 still pending, 0.5 + 0.5 would not cross a line.
        assert_eq!(term.scroll_view_by_lines(0.5), 0);
        assert_eq!(term.scroll_view_by_lines(0.5), 1);
    }

    #[test]
    fn process_bytes_with_sub_row_carry_anchors_instead_of_snapping() {
        let mut term = term_with_scrollback();
        // A positive carry at offset 0 is a rendered sub-row position
        // (reading intent), so output anchors rather than snaps.
        assert_eq!(term.scroll_view_by_lines(0.6), 0);
        let (offset, fraction) = term.fractional_view_position();
        assert_eq!((offset, fraction), (0, 0.6));
        // The newline scrolls exactly one line into scrollback.
        term.process_bytes(b"\r\nFFFF");
        let (offset, fraction) = term.fractional_view_position();
        assert_eq!(offset, 1, "one pushed line bumps the anchor by one row");
        assert!(
            (fraction - 0.6).abs() < 1e-6,
            "the sub-row fraction survives"
        );
    }

    // -- animated wheel scrolling tests (Phase 3 Stage T) ---------------------

    const CELL_H: f32 = 20.0;

    /// 3x5 terminal with `lines` numbered lines pushed into scrollback.
    fn term_with_deep_scrollback(lines: usize) -> Terminal {
        let mut term = Terminal::new(3, 5);
        let mut fill = String::new();
        for i in 0..lines {
            fill.push_str(&format!("{}\r\n", i % 10));
        }
        fill.push_str("end");
        term.process_bytes(fill.as_bytes());
        term
    }

    #[test]
    fn scroll_animate_target_accumulates_exactly_per_notch() {
        // Gate H5 at the target level: N notches move the animation
        // target by exactly N * delta_px (wheel-train compounding off
        // the previous target, no per-event rounding).
        let mut term = term_with_deep_scrollback(100);
        let t0 = Instant::now();
        let d = Duration::from_millis(180);
        for n in 1..=3 {
            assert!(term.scroll_animate_by_px(
                120.0,
                t0 + Duration::from_millis(20 * n as u64),
                d,
                0.25,
                CELL_H
            ));
            assert_eq!(term.scroll_anim.unwrap().target, 120.0 * n as f32);
        }
    }

    #[test]
    fn scroll_animate_target_clamps_to_scrollback_extent() {
        let mut term = term_with_deep_scrollback(10);
        let sb_len = term.scrollback_len();
        let max_px = sb_len as f32 * CELL_H;
        let t0 = Instant::now();
        assert!(term.scroll_animate_by_px(
            max_px * 4.0,
            t0,
            Duration::from_millis(180),
            0.25,
            CELL_H
        ));
        assert_eq!(term.scroll_anim.unwrap().target, max_px);
        // Completion pins exactly to the top: whole offset, no fraction.
        let (changed, finished) = term
            .sample_scroll_animation(t0 + Duration::from_millis(200), CELL_H)
            .unwrap();
        assert!(changed && finished);
        assert_eq!(term.fractional_view_position(), (sb_len, 0.0));
        assert!(term.scroll_anim.is_none());
    }

    #[test]
    fn scroll_animate_downward_at_live_bottom_is_a_no_op() {
        let mut term = term_with_deep_scrollback(10);
        assert!(!term.scroll_animate_by_px(
            -100.0,
            Instant::now(),
            Duration::from_millis(180),
            0.25,
            CELL_H
        ));
        assert!(term.scroll_anim.is_none());
        assert_eq!(term.fractional_view_position(), (0, 0.0));
    }

    #[test]
    fn scroll_animate_rejects_non_finite_inputs() {
        let mut term = term_with_deep_scrollback(10);
        let now = Instant::now();
        let d = Duration::from_millis(180);
        assert!(!term.scroll_animate_by_px(f32::NAN, now, d, 0.25, CELL_H));
        assert!(!term.scroll_animate_by_px(f32::INFINITY, now, d, 0.25, CELL_H));
        assert!(!term.scroll_animate_by_px(100.0, now, d, f32::NAN, CELL_H));
        assert!(!term.scroll_animate_by_px(100.0, now, d, 0.25, 0.0));
        assert!(!term.scroll_animate_by_px(100.0, now, d, 0.25, f32::NAN));
        assert!(term.scroll_anim.is_none());
        assert_eq!(term.fractional_view_position(), (0, 0.0));
    }

    #[test]
    fn scroll_animate_folds_parked_touchpad_carry_into_start() {
        let mut term = term_with_deep_scrollback(50);
        assert_eq!(term.scroll_view_by_lines(0.5), 0);
        let t0 = Instant::now();
        assert!(term.scroll_animate_by_px(40.0, t0, Duration::from_millis(180), 0.25, CELL_H));
        let motion = term.scroll_anim.unwrap();
        assert_eq!(
            motion.start,
            0.5 * CELL_H,
            "carry folds into the start position"
        );
        assert_eq!(motion.target, 0.5 * CELL_H + 40.0);
        assert_eq!(
            term.scroll_accum_lines, 0.0,
            "the motion now owns the fraction"
        );
    }

    #[test]
    fn sample_scroll_animation_maps_px_to_whole_rows_plus_fraction() {
        let mut term = term_with_deep_scrollback(50);
        let t0 = Instant::now();
        // Slope 0 over 100ms: the symmetric ease puts the midpoint at
        // exactly 50% of the 50px distance = 1.25 lines.
        assert!(term.scroll_animate_by_px(50.0, t0, Duration::from_millis(100), 0.0, CELL_H));
        let (changed, finished) = term
            .sample_scroll_animation(t0 + Duration::from_millis(50), CELL_H)
            .unwrap();
        assert!(changed && !finished);
        let (offset, fraction) = term.fractional_view_position();
        assert_eq!(offset, 1);
        assert!((fraction - 0.25).abs() < 1e-3, "fraction was {fraction}");
    }

    #[test]
    fn animation_completion_lands_exactly_on_target_with_resting_fraction() {
        let mut term = term_with_deep_scrollback(50);
        let t0 = Instant::now();
        // 50px = 2.5 lines: the resting position is fractional.
        assert!(term.scroll_animate_by_px(50.0, t0, Duration::from_millis(100), 0.25, CELL_H));
        let just_before = term
            .sample_scroll_animation(t0 + Duration::from_millis(99), CELL_H)
            .unwrap();
        assert!(!just_before.1);
        let rendered_before_done = term.rendered_scroll_px(CELL_H);
        let (_, finished) = term
            .sample_scroll_animation(t0 + Duration::from_millis(100), CELL_H)
            .unwrap();
        assert!(finished);
        assert!(term.scroll_anim.is_none());
        // The resting fraction was handed to the carry: the view renders
        // 2.5 lines back from the non-animating state.
        assert_eq!(term.scroll_offset(), 2);
        assert!((term.scroll_accum_lines - 0.5).abs() < 1e-6);
        assert_eq!(term.rendered_scroll_px(CELL_H), (2, -10));
        // No end-of-fling jerk: the final px position equals the target,
        // which the pre-completion samples were converging to.
        assert!(rendered_before_done.0 <= 2);
        // A follow-up sample reports no animation.
        assert!(term
            .sample_scroll_animation(t0 + Duration::from_millis(120), CELL_H)
            .is_none());
    }

    #[test]
    fn wheel_retarget_mid_flight_is_velocity_continuous() {
        let mut term = term_with_deep_scrollback(100);
        let t0 = Instant::now();
        let d = Duration::from_millis(180);
        assert!(term.scroll_animate_by_px(120.0, t0, d, 0.25, CELL_H));
        let first = term.scroll_anim.unwrap();
        let t1 = t0 + Duration::from_millis(40);
        let (mid_pos, _, _) = first.sample(t1);
        assert!(term.scroll_animate_by_px(120.0, t1, d, 0.25, CELL_H));
        let second = term.scroll_anim.unwrap();
        assert_eq!(
            second.start, mid_pos,
            "the retarget starts from the sampled position"
        );
        assert_eq!(
            second.target, 240.0,
            "the retarget compounds off the previous target"
        );
        assert!(
            second.initial_slope > 0.25,
            "in-flight velocity must carry into the new curve"
        );
    }

    #[test]
    fn epsilon_retarget_collapses_animation_with_one_settling_repaint() {
        let mut term = term_with_deep_scrollback(50);
        let t0 = Instant::now();
        let d = Duration::from_millis(180);
        assert!(term.scroll_animate_by_px(100.0, t0, d, 0.25, CELL_H));
        // An equal-and-opposite notch at the same instant collapses the
        // compounded target back onto the current position (epsilon
        // no-op): the animation clears but one repaint is still due.
        assert!(term.scroll_animate_by_px(-100.0, t0, d, 0.25, CELL_H));
        assert!(term.scroll_anim.is_none());
        assert_eq!(term.fractional_view_position(), (0, 0.0));
        // And with no animation at all, the same call reports nothing.
        assert!(!term.scroll_animate_by_px(-100.0, t0, d, 0.25, CELL_H));
    }

    #[test]
    fn one_notch_renders_at_least_twelve_distinct_positions_at_8ms_cadence() {
        // Gate H4 at the terminal level: a single notch sampled on the
        // 8ms animation cadence must pass through >= 12 distinct
        // device-pixel positions and land exactly on its target.
        use unshit::app::scroll_motion::{browser_like_initial_slope, browser_like_wheel_duration};
        let mut term = term_with_deep_scrollback(100);
        let delta = (0.0, 120.0);
        let duration = browser_like_wheel_duration(delta, unshit::app::ScrollTuning::default());
        let slope = browser_like_initial_slope(delta);
        let cell_h = 40.0;
        let t0 = Instant::now();
        assert!(term.scroll_animate_by_px(120.0, t0, duration, slope, cell_h));

        let mut positions = vec![term.rendered_scroll_px(cell_h)];
        let mut tick = t0;
        loop {
            tick += Duration::from_millis(8);
            let (_, finished) = term.sample_scroll_animation(tick, cell_h).unwrap();
            let rendered = term.rendered_scroll_px(cell_h);
            if positions.last() != Some(&rendered) {
                positions.push(rendered);
            }
            if finished {
                break;
            }
        }
        assert!(
            positions.len() >= 12,
            "expected >= 12 distinct rendered positions per notch, got {}",
            positions.len()
        );
        assert_eq!(
            term.fractional_view_position(),
            (3, 0.0),
            "120px at cell_h 40 must land exactly 3 lines back"
        );
    }

    #[test]
    fn touchpad_takeover_collapses_animation_to_current_sample() {
        let mut term = term_with_deep_scrollback(50);
        let t0 = Instant::now();
        assert!(term.scroll_animate_by_px(50.0, t0, Duration::from_millis(100), 0.0, CELL_H));
        term.sample_scroll_animation(t0 + Duration::from_millis(50), CELL_H);
        let (offset, fraction) = term.fractional_view_position();
        // A touchpad delta cancels the animation and continues from the
        // sampled position exactly.
        term.scroll_view_by_lines(0.2);
        assert!(term.scroll_anim.is_none());
        let (offset2, fraction2) = term.fractional_view_position();
        assert_eq!(offset2, offset);
        assert!((fraction2 - (fraction + 0.2)).abs() < 1e-5);
    }

    #[test]
    fn page_jump_mid_animation_collapses_then_applies_whole_lines() {
        let mut term = term_with_deep_scrollback(50);
        let t0 = Instant::now();
        assert!(term.scroll_animate_by_px(50.0, t0, Duration::from_millis(100), 0.0, CELL_H));
        term.sample_scroll_animation(t0 + Duration::from_millis(50), CELL_H);
        let (offset, fraction) = term.fractional_view_position();
        term.scroll_view_up(5);
        assert!(
            term.scroll_anim.is_none(),
            "a direct jump cancels the animation"
        );
        let (offset2, fraction2) = term.fractional_view_position();
        assert_eq!(offset2, offset + 5);
        assert!(
            (fraction2 - fraction).abs() < 1e-6,
            "the sampled fraction survives the jump"
        );
    }

    #[test]
    fn reset_scroll_cancels_animation_and_fraction() {
        let mut term = term_with_deep_scrollback(50);
        let t0 = Instant::now();
        assert!(term.scroll_animate_by_px(50.0, t0, Duration::from_millis(100), 0.0, CELL_H));
        term.sample_scroll_animation(t0 + Duration::from_millis(50), CELL_H);
        term.reset_scroll();
        assert!(term.scroll_anim.is_none());
        assert_eq!(term.fractional_view_position(), (0, 0.0));
        assert!(term
            .sample_scroll_animation(t0 + Duration::from_millis(60), CELL_H)
            .is_none());
    }

    #[test]
    fn zoom_mid_flight_rescales_animation_in_line_space() {
        // R1: a cell-height change mid-animation rescales the motion's
        // pixel space so the line-space position stays continuous.
        let mut term = term_with_deep_scrollback(50);
        let mut control = term_with_deep_scrollback(50);
        let t0 = Instant::now();
        let d = Duration::from_millis(100);
        assert!(term.scroll_animate_by_px(50.0, t0, d, 0.0, CELL_H));
        assert!(control.scroll_animate_by_px(50.0, t0, d, 0.0, CELL_H));
        let t1 = t0 + Duration::from_millis(50);
        // Zoom doubles the cell height for the test terminal only.
        term.sample_scroll_animation(t1, CELL_H * 2.0);
        control.sample_scroll_animation(t1, CELL_H);
        let (offset, fraction) = term.fractional_view_position();
        let (c_offset, c_fraction) = control.fractional_view_position();
        assert_eq!(offset, c_offset);
        assert!((fraction - c_fraction).abs() < 1e-3);
        let motion = term.scroll_anim.unwrap();
        assert_eq!(
            motion.target, 100.0,
            "the target scaled with the zoom ratio"
        );
    }

    #[test]
    fn pty_output_anchors_animated_view_by_shifting_motion() {
        let mut term = term_with_deep_scrollback(50);
        let t0 = Instant::now();
        assert!(term.scroll_animate_by_px(100.0, t0, Duration::from_millis(100), 0.0, CELL_H));
        let t1 = t0 + Duration::from_millis(50);
        term.sample_scroll_animation(t1, CELL_H);
        let top_before = term.abs_line_at_display(0);
        let target_before = term.scroll_anim.unwrap().target;
        // One line of output while reading scrollback.
        term.process_bytes(b"\r\nX");
        assert_eq!(
            term.abs_line_at_display(0),
            top_before,
            "anchoring must keep the same content at the viewport top"
        );
        let motion = term.scroll_anim.unwrap();
        assert_eq!(motion.target, target_before + CELL_H);
        // Re-sampling at the same timestamp reproduces the anchored view.
        term.sample_scroll_animation(t1, CELL_H);
        assert_eq!(term.abs_line_at_display(0), top_before);
    }

    #[test]
    fn sub_half_pixel_landing_keeps_follow_output() {
        // A wheel train that nets out to less than half a device pixel
        // off the live bottom renders identically to the live bottom, so
        // PTY output must keep following instead of anchoring; the dead
        // residue is discarded by the next output chunk.
        let mut term = term_with_deep_scrollback(50);
        let t0 = Instant::now();
        let d = Duration::from_millis(100);
        assert!(term.scroll_animate_by_px(20.4, t0, d, 0.0, CELL_H));
        term.sample_scroll_animation(t0 + d, CELL_H);
        assert!(term.scroll_animate_by_px(-20.0, t0 + d, d, 0.0, CELL_H));
        term.sample_scroll_animation(t0 + d + d, CELL_H);
        assert!(term.scroll_anim.is_none());
        let (offset, fraction) = term.fractional_view_position();
        assert_eq!(offset, 0);
        assert!(
            fraction > 0.0 && fraction * CELL_H < 0.5,
            "the landing left a sub-half-pixel residue, fraction = {fraction}"
        );
        term.process_bytes(b"\r\nX");
        assert_eq!(term.scroll_offset(), 0, "follow-output stays enabled");
        assert_eq!(
            term.scroll_accum_lines, 0.0,
            "the dead residue is discarded"
        );
    }

    #[test]
    fn render_offset_is_whole_device_pixels_for_fractional_cell_heights() {
        // The paint-time translation must always be a whole device pixel,
        // even when the cell height is fractional (e.g. 14pt * 1.25 line
        // height = 17.5px): cell quads are pixel-snapped and glyph
        // subpixel bins chosen at the pre-translation position, so a
        // fractional offset would knock every frame (idle included) off
        // its rasterized phase. `render_offset_px` returning i64 makes
        // wholeness structural; this pins the rounding of the whole
        // offset, not just the animated component.
        assert_eq!(Terminal::render_offset_px(0.0, 17.5), -18);
        assert_eq!(Terminal::render_offset_px(0.5, 17.5), -9);
        assert_eq!(Terminal::render_offset_px(0.0, 20.0), -20);
        // The change detector and the painted offset share one
        // quantization, so equal detector values paint identically.
        let mut term = term_with_deep_scrollback(10);
        term.scroll_view_by_lines(0.5);
        let (_, px) = term.rendered_scroll_px(17.5);
        assert_eq!(px, Terminal::render_offset_px(0.5, 17.5));
    }

    #[test]
    fn anchoring_survives_eviction_at_scrollback_capacity() {
        let mut term = Terminal::new(2, 3);
        for _ in 0..MAX_SCROLLBACK {
            term.scrollback.push_back(vec![Cell::default(); 3]);
        }
        term.process_bytes(b"A\r\nB");
        // Pushes above capacity evict from the front.
        assert_eq!(term.scrollback_len(), MAX_SCROLLBACK);
        term.scroll_view_up(10);
        let top_before = term.abs_line_at_display(0);
        let row_content = term.display_grid().get_cell(1, 0).unwrap().ch;
        term.process_bytes(b"\r\nC\r\nD");
        assert_eq!(term.scrollback_len(), MAX_SCROLLBACK);
        assert_eq!(
            term.abs_line_at_display(0),
            top_before,
            "the anchor survives eviction until the view is pinned at the top"
        );
        assert_eq!(term.display_grid().get_cell(1, 0).unwrap().ch, row_content);
        assert_eq!(term.scroll_offset(), 12);
    }

    #[test]
    fn anchoring_pinned_at_top_lets_eviction_consume_the_view() {
        let mut term = Terminal::new(2, 3);
        for _ in 0..MAX_SCROLLBACK {
            term.scrollback.push_back(vec![Cell::default(); 3]);
        }
        term.process_bytes(b"A\r\nB");
        term.scroll_view_up(MAX_SCROLLBACK * 2);
        assert_eq!(term.scroll_offset(), MAX_SCROLLBACK);
        let top_before = term.abs_line_at_display(0);
        term.process_bytes(b"\r\nC");
        // Content above was destroyed; the clamped offset means the view
        // advances one line (nothing else is possible).
        assert_eq!(term.scroll_offset(), MAX_SCROLLBACK);
        assert_eq!(term.abs_line_at_display(0), top_before + 1);
    }

    #[test]
    fn downward_fling_to_live_skips_anchoring() {
        let mut term = term_with_deep_scrollback(50);
        term.scroll_view_up(5);
        let t0 = Instant::now();
        // Fling toward the live bottom: target clamps to 0.
        assert!(term.scroll_animate_by_px(
            -10.0 * CELL_H,
            t0,
            Duration::from_millis(100),
            0.0,
            CELL_H
        ));
        assert_eq!(term.scroll_anim.unwrap().target, 0.0);
        let offset_before = term.scroll_offset();
        term.process_bytes(b"\r\nX");
        assert_eq!(
            term.scroll_offset(),
            offset_before,
            "output must not anchor a fling whose stated intent is the live bottom"
        );
        assert_eq!(term.scroll_anim.unwrap().target, 0.0);
        // The fling still lands at the live screen.
        let (_, finished) = term
            .sample_scroll_animation(t0 + Duration::from_millis(120), CELL_H)
            .unwrap();
        assert!(finished);
        assert_eq!(term.fractional_view_position(), (0, 0.0));
    }

    #[test]
    fn wheel_then_touchpad_position_is_exact_clamped_sum() {
        let mut term = term_with_deep_scrollback(50);
        let t0 = Instant::now();
        // 45px notch = 2.25 lines, run to completion.
        assert!(term.scroll_animate_by_px(45.0, t0, Duration::from_millis(100), 0.25, CELL_H));
        term.sample_scroll_animation(t0 + Duration::from_millis(150), CELL_H);
        // Touchpad adds half a line.
        term.scroll_view_by_lines(0.5);
        let (offset, fraction) = term.fractional_view_position();
        assert_eq!(offset, 2);
        assert!(
            (fraction - 0.75).abs() < 1e-5,
            "2.25 + 0.5 lines must render as (2, 0.75), got (2, {fraction})"
        );
    }

    #[test]
    fn fractional_view_position_normalizes_negative_carry() {
        let mut term = term_with_deep_scrollback(50);
        term.scroll_view_up(2);
        assert_eq!(term.scroll_view_by_lines(-0.25), 0);
        assert_eq!(
            term.scroll_offset(),
            2,
            "whole offset is untouched by a sub-line delta"
        );
        let (offset, fraction) = term.fractional_view_position();
        assert_eq!(offset, 1, "a negative carry borrows a row from the offset");
        assert!((fraction - 0.75).abs() < 1e-6);
    }

    #[test]
    fn is_view_scrolled_covers_rows_fractions_and_animations() {
        let mut term = term_with_deep_scrollback(50);
        assert!(!term.is_view_scrolled());
        term.scroll_view_by_lines(0.5);
        assert!(
            term.is_view_scrolled(),
            "a sub-row fraction counts as scrolled"
        );
        term.reset_scroll();
        assert!(!term.is_view_scrolled());
        assert!(term.scroll_animate_by_px(
            50.0,
            Instant::now(),
            Duration::from_millis(180),
            0.25,
            CELL_H
        ));
        assert!(
            term.is_view_scrolled(),
            "an in-flight animation counts as scrolled"
        );
    }

    #[test]
    fn view_pixel_to_abs_line_maps_whole_and_fractional_positions() {
        let mut term = Terminal::new(2, 10);
        term.process_bytes(b"L0\r\nL1\r\nL2\r\nL3");
        // Live bottom: top abs line is 2.
        assert_eq!(term.view_pixel_to_abs_line(0.0, CELL_H), 2);
        assert_eq!(term.view_pixel_to_abs_line(CELL_H * 1.5, CELL_H), 3);
        // Below the last row clamps onto it; negative clamps to the top.
        assert_eq!(term.view_pixel_to_abs_line(CELL_H * 10.0, CELL_H), 3);
        assert_eq!(term.view_pixel_to_abs_line(-5.0, CELL_H), 2);
        // Degenerate metrics fall back to the top line.
        assert_eq!(term.view_pixel_to_abs_line(35.0, 0.0), 2);

        // Scrolled half a row back (offset 1, fraction 0.5): the content
        // is displaced down, exposing the overscan line (abs 0) at the
        // top edge.
        term.scroll_view_up(1);
        term.scroll_view_by_lines(0.5);
        let (offset, fraction) = term.fractional_view_position();
        assert_eq!((offset, fraction), (1, 0.5));
        assert_eq!(
            term.view_pixel_to_abs_line(0.0, CELL_H),
            0,
            "overscan line at the top edge"
        );
        assert_eq!(term.view_pixel_to_abs_line(CELL_H * 0.6, CELL_H), 1);
        assert_eq!(term.view_pixel_to_abs_line(CELL_H * 1.6, CELL_H), 2);
    }

    #[test]
    fn pinned_at_bottom_discards_sub_line_carry_into_boundary() {
        let mut term = term_with_scrollback();
        assert_eq!(term.scroll_offset(), 0);
        // Three downward sub-line nudges while already pinned at the
        // bottom are dead input: each carry is discarded immediately.
        assert_eq!(term.scroll_view_by_lines(-0.4), 0);
        assert_eq!(term.scroll_view_by_lines(-0.4), 0);
        assert_eq!(term.scroll_view_by_lines(-0.4), 0);
        // The reversal behaves as if the carry were zero: 0.5 neither
        // jumps a line instantly nor gets swallowed by -1.2 of dead carry.
        assert_eq!(term.scroll_view_by_lines(0.5), 0);
        assert_eq!(term.scroll_view_by_lines(0.5), 1);
        assert_eq!(term.scroll_offset(), 1);
    }

    #[test]
    fn scroll_view_by_lines_nan_delta_is_ignored() {
        let mut term = term_with_scrollback();
        assert_eq!(term.scroll_view_by_lines(0.4), 0);
        assert_eq!(term.scroll_view_by_lines(f32::NAN), 0);
        // Subsequent calls are not poisoned: the 0.4 carry survives and
        // 0.4 + 0.6 crosses exactly one line.
        assert_eq!(term.scroll_view_by_lines(0.6), 1);
        assert_eq!(term.scroll_offset(), 1);
    }

    #[test]
    fn scroll_view_by_lines_infinite_delta_is_ignored_and_carry_stays_finite() {
        let mut term = term_with_scrollback();
        assert_eq!(term.scroll_view_by_lines(f32::NEG_INFINITY), 0);
        assert_eq!(term.scroll_offset(), 0);
        // The accumulator stayed finite: half-line nudges still sum up
        // normally instead of being absorbed by -inf.
        assert_eq!(term.scroll_view_by_lines(0.5), 0);
        assert_eq!(term.scroll_view_by_lines(0.5), 1);
        assert_eq!(term.scroll_offset(), 1);
    }

    #[test]
    fn display_grid_at_bottom_matches_live_grid() {
        let term = term_with_scrollback();
        let live = term.grid().clone();
        let display = term.display_grid();
        // One overscan row is prepended above the viewport, so the live
        // rows appear shifted down by one grid index (the renderer's
        // `render_offset_y` translation puts them back).
        for row in 0..term.rows {
            for col in 0..term.cols {
                let live_cell = live.get_cell(row, col).unwrap();
                let disp_cell = display.get_cell(row + 1, col).unwrap();
                assert_eq!(
                    live_cell.ch, disp_cell.ch,
                    "display_grid at bottom should match live grid at ({},{})",
                    row, col
                );
            }
        }
    }

    #[test]
    fn display_grid_always_has_one_overscan_row() {
        let mut term = term_with_scrollback();
        // Live and scrolled snapshots keep the same (rows + 1) shape so
        // paint-only grid patches never see a dimension change.
        let live = term.display_grid();
        assert_eq!(live.rows(), term.rows + 1);
        assert_eq!(live.overscan_rows(), 1);
        term.scroll_view_up(1);
        let scrolled = term.display_grid();
        assert_eq!(scrolled.rows(), term.rows + 1);
        assert_eq!(scrolled.overscan_rows(), 1);
    }

    #[test]
    fn display_grid_live_overscan_row_is_newest_scrollback_line() {
        let term = term_with_scrollback();
        // Scrollback holds AAAA, BBBB; the overscan row above the live
        // viewport must be the newest one (BBBB).
        let display = term.display_grid();
        assert_eq!(display.get_cell(0, 0).unwrap().ch, 'B');
    }

    #[test]
    fn display_grid_live_overscan_row_is_blank_without_scrollback() {
        let mut term = Terminal::new(3, 5);
        term.process_bytes(b"XXXX");
        let display = term.display_grid();
        assert_eq!(display.get_cell(0, 0).unwrap().ch, '\0');
        assert_eq!(display.get_cell(1, 0).unwrap().ch, 'X');
    }

    #[test]
    fn display_grid_scrolled_shows_scrollback_content() {
        let mut term = term_with_scrollback();
        // Scroll all the way back (2 lines of scrollback).
        term.scroll_view_up(2);
        let display = term.display_grid();

        // Row 0 is the overscan row; pinned at the top of scrollback no
        // line above exists, so it is blank.
        let ch = display.get_cell(0, 0).unwrap().ch;
        assert_eq!(ch, '\0', "overscan row at the top must be blank");

        // Viewport row 0 (grid row 1) shows the first scrollback line.
        let ch = display.get_cell(1, 0).unwrap().ch;
        assert_eq!(
            ch, 'A',
            "scrolled-back viewport row 0 should be 'A', got '{}'",
            ch
        );

        // Viewport row 1 shows the second scrollback line (BBBB).
        let ch = display.get_cell(2, 0).unwrap().ch;
        assert_eq!(
            ch, 'B',
            "scrolled-back viewport row 1 should be 'B', got '{}'",
            ch
        );

        // Viewport row 2 shows the first screen row (CCCC).
        let ch = display.get_cell(3, 0).unwrap().ch;
        assert_eq!(
            ch, 'C',
            "scrolled-back viewport row 2 should be 'C', got '{}'",
            ch
        );
    }

    #[test]
    fn display_grid_partially_scrolled_overscan_row_holds_line_above() {
        let mut term = term_with_scrollback();
        // Offset 1 of 2: the overscan row above the viewport is the
        // older scrollback line (AAAA).
        term.scroll_view_up(1);
        let display = term.display_grid();
        assert_eq!(display.get_cell(0, 0).unwrap().ch, 'A');
        assert_eq!(display.get_cell(1, 0).unwrap().ch, 'B');
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
    fn dec_private_25_toggles_cursor_visibility() {
        let mut term = Terminal::new(3, 5);
        assert!(term.grid().cursor_visible());

        term.process_bytes(b"\x1b[?25l");
        assert!(
            !term.grid().cursor_visible(),
            "CSI ?25l must hide the terminal-owned cursor"
        );

        term.process_bytes(b"\x1b[?25h");
        assert!(
            term.grid().cursor_visible(),
            "CSI ?25h must restore the terminal-owned cursor"
        );
    }

    #[test]
    fn synchronized_output_mode_tracks_dec_private_2026() {
        let mut term = Terminal::new(3, 5);
        assert!(!term.synchronized_output_active());

        term.process_bytes(b"\x1b[?2026h");
        assert!(
            term.synchronized_output_active(),
            "CSI ?2026h must enter synchronized output mode"
        );

        term.process_bytes(b"\x1b[?2026l");
        assert!(
            !term.synchronized_output_active(),
            "CSI ?2026l must leave synchronized output mode"
        );
    }

    #[test]
    fn bracketed_paste_mode_tracks_dec_private_2004() {
        let mut term = Terminal::new(3, 5);
        assert!(!term.bracketed_paste_active());

        term.process_bytes(b"\x1b[?2004h");
        assert!(
            term.bracketed_paste_active(),
            "CSI ?2004h must enable bracketed paste mode"
        );

        term.process_bytes(b"\x1b[?2004l");
        assert!(
            !term.bracketed_paste_active(),
            "CSI ?2004l must disable bracketed paste mode"
        );
    }

    #[test]
    fn selection_text_reads_absolute_lines_across_scrollback() {
        let mut term = Terminal::new(2, 10);
        term.process_bytes(b"line0\r\nline1\r\nline2\r\nline3");
        // line0/line1 scrolled into scrollback (off the live screen), but
        // selection_text addresses them by absolute line regardless of view.
        assert_eq!(term.selection_text((0, 0), (0, 4)), "line0");
        assert_eq!(term.selection_text((3, 0), (3, 4)), "line3");
        // The live view shows the last two absolute lines.
        assert_eq!(term.abs_line_at_display(0), 2);
        assert_eq!(term.abs_line_at_display(1), 3);
        // Scrolling the view back does not change which absolute line maps
        // to a given display row's content; line0 becomes visible at row 0.
        term.scroll_view_up(2);
        assert_eq!(term.abs_line_at_display(0), 0);
        assert_eq!(term.selection_text((0, 0), (0, 4)), "line0");
    }

    #[test]
    fn paint_selection_follows_content_when_view_scrolls() {
        use unshit::core::style::types::Color;
        let bg = Color::rgb(1, 2, 3);
        let mut term = Terminal::new(2, 10);
        term.process_bytes(b"line0\r\nline1\r\nline2\r\nline3");
        // Select absolute line 0 ("line0"), which is currently scrolled out
        // of the live view (and out of the overscan row, which shows
        // line1): nothing should paint.
        let mut live = term.display_grid();
        term.paint_selection(&mut live, (0, 0), (0, 4), bg);
        assert!(
            (0..10).all(|c| (0..3).all(|r| live.get_cell(r, c).unwrap().bg != bg)),
            "an off-screen selection must not paint the live view"
        );
        // Scroll line0 into view at viewport row 0 (grid row 1); now it
        // paints there.
        term.scroll_view_up(2);
        let mut scrolled = term.display_grid();
        term.paint_selection(&mut scrolled, (0, 0), (0, 4), bg);
        for c in 0..=4 {
            assert_eq!(scrolled.get_cell(1, c).unwrap().bg, bg, "col {c} of line0");
        }
    }

    #[test]
    fn paint_selection_paints_partially_visible_overscan_line() {
        use unshit::core::style::types::Color;
        let bg = Color::rgb(9, 8, 7);
        let mut term = Terminal::new(2, 10);
        term.process_bytes(b"line0\r\nline1\r\nline2\r\nline3");
        // At the live bottom the overscan row shows line1 (the newest
        // scrollback line); selecting it must paint grid row 0 so a
        // sub-row scroll position never clips the highlight.
        let mut grid = term.display_grid();
        term.paint_selection(&mut grid, (1, 0), (1, 4), bg);
        for c in 0..=4 {
            assert_eq!(
                grid.get_cell(0, c).unwrap().bg,
                bg,
                "col {c} of overscan line1"
            );
        }
    }

    #[test]
    fn selection_text_single_row_trims_trailing_blanks() {
        let mut term = Terminal::new(2, 10);
        term.process_bytes(b"hello");
        // Selecting the whole row trims the trailing blank cells.
        assert_eq!(term.selection_text((0, 0), (0, 9)), "hello");
        // A sub-range returns exactly the spanned columns (inclusive).
        assert_eq!(term.selection_text((0, 0), (0, 3)), "hell");
    }

    #[test]
    fn selection_text_is_order_independent() {
        let mut term = Terminal::new(2, 10);
        term.process_bytes(b"abcdef");
        assert_eq!(
            term.selection_text((0, 4), (0, 1)),
            term.selection_text((0, 1), (0, 4)),
        );
        assert_eq!(term.selection_text((0, 1), (0, 4)), "bcde");
    }

    #[test]
    fn selection_text_multi_row_joins_with_newline() {
        let mut term = Terminal::new(3, 10);
        term.process_bytes(b"line1\r\nline2");
        // First row runs to end of line (trailing blanks trimmed), last row
        // stops at the end column.
        assert_eq!(term.selection_text((0, 0), (1, 4)), "line1\nline2");
    }

    #[test]
    fn word_bounds_selects_whole_token() {
        let mut term = Terminal::new(2, 20);
        term.process_bytes(b"foo bar baz");
        assert_eq!(term.word_bounds_at(0, 1), (0, 2), "inside 'foo'");
        assert_eq!(term.word_bounds_at(0, 4), (4, 6), "inside 'bar'");
        // A click on the separating space selects just that cell.
        assert_eq!(term.word_bounds_at(0, 3), (3, 3), "on the space");
    }

    #[test]
    fn word_bounds_includes_path_punctuation() {
        let mut term = Terminal::new(2, 30);
        term.process_bytes(b"see /usr/local/bin here");
        // The path token spans the slashes, not just one segment.
        assert_eq!(term.word_bounds_at(0, 8), (4, 17));
    }

    #[test]
    fn line_bounds_cover_full_row() {
        let term = Terminal::new(2, 12);
        assert_eq!(term.line_bounds_at(0), (0, 11));
    }

    #[test]
    fn url_at_detects_https_link_under_click() {
        let mut term = Terminal::new(2, 40);
        term.process_bytes(b"go to https://example.com/path now");
        // Click anywhere inside the URL span (cols 6..30).
        assert_eq!(
            term.url_at(0, 6).as_deref(),
            Some("https://example.com/path")
        );
        assert_eq!(
            term.url_at(0, 20).as_deref(),
            Some("https://example.com/path")
        );
    }

    #[test]
    fn url_at_returns_none_off_the_link() {
        let mut term = Terminal::new(2, 40);
        term.process_bytes(b"go to https://example.com now");
        // "go" at col 0 is not a link, and the trailing "now" is not either.
        assert_eq!(term.url_at(0, 0), None);
        assert_eq!(term.url_at(0, 27), None);
    }

    #[test]
    fn url_at_preserves_query_and_fragment() {
        let mut term = Terminal::new(2, 60);
        term.process_bytes(b"open https://ex.com/a?b=c&d=e#frag end");
        assert_eq!(
            term.url_at(0, 10).as_deref(),
            Some("https://ex.com/a?b=c&d=e#frag")
        );
    }

    #[test]
    fn url_at_trims_trailing_sentence_punctuation() {
        let mut term = Terminal::new(2, 50);
        term.process_bytes(b"see (http://example.com).");
        // The click lands inside the link; the wrapping ")." is stripped but
        // a balanced parenthesis inside a path is kept.
        assert_eq!(term.url_at(0, 8).as_deref(), Some("http://example.com"));

        let mut wiki = Terminal::new(2, 60);
        wiki.process_bytes(b"http://en.wikipedia.org/wiki/Foo_(bar)");
        assert_eq!(
            wiki.url_at(0, 5).as_deref(),
            Some("http://en.wikipedia.org/wiki/Foo_(bar)")
        );
    }

    #[test]
    fn url_at_ignores_non_http_schemes() {
        let mut term = Terminal::new(2, 40);
        term.process_bytes(b"file:///etc/passwd ftp://h/x");
        assert_eq!(term.url_at(0, 2), None);
        assert_eq!(term.url_at(0, 22), None);
    }

    #[test]
    fn url_at_bare_scheme_is_not_a_link() {
        let mut term = Terminal::new(2, 20);
        term.process_bytes(b"https:// x");
        // No host after the scheme: nothing to open.
        assert_eq!(term.url_at(0, 2), None);
    }

    #[test]
    fn combined_private_modes_update_cursor_and_sync_output() {
        let mut term = Terminal::new(3, 5);

        term.process_bytes(b"\x1b[?25;2026l");
        assert!(!term.grid().cursor_visible());
        assert!(!term.synchronized_output_active());

        term.process_bytes(b"\x1b[?25;2026h");
        assert!(term.grid().cursor_visible());
        assert!(term.synchronized_output_active());
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

#[cfg(test)]
mod tests_selection_clipboard_comprehensive {
    use super::*;
    use unshit::core::style::types::Color;

    // Helper: extract text from a range of cells in a single row.
    fn read_range(term: &Terminal, abs_line: u64, col_start: usize, col_end: usize) -> String {
        let mut s = String::new();
        for c in col_start..=col_end {
            if let Some(cell) = term.cell_at_abs(abs_line, c) {
                s.push(if cell.ch == '\0' { ' ' } else { cell.ch });
            }
        }
        s
    }

    // ---- selection_text tests ----

    #[test]
    fn selection_text_empty_grid_returns_empty() {
        let term = Terminal::new(0, 0);
        let result = term.selection_text((0, 0), (0, 0));
        assert_eq!(result, "");
    }

    #[test]
    fn selection_text_1x1_grid_single_cell() {
        let mut term = Terminal::new(1, 1);
        term.process_bytes(b"X");
        assert_eq!(term.selection_text((0, 0), (0, 0)), "X");
    }

    #[test]
    fn selection_text_clamped_start_below_first_abs_line() {
        // Evict some lines, then select a range that overlaps evicted area.
        let mut term = Terminal::new(2, 5);
        // Write 3 lines in a 2-row terminal to push one into scrollback.
        term.process_bytes(b"old0\r\nline1\r\nline2");
        // Evict the scrollback line by feeding MAX_SCROLLBACK more content.
        for i in 0..MAX_SCROLLBACK {
            let line = format!("x{}\r\n", i);
            term.process_bytes(line.as_bytes());
        }
        let first_abs = term.first_abs_line();
        // Try to select starting before first_abs_line (saturating so the
        // u64 does not underflow): should clamp and return the selection from
        // first_abs_line onward without padding lines for the evicted range.
        let result = term.selection_text((first_abs.saturating_sub(10), 0), (first_abs + 1, 3));
        assert!(!result.contains("\n\n"));
    }

    #[test]
    fn selection_text_clamped_end_beyond_buffer() {
        let mut term = Terminal::new(2, 8);
        term.process_bytes(b"line0\r\nline1");
        let end_abs = term.end_abs_line();
        // Select beyond the end: should clamp gracefully.
        let result = term.selection_text((0, 0), (end_abs + 100, 9));
        // Should not panic, result should not be excessively padded.
        assert!(result.len() < 100);
    }

    #[test]
    fn selection_text_zero_width_returns_single_cell() {
        let mut term = Terminal::new(2, 10);
        term.process_bytes(b"hello");
        // A zero-width (anchor==focus) range is one inclusive cell. Copy
        // callers gate on TermSelection::is_empty, so selection_text itself
        // returns that single cell — which is what makes a single-character
        // word (double-click) copy the character.
        assert_eq!(term.selection_text((0, 3), (0, 3)), "l");
    }

    #[test]
    fn selection_text_sub_range_within_row() {
        let mut term = Terminal::new(2, 10);
        term.process_bytes(b"0123456789");
        assert_eq!(term.selection_text((0, 2), (0, 5)), "2345");
    }

    #[test]
    fn selection_text_sub_range_inclusive_endpoints() {
        let mut term = Terminal::new(2, 10);
        term.process_bytes(b"abc");
        // [0, 0] should select just 'a', [0, 2] should select 'a', 'b', 'c'.
        assert_eq!(term.selection_text((0, 0), (0, 0)), "a");
        assert_eq!(term.selection_text((0, 0), (0, 2)), "abc");
    }

    #[test]
    fn selection_text_full_row_with_trailing_blanks_trims() {
        let mut term = Terminal::new(2, 10);
        term.process_bytes(b"text");
        // Selecting the whole row (including trailing blanks) should trim.
        assert_eq!(term.selection_text((0, 0), (0, 9)), "text");
    }

    #[test]
    fn selection_text_multirow_first_line_partial() {
        let mut term = Terminal::new(3, 10);
        term.process_bytes(b"line0xxxx\r\nline1xxxx\r\nline2");
        // Select from col 2 of line0 to col 3 of line1.
        let result = term.selection_text((0, 2), (1, 3));
        // First line: [2..eol] -> "ne0xxxx"; last line: [0..3] -> "line".
        assert_eq!(result, "ne0xxxx\nline");
    }

    #[test]
    fn selection_text_multirow_interior_lines_full() {
        let mut term = Terminal::new(5, 10);
        term.process_bytes(b"line0\r\nline1\r\nline2\r\nline3\r\nline4");
        // Select line0[0..eol], full line1, line2[0..4].
        let result = term.selection_text((0, 0), (2, 4));
        assert!(result.contains("line0"));
        assert!(result.contains("line1"));
        assert!(result.contains("line"));
    }

    #[test]
    fn selection_text_order_independence_same_line() {
        let mut term = Terminal::new(2, 10);
        term.process_bytes(b"abcdefghij");
        let ab = term.selection_text((0, 1), (0, 4));
        let ba = term.selection_text((0, 4), (0, 1));
        assert_eq!(ab, ba);
        assert_eq!(ab, "bcde");
    }

    #[test]
    fn selection_text_order_independence_multirow() {
        let mut term = Terminal::new(3, 10);
        term.process_bytes(b"line0\r\nline1\r\nline2");
        let forward = term.selection_text((0, 2), (1, 3));
        let backward = term.selection_text((1, 3), (0, 2));
        assert_eq!(forward, backward);
    }

    #[test]
    fn selection_text_wide_continuation_skipped() {
        // Wide CJK characters occupy 2 cells: primary + wide_continuation.
        // They should appear once in the selected text, not twice.
        let mut term = Terminal::new(2, 10);
        // Lay out a wide char (primary + continuation) followed by 'b'
        // directly in the grid (no trailing process_bytes that would
        // overwrite the manually placed cells at the cursor).
        let cell = |ch, cont| Cell {
            ch,
            fg: DEFAULT_FG,
            bg: DEFAULT_BG,
            attrs: CellAttrs::empty(),
            wide_continuation: cont,
        };
        term.grid.set_cell(0, 1, cell('世', false));
        term.grid.set_cell(0, 2, cell('\0', true));
        term.grid.set_cell(0, 3, cell('b', false));
        // The continuation cell is skipped entirely, so the wide char appears
        // once: "世b", NOT "世 b" (the '\0' is not turned into a space).
        let result = term.selection_text((0, 1), (0, 3));
        assert_eq!(result, "世b");
    }

    #[test]
    fn selection_text_null_char_becomes_space() {
        let mut term = Terminal::new(2, 5);
        term.process_bytes(b"text");
        // Null out col 0 AFTER writing the text (process_bytes would
        // otherwise overwrite a pre-placed cell at the cursor).
        let blank_cell = Cell {
            ch: '\0',
            fg: DEFAULT_FG,
            bg: DEFAULT_BG,
            attrs: CellAttrs::empty(),
            wide_continuation: false,
        };
        term.grid.set_cell(0, 0, blank_cell);
        // A non-continuation '\0' renders as a space: "text" -> " ext".
        let result = term.selection_text((0, 0), (0, 3));
        assert_eq!(result, " ext");
    }

    #[test]
    fn selection_text_across_scrollback_boundary() {
        let mut term = Terminal::new(2, 5);
        term.process_bytes(b"old\r\nnew");
        // old scrolled to scrollback[0], new on screen row 0.
        // abs line 0 is "old", abs line 1 is "new".
        let result = term.selection_text((0, 0), (1, 4));
        assert_eq!(result, "old\nnew");
    }

    #[test]
    fn selection_text_no_padding_blank_lines_evicted_top() {
        let mut term = Terminal::new(2, 3);
        term.process_bytes(b"A\r\nB\r\nC");
        // Evict to max.
        for _ in 0..MAX_SCROLLBACK {
            term.process_bytes(b"x\r\n");
        }
        let first = term.first_abs_line();
        // Select from before first_abs_line to well after: should clamp
        // and not pad the result with blank lines.
        let result = term.selection_text((first.saturating_sub(100), 0), (first + 10, 2));
        // Count leading/trailing newlines: should not have large runs.
        let mut leading_newlines = 0;
        for c in result.chars() {
            if c == '\n' {
                leading_newlines += 1;
            } else {
                break;
            }
        }
        assert_eq!(leading_newlines, 0, "should not pad with blank lines");
    }

    #[test]
    fn selection_text_scrollback_content_stable_after_scroll() {
        let mut term = Terminal::new(2, 5);
        term.process_bytes(b"line0\r\nline1\r\nline2");
        // line0 is in scrollback, line1/line2 on screen.
        let text_at_0 = term.selection_text((0, 0), (0, 4));
        // Scroll back to view line0.
        term.scroll_view_up(1);
        let text_after_scroll = term.selection_text((0, 0), (0, 4));
        // Text should be identical (by absolute line).
        assert_eq!(text_at_0, text_after_scroll);
    }

    #[test]
    fn selection_text_scrollback_content_stable_after_new_output() {
        let mut term = Terminal::new(2, 5);
        term.process_bytes(b"line0\r\nline1\r\nline2");
        let text_before = term.selection_text((0, 0), (0, 4));
        // More output pushes line0 deeper into scrollback.
        term.process_bytes(b"\r\nline3");
        let text_after = term.selection_text((0, 0), (0, 4));
        // Text is still the same.
        assert_eq!(text_before, text_after);
    }

    // ---- word_bounds_at tests ----

    #[test]
    fn word_bounds_empty_grid() {
        let term = Terminal::new(0, 0);
        assert_eq!(term.word_bounds_at(0, 0), (0, 0));
    }

    #[test]
    fn word_bounds_1x1_grid_alphanumeric() {
        let mut term = Terminal::new(1, 1);
        term.process_bytes(b"a");
        assert_eq!(term.word_bounds_at(0, 0), (0, 0));
    }

    #[test]
    fn word_bounds_word_in_middle() {
        let mut term = Terminal::new(2, 20);
        term.process_bytes(b"hello world test");
        // Click inside "hello" at col 2.
        assert_eq!(term.word_bounds_at(0, 2), (0, 4));
        // Click inside "world" at col 8.
        assert_eq!(term.word_bounds_at(0, 8), (6, 10));
    }

    #[test]
    fn word_bounds_at_col_0() {
        let mut term = Terminal::new(2, 10);
        term.process_bytes(b"hello");
        assert_eq!(term.word_bounds_at(0, 0), (0, 4));
    }

    #[test]
    fn word_bounds_at_last_col_in_word() {
        let mut term = Terminal::new(2, 10);
        term.process_bytes(b"hello");
        // Click at the 'o' (col 4).
        assert_eq!(term.word_bounds_at(0, 4), (0, 4));
    }

    #[test]
    fn word_bounds_on_whitespace_selects_cell_only() {
        let mut term = Terminal::new(2, 20);
        term.process_bytes(b"hello world");
        // Space at col 5.
        assert_eq!(term.word_bounds_at(0, 5), (5, 5));
    }

    #[test]
    fn word_bounds_path_like_token_slash() {
        let mut term = Terminal::new(2, 30);
        term.process_bytes(b"check /usr/bin/bash here");
        // Click inside /usr/bin/bash at col 8.
        let (start, end) = term.word_bounds_at(0, 8);
        assert!(start <= 8 && 8 <= end);
        let path = read_range(&term, 0, start, end);
        assert!(path.contains("/"));
    }

    #[test]
    fn word_bounds_path_token_with_dots() {
        let mut term = Terminal::new(2, 30);
        term.process_bytes(b"../foo.bar/baz.rs");
        // Click inside the token.
        let (start, end) = term.word_bounds_at(0, 5);
        let token = read_range(&term, 0, start, end);
        // Should span the whole path including dots and slashes.
        assert!(token.contains("."));
    }

    #[test]
    fn word_bounds_token_with_tilde() {
        let mut term = Terminal::new(2, 30);
        term.process_bytes(b"~/home/user");
        let (start, end) = term.word_bounds_at(0, 2);
        let token = read_range(&term, 0, start, end);
        assert!(token.contains("~"));
    }

    #[test]
    fn word_bounds_token_with_hyphen() {
        let mut term = Terminal::new(2, 30);
        term.process_bytes(b"my-lib-name");
        let (start, end) = term.word_bounds_at(0, 5);
        let token = read_range(&term, 0, start, end);
        assert_eq!(token, "my-lib-name");
    }

    #[test]
    fn word_bounds_token_with_colon() {
        let mut term = Terminal::new(2, 30);
        term.process_bytes(b"user@host:22");
        let (start, end) = term.word_bounds_at(0, 5);
        let token = read_range(&term, 0, start, end);
        assert!(token.contains(":"));
    }

    #[test]
    fn word_bounds_single_char_word() {
        let mut term = Terminal::new(2, 10);
        term.process_bytes(b"a b c");
        assert_eq!(term.word_bounds_at(0, 0), (0, 0));
        assert_eq!(term.word_bounds_at(0, 2), (2, 2));
    }

    #[test]
    fn word_bounds_nonword_char_selects_only_that_cell() {
        let mut term = Terminal::new(2, 20);
        term.process_bytes(b"hello(world)");
        // Parenthesis at col 5.
        assert_eq!(term.word_bounds_at(0, 5), (5, 5));
        // Parenthesis at col 11.
        assert_eq!(term.word_bounds_at(0, 11), (11, 11));
    }

    #[test]
    fn word_bounds_punctuation_boundary() {
        let mut term = Terminal::new(2, 20);
        term.process_bytes(b"hello,world");
        // 'o' in hello -> word ends before the comma.
        let (_s1, e1) = term.word_bounds_at(0, 4);
        // 'w' in world -> word starts after the comma.
        let (s2, _e2) = term.word_bounds_at(0, 6);
        // The comma is not a word char, so the two words do not overlap.
        assert!(e1 < s2);
    }

    #[test]
    fn word_bounds_col_clamped_to_grid() {
        let mut term = Terminal::new(2, 5);
        term.process_bytes(b"hello");
        // Request col 10 (beyond grid): should clamp to col 4 (last col).
        let (start, end) = term.word_bounds_at(0, 10);
        // Should select the 'o' at col 4.
        assert!(start <= 4 && 4 <= end);
    }

    #[test]
    fn word_bounds_empty_line() {
        let mut term = Terminal::new(2, 10);
        // Line 1 is blank.
        term.process_bytes(b"hello\r\n");
        // Click on the blank line.
        assert_eq!(term.word_bounds_at(1, 0), (0, 0));
    }

    // ---- line_bounds_at tests ----

    #[test]
    fn line_bounds_full_row() {
        let term = Terminal::new(2, 12);
        assert_eq!(term.line_bounds_at(0), (0, 11));
    }

    #[test]
    fn line_bounds_1col_grid() {
        let term = Terminal::new(2, 1);
        assert_eq!(term.line_bounds_at(0), (0, 0));
    }

    #[test]
    fn line_bounds_large_grid() {
        let term = Terminal::new(10, 200);
        assert_eq!(term.line_bounds_at(5), (0, 199));
    }

    #[test]
    fn line_bounds_unused_abs_line_param() {
        // line_bounds_at ignores the abs_line parameter (always returns full row).
        let term = Terminal::new(2, 10);
        assert_eq!(term.line_bounds_at(0), (0, 9));
        assert_eq!(term.line_bounds_at(999), (0, 9));
    }

    // ---- paint_selection tests ----

    #[test]
    fn paint_selection_empty_grid_is_noop() {
        let term = Terminal::new(0, 0);
        let mut grid = term.display_grid();
        // Should not panic.
        term.paint_selection(&mut grid, (0, 0), (0, 5), Color::rgb(255, 0, 0));
    }

    #[test]
    fn paint_selection_collapsed_anchor_focus_is_noop() {
        let mut term = Terminal::new(2, 10);
        term.process_bytes(b"hello");
        let mut grid = term.display_grid();
        let bg_before = grid.get_cell(0, 0).unwrap().bg;
        term.paint_selection(&mut grid, (0, 2), (0, 2), Color::rgb(100, 100, 100));
        let bg_after = grid.get_cell(0, 0).unwrap().bg;
        assert_eq!(bg_before, bg_after);
    }

    #[test]
    fn paint_selection_single_cell() {
        let mut term = Terminal::new(2, 10);
        term.process_bytes(b"hello");
        let mut grid = term.display_grid();
        let bg = Color::rgb(200, 100, 50);
        term.paint_selection(&mut grid, (0, 0), (0, 0), bg);
        // Line 0 renders at grid row 1 (row 0 is the overscan row).
        assert_eq!(grid.get_cell(1, 0).unwrap().bg, bg);
    }

    #[test]
    fn paint_selection_single_row_range() {
        let mut term = Terminal::new(2, 10);
        term.process_bytes(b"hello");
        let mut grid = term.display_grid();
        let bg = Color::rgb(100, 150, 200);
        term.paint_selection(&mut grid, (0, 1), (0, 3), bg);
        // Cells 1, 2, 3 of line 0 (grid row 1) should have the color.
        assert_eq!(grid.get_cell(1, 1).unwrap().bg, bg);
        assert_eq!(grid.get_cell(1, 2).unwrap().bg, bg);
        assert_eq!(grid.get_cell(1, 3).unwrap().bg, bg);
        // Cell 0 should not.
        assert_ne!(grid.get_cell(1, 0).unwrap().bg, bg);
    }

    #[test]
    fn paint_selection_multirow_first_line_partial() {
        let mut term = Terminal::new(3, 10);
        term.process_bytes(b"line0\r\nline1\r\nline2");
        let mut grid = term.display_grid();
        let bg = Color::rgb(150, 150, 150);
        // Select from (0, 2) to (1, 3). Lines render at grid row + 1
        // because of the overscan row.
        term.paint_selection(&mut grid, (0, 2), (1, 3), bg);
        // Line 0 (grid row 1): cells 2..9 should be painted.
        for c in 2..10 {
            assert_eq!(grid.get_cell(1, c).unwrap().bg, bg, "line 0 col {}", c);
        }
        // Line 1 (grid row 2): cells 0..3 should be painted.
        for c in 0..=3 {
            assert_eq!(grid.get_cell(2, c).unwrap().bg, bg, "line 1 col {}", c);
        }
        // Line 2 (grid row 3) should not be painted.
        assert_ne!(grid.get_cell(3, 0).unwrap().bg, bg);
    }

    #[test]
    fn paint_selection_multirow_all_interior_lines() {
        let mut term = Terminal::new(5, 10);
        term.process_bytes(b"L0\r\nL1\r\nL2\r\nL3\r\nL4");
        let mut grid = term.display_grid();
        let bg = Color::rgb(50, 100, 150);
        // Select entire lines 1 and 2 (grid rows 2 and 3).
        term.paint_selection(&mut grid, (1, 0), (2, 9), bg);
        for r in 2..=3 {
            for c in 0..10 {
                assert_eq!(grid.get_cell(r, c).unwrap().bg, bg, "({}, {})", r, c);
            }
        }
    }

    #[test]
    fn paint_selection_order_independent() {
        let mut term = Terminal::new(2, 10);
        term.process_bytes(b"hello");
        let bg = Color::rgb(75, 75, 75);
        let mut grid1 = term.display_grid();
        term.paint_selection(&mut grid1, (0, 1), (0, 3), bg);
        let mut grid2 = term.display_grid();
        term.paint_selection(&mut grid2, (0, 3), (0, 1), bg);
        // Both should have the same cells painted.
        for c in 0..10 {
            assert_eq!(
                grid1.get_cell(0, c).unwrap().bg,
                grid2.get_cell(0, c).unwrap().bg,
                "col {}",
                c
            );
        }
    }

    #[test]
    fn paint_selection_off_screen_above_is_noop() {
        let mut term = Terminal::new(2, 10);
        term.process_bytes(b"L0\r\nL1\r\nL2\r\nL3");
        // L0/L1 scrolled into scrollback; live view shows L2/L3.
        let mut grid = term.display_grid();
        let bg = Color::rgb(80, 80, 80);
        // Select L0 (abs line 0), which is not visible.
        term.paint_selection(&mut grid, (0, 0), (0, 9), bg);
        // Nothing should be painted.
        for r in 0..2 {
            for c in 0..10 {
                assert_ne!(grid.get_cell(r, c).unwrap().bg, bg, "({}, {})", r, c);
            }
        }
    }

    #[test]
    fn paint_selection_off_screen_below_is_noop() {
        let mut term = Terminal::new(2, 10);
        term.process_bytes(b"L0\r\nL1");
        // Only 2 lines in the buffer.
        let mut grid = term.display_grid();
        let bg = Color::rgb(80, 80, 80);
        // Select lines 5..6 (well beyond).
        term.paint_selection(&mut grid, (5, 0), (6, 9), bg);
        // Nothing should be painted.
        for r in 0..2 {
            for c in 0..10 {
                assert_ne!(grid.get_cell(r, c).unwrap().bg, bg);
            }
        }
    }

    #[test]
    fn paint_selection_partially_scrolled_paints_visible_slice() {
        let mut term = Terminal::new(2, 10);
        term.process_bytes(b"line0\r\nline1\r\nline2\r\nline3");
        // line0/line1 in scrollback, line2/line3 on screen.
        let mut grid = term.display_grid();
        let bg = Color::rgb(120, 120, 120);
        // Select line0 (abs line 0) to line2 (abs line 2). line1 sits on
        // the partially visible overscan row (grid row 0) and paints in
        // full as an interior line; line2 (grid row 1) paints its
        // partial range; line0 stays off screen.
        term.paint_selection(&mut grid, (0, 0), (2, 5), bg);
        for c in 0..10 {
            assert_eq!(
                grid.get_cell(0, c).unwrap().bg,
                bg,
                "overscan line1 col {}",
                c
            );
        }
        for c in 0..=5 {
            assert_eq!(grid.get_cell(1, c).unwrap().bg, bg, "line2 col {}", c);
        }
        // line3 (grid row 2) should not be painted (selection ends at line2).
        assert_ne!(grid.get_cell(2, 0).unwrap().bg, bg);
    }

    #[test]
    fn paint_selection_clears_inverse_attr() {
        let mut term = Terminal::new(2, 10);
        term.process_bytes(b"hello");
        // Manually set INVERSE on a cell.
        let mut cell = term.grid.get_cell(0, 1).copied().unwrap();
        cell.attrs.insert(CellAttrs::INVERSE);
        term.grid.set_cell(0, 1, cell);
        let mut grid = term.display_grid();
        // Line 0 renders at grid row 1 (row 0 is the overscan row).
        assert!(grid
            .get_cell(1, 1)
            .unwrap()
            .attrs
            .contains(CellAttrs::INVERSE));
        let bg = Color::rgb(200, 200, 200);
        term.paint_selection(&mut grid, (0, 1), (0, 1), bg);
        // INVERSE should be cleared on the painted cell.
        assert!(!grid
            .get_cell(1, 1)
            .unwrap()
            .attrs
            .contains(CellAttrs::INVERSE));
    }

    #[test]
    fn paint_selection_other_attrs_preserved() {
        let mut term = Terminal::new(2, 10);
        term.process_bytes(b"hello");
        // Manually set BOLD on a cell.
        let mut cell = term.grid.get_cell(0, 2).copied().unwrap();
        cell.attrs.insert(CellAttrs::BOLD);
        term.grid.set_cell(0, 2, cell);
        let mut grid = term.display_grid();
        // Line 0 renders at grid row 1 (row 0 is the overscan row).
        assert!(grid.get_cell(1, 2).unwrap().attrs.contains(CellAttrs::BOLD));
        let bg = Color::rgb(200, 200, 200);
        term.paint_selection(&mut grid, (0, 2), (0, 2), bg);
        // BOLD should still be there.
        assert!(grid.get_cell(1, 2).unwrap().attrs.contains(CellAttrs::BOLD));
    }

    #[test]
    fn paint_selection_non_selected_cells_untouched() {
        let mut term = Terminal::new(2, 10);
        term.process_bytes(b"hello");
        let mut grid = term.display_grid();
        let original_bg = grid.get_cell(0, 0).unwrap().bg;
        let paint_bg = Color::rgb(100, 100, 100);
        term.paint_selection(&mut grid, (0, 2), (0, 4), paint_bg);
        // Unselected cells should keep their original bg.
        assert_eq!(grid.get_cell(0, 0).unwrap().bg, original_bg);
        assert_eq!(grid.get_cell(0, 1).unwrap().bg, original_bg);
        assert_eq!(grid.get_cell(0, 5).unwrap().bg, original_bg);
    }

    // ---- bracketed paste mode tests ----

    #[test]
    fn bracketed_paste_default_off() {
        let term = Terminal::new(3, 5);
        assert!(!term.bracketed_paste_active());
    }

    #[test]
    fn bracketed_paste_csi_2004h_enables() {
        let mut term = Terminal::new(3, 5);
        term.process_bytes(b"\x1b[?2004h");
        assert!(term.bracketed_paste_active());
    }

    #[test]
    fn bracketed_paste_csi_2004l_disables() {
        let mut term = Terminal::new(3, 5);
        term.process_bytes(b"\x1b[?2004h");
        assert!(term.bracketed_paste_active());
        term.process_bytes(b"\x1b[?2004l");
        assert!(!term.bracketed_paste_active());
    }

    #[test]
    fn bracketed_paste_toggle_multiple_times() {
        let mut term = Terminal::new(3, 5);
        for _ in 0..3 {
            assert!(!term.bracketed_paste_active());
            term.process_bytes(b"\x1b[?2004h");
            assert!(term.bracketed_paste_active());
            term.process_bytes(b"\x1b[?2004l");
        }
        assert!(!term.bracketed_paste_active());
    }

    #[test]
    fn bracketed_paste_interleaved_with_cursor_hide() {
        let mut term = Terminal::new(3, 5);
        // ?25h SHOWS the cursor (h = set), ?2004h enables bracketed paste.
        term.process_bytes(b"\x1b[?25;2004h");
        assert!(term.bracketed_paste_active());
        assert!(term.grid().cursor_visible());
        // ?25l HIDES the cursor (l = reset), ?2004l disables bracketed paste.
        term.process_bytes(b"\x1b[?25;2004l");
        assert!(!term.bracketed_paste_active());
        assert!(!term.grid().cursor_visible());
    }

    #[test]
    fn bracketed_paste_independent_from_sync_output() {
        let mut term = Terminal::new(3, 5);
        term.process_bytes(b"\x1b[?2004h");
        assert!(term.bracketed_paste_active());
        assert!(!term.synchronized_output_active());
        term.process_bytes(b"\x1b[?2026h");
        assert!(term.bracketed_paste_active());
        assert!(term.synchronized_output_active());
        term.process_bytes(b"\x1b[?2004l");
        assert!(!term.bracketed_paste_active());
        assert!(term.synchronized_output_active());
    }

    // ---- absolute line mapping tests ----

    #[test]
    fn first_abs_line_at_start() {
        let term = Terminal::new(3, 5);
        assert_eq!(term.first_abs_line(), 0);
    }

    #[test]
    fn first_abs_line_after_scrollback() {
        let mut term = Terminal::new(2, 5);
        term.process_bytes(b"L0\r\nL1\r\nL2");
        // L0 scrolled to scrollback.
        assert_eq!(term.first_abs_line(), 0);
    }

    #[test]
    fn abs_line_at_display_row_0_bottom() {
        let mut term = Terminal::new(2, 5);
        term.process_bytes(b"L0\r\nL1");
        // At bottom, display row 0 shows L0 (abs line 0).
        assert_eq!(term.abs_line_at_display(0), 0);
    }

    #[test]
    fn abs_line_at_display_stable_after_scroll_view_up() {
        let mut term = Terminal::new(2, 5);
        term.process_bytes(b"L0\r\nL1\r\nL2\r\nL3");
        // L0/L1 scrolled, display rows show L2/L3 (abs 2, 3).
        assert_eq!(term.abs_line_at_display(0), 2);
        assert_eq!(term.abs_line_at_display(1), 3);
        term.scroll_view_up(1);
        // Now display row 0 shows L1 (abs 1), row 1 shows L2 (abs 2).
        assert_eq!(term.abs_line_at_display(0), 1);
        assert_eq!(term.abs_line_at_display(1), 2);
    }

    #[test]
    fn abs_line_at_display_stable_after_scroll_view_down() {
        let mut term = Terminal::new(2, 5);
        term.process_bytes(b"L0\r\nL1\r\nL2\r\nL3");
        term.scroll_view_up(2);
        assert_eq!(term.abs_line_at_display(0), 0);
        term.scroll_view_down(1);
        assert_eq!(term.abs_line_at_display(0), 1);
    }

    #[test]
    fn abs_line_at_display_stable_after_reset_scroll() {
        let mut term = Terminal::new(2, 5);
        term.process_bytes(b"L0\r\nL1\r\nL2\r\nL3");
        term.scroll_view_up(2);
        term.reset_scroll();
        // Back at bottom.
        assert_eq!(term.abs_line_at_display(0), 2);
    }

    #[test]
    fn selection_text_stable_across_scroll_view_up_down() {
        let mut term = Terminal::new(2, 5);
        term.process_bytes(b"old\r\nnew");
        let text_initial = term.selection_text((0, 0), (0, 2));
        term.scroll_view_up(1);
        let text_scrolled_up = term.selection_text((0, 0), (0, 2));
        term.scroll_view_down(1);
        let text_back = term.selection_text((0, 0), (0, 2));
        assert_eq!(text_initial, text_scrolled_up);
        assert_eq!(text_initial, text_back);
    }

    #[test]
    fn selection_text_survives_new_output_pushing_into_scrollback() {
        let mut term = Terminal::new(2, 5);
        term.process_bytes(b"L0\r\nL1\r\nL2");
        let text_at_0 = term.selection_text((0, 0), (0, 1));
        // More output pushes L0 deeper.
        term.process_bytes(b"\r\nL3\r\nL4");
        let text_still_0 = term.selection_text((0, 0), (0, 1));
        assert_eq!(text_at_0, text_still_0);
    }

    #[test]
    fn paint_selection_survives_scroll_view_changes() {
        let mut term = Terminal::new(2, 5);
        term.process_bytes(b"L0\r\nL1\r\nL2\r\nL3");
        // Select L0 (abs line 0) — off-screen (the overscan row shows L1).
        let bg = Color::rgb(99, 99, 99);
        let mut grid = term.display_grid();
        term.paint_selection(&mut grid, (0, 0), (0, 2), bg);
        // Should not paint at live view (L0 not visible).
        assert!((0..3).all(|r| grid.get_cell(r, 0).unwrap().bg != bg));
        // Scroll back to view L0.
        term.scroll_view_up(2);
        let mut grid2 = term.display_grid();
        term.paint_selection(&mut grid2, (0, 0), (0, 2), bg);
        // Now it should paint at viewport row 0 (grid row 1).
        assert_eq!(grid2.get_cell(1, 0).unwrap().bg, bg);
    }
}
