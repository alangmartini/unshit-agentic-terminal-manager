//! Paints an `EditorBuffer` viewport into a `CellGrid`.
//!
//! The editor reuses the terminal's grid primitive so it inherits the
//! renderer's damage tracking and the `LineQuadCache`'s stable-line-id
//! replay. Only the visible window is ever painted; scrolls move rows
//! with `shift_rows` so unchanged lines keep their identity (and their
//! cached quads) across the move. Everything a row shows — gutter,
//! selection, syntax colour, search hits, diff tints — is cell `fg`/`bg`:
//! the renderer has no overlay or per-row decoration seam, so colour is
//! the only lever there is.
//!
//! Row *content* comes from the buffer; everything else comes from
//! [`RowPaint`], which the pane fills in. Keeping the decisions in the
//! pane (which line is the cursor's, which byte ranges matched a search,
//! what a diff row means) leaves this module a pure painter.

use unshit::core::cell_grid::{Cell, CellAttrs, CellGrid};
use unshit::core::style::types::Color;

use crate::syntax::Span;

use super::buffer::{EditorBuffer, Position};
use super::colors::EditorColors;
use super::find::Match;

/// Gutter width in cells: right-aligned line number plus one space,
/// with a 3-digit floor so short files don't jitter the text column.
pub fn gutter_width(line_count: usize) -> usize {
    let digits = line_count.max(1).ilog10() as usize + 1;
    digits.max(3) + 1
}

/// Gutter width for a diff document: two right-aligned line-number
/// columns (old and new), then the `+`/`-` marker, each separated by one
/// space — `"  1   1 + "`. Three-digit floor per column, matching
/// [`gutter_width`], so short diffs do not sit flush against the text.
///
/// Must stay in lockstep with `DiffView::gutter_text`, which writes
/// exactly this many cells.
pub fn diff_gutter_width(max_old: u32, max_new: u32) -> usize {
    let digits = |n: u32| (n.max(1).ilog10() as usize + 1).max(3);
    digits(max_old) + 1 + digits(max_new) + 1 + 1 + 1
}

/// Is byte offset `col` of `line_idx` inside the ordered selection?
fn in_selection(sel: Option<(Position, Position)>, line_idx: usize, col: usize) -> bool {
    let Some((start, end)) = sel else {
        return false;
    };
    let pos = Position {
        line: line_idx,
        col,
    };
    pos >= start && pos < end
}

/// Everything a row needs beyond the grid and the buffer.
///
/// Built per row by the pane. The lifetimes all borrow pane-owned
/// scratch, so painting a viewport allocates nothing.
pub struct RowPaint<'a> {
    /// Visual columns (tabs expanded) skipped at the left of the line.
    pub h_offset: usize,
    pub gutter_w: usize,
    pub selection: Option<(Position, Position)>,
    pub colors: &'a EditorColors,
    /// Token spans tiling the line, or empty for no highlighting.
    pub spans: &'a [Span],
    /// Background for the whole row, under everything but the selection
    /// (diff add/remove tints, the cursor line).
    pub row_bg: Option<Color>,
    /// Replaces the line number, for diff panes. An empty string paints
    /// a blank gutter (header rows).
    pub gutter_text: Option<&'a str>,
    /// Colour of the gutter text.
    pub gutter_fg: Color,
    /// Forces one colour for the whole line, overriding syntax (diff
    /// file and hunk headers, the "loading" placeholder).
    pub fg_override: Option<Color>,
    /// Bold the whole line (diff file headers).
    pub bold: bool,
    /// Search hits on this line, as byte ranges.
    pub finds: &'a [Match],
    /// The one hit the find bar considers current, if it is on this line.
    pub current_find: Option<Match>,
}

impl<'a> RowPaint<'a> {
    /// A plain file row: line-numbered gutter, no decorations.
    pub fn plain(
        h_offset: usize,
        gutter_w: usize,
        selection: Option<(Position, Position)>,
        colors: &'a EditorColors,
    ) -> Self {
        Self {
            h_offset,
            gutter_w,
            selection,
            colors,
            spans: &[],
            row_bg: None,
            gutter_text: None,
            gutter_fg: colors.gutter,
            fg_override: None,
            bold: false,
            finds: &[],
            current_find: None,
        }
    }

    /// Foreground for byte offset `byte` of the line.
    fn fg_at(&self, byte: usize) -> Color {
        if let Some(fg) = self.fg_override {
            return fg;
        }
        // Spans tile the line in order, so a linear scan is exact; lines
        // are short enough that a binary search would not pay for itself.
        for span in self.spans {
            if byte < span.end {
                return if byte >= span.start {
                    self.colors.token(span.kind)
                } else {
                    self.colors.text
                };
            }
        }
        self.colors.text
    }

    /// Background for byte offset `byte`, in priority order: selection
    /// beats the current search hit, which beats other hits, which beat
    /// the row tint.
    fn bg_at(&self, line_idx: usize, byte: usize) -> Color {
        if in_selection(self.selection, line_idx, byte) {
            return self.colors.selection_bg;
        }
        if let Some(current) = self.current_find {
            if byte >= current.start && byte < current.end {
                return self.colors.find_current_bg;
            }
        }
        if self.finds.iter().any(|m| byte >= m.start && byte < m.end) {
            return self.colors.find_bg;
        }
        self.row_bg.unwrap_or(Color::TRANSPARENT)
    }

    fn attrs(&self) -> CellAttrs {
        if self.bold {
            CellAttrs::BOLD
        } else {
            CellAttrs::empty()
        }
    }
}

/// Paint one grid row from `line_idx` of the buffer. Rows past the end
/// of the buffer are blank (no gutter number, mirroring code editors).
pub fn paint_row(
    grid: &mut CellGrid,
    row: usize,
    line_idx: usize,
    buffer: &EditorBuffer,
    paint: &RowPaint<'_>,
) {
    let cols = grid.cols();
    let Some(line) = buffer.line(line_idx) else {
        for col in 0..cols {
            grid.set_cell(row, col, Cell::default());
        }
        return;
    };

    paint_gutter(grid, row, line_idx, paint, cols);

    // Content cells from visual column `h_offset` onward. Tabs expand
    // to their next stop and paint as spaces (selection background
    // covers the whole span); byte offsets ride along so selection
    // membership is exact even mid-tab.
    let attrs = paint.attrs();
    let mut col = paint.gutter_w;
    let mut vcol = 0usize;
    'content: for (byte, ch) in line.char_indices() {
        let width = super::buffer::char_width_at(ch, vcol);
        for k in 0..width {
            if vcol + k < paint.h_offset {
                continue;
            }
            if col >= cols {
                break 'content;
            }
            let display = if ch == '\t' { ' ' } else { ch };
            grid.set_cell(
                row,
                col,
                Cell {
                    ch: display,
                    fg: paint.fg_at(byte),
                    bg: paint.bg_at(line_idx, byte),
                    attrs,
                    ..Default::default()
                },
            );
            col += 1;
        }
        vcol += width;
    }
    // A selection that continues past the end of this line paints one
    // trailing marker cell (the newline), like every code editor.
    if col < cols && in_selection(paint.selection, line_idx, line.len()) {
        grid.set_cell(
            row,
            col,
            Cell {
                ch: ' ',
                fg: paint.colors.text,
                bg: paint.colors.selection_bg,
                ..Default::default()
            },
        );
        col += 1;
    }
    // A row tint (diff add/remove, cursor line) covers the full width,
    // not just the text: a half-tinted row reads as a rendering bug. The
    // renderer merges same-background cells into one quad, so this costs
    // nothing extra.
    let trailing = match paint.row_bg {
        Some(bg) => Cell {
            bg,
            ..Default::default()
        },
        None => Cell::default(),
    };
    for c in col..cols {
        grid.set_cell(row, c, trailing);
    }
}

/// Paint the gutter: either the buffer line number or the text the pane
/// supplied (diff panes show old and new numbers plus a marker).
fn paint_gutter(
    grid: &mut CellGrid,
    row: usize,
    line_idx: usize,
    paint: &RowPaint<'_>,
    cols: usize,
) {
    let owned;
    let text: &str = match paint.gutter_text {
        Some(text) => text,
        None => {
            // Right-aligned line number, one trailing space of separation.
            let number = (line_idx + 1).to_string();
            let digit_cells = paint.gutter_w.saturating_sub(1);
            let pad = digit_cells.saturating_sub(number.len());
            owned = format!("{:pad$}{} ", "", number, pad = pad);
            &owned
        }
    };
    let mut chars = text.chars();
    for col in 0..paint.gutter_w.min(cols) {
        let ch = chars.next().unwrap_or(' ');
        grid.set_cell(
            row,
            col,
            Cell {
                ch,
                fg: paint.gutter_fg,
                bg: paint.row_bg.unwrap_or(Color::TRANSPARENT),
                ..Default::default()
            },
        );
    }
}

/// What [`plan_scroll`] left for the caller to repaint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScrollPlan {
    /// The move was too large to shift: repaint every row.
    Full,
    /// Rows `start..end` were newly exposed and need painting; the rest
    /// kept their content and their line identity.
    Rows(std::ops::Range<usize>),
}

/// Shift the painted viewport from `old_top` to `new_top`, preserving the
/// line identity (and cached quads) of the rows that survive the move,
/// and report which rows the caller must repaint.
///
/// Control is inverted — this does not paint — because what a row shows
/// depends on pane state (syntax cache, search hits, diff decorations)
/// that this module deliberately knows nothing about.
pub fn plan_scroll(grid: &mut CellGrid, old_top: usize, new_top: usize) -> ScrollPlan {
    if new_top == old_top {
        return ScrollPlan::Rows(0..0);
    }
    let rows = grid.rows();
    let delta = new_top.abs_diff(old_top);
    if delta >= rows {
        return ScrollPlan::Full;
    }
    let keep = rows - delta;
    if new_top > old_top {
        // Content moves up: rows delta..rows shift to 0..keep.
        grid.shift_rows(0, delta, keep);
        for row in keep..rows {
            grid.reset_line_identity(row);
        }
        ScrollPlan::Rows(keep..rows)
    } else {
        // Content moves down: rows 0..keep shift to delta..rows.
        grid.shift_rows(delta, 0, keep);
        for row in 0..delta {
            grid.reset_line_identity(row);
        }
        ScrollPlan::Rows(0..delta)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::TokenKind;

    fn grid_row_text(grid: &CellGrid, row: usize) -> String {
        (0..grid.cols())
            .map(|c| {
                // `Cell::default()` holds NUL, which is how a blank cell
                // is stored; render it as the space it paints as.
                match grid.get_cell(row, c).map(|cell| cell.ch) {
                    Some('\0') | None => ' ',
                    Some(ch) => ch,
                }
            })
            .collect()
    }

    fn paint(buffer: &EditorBuffer, paint: &RowPaint<'_>, line: usize) -> CellGrid {
        let mut grid = CellGrid::new(3, 40);
        paint_row(&mut grid, 0, line, buffer, paint);
        grid
    }

    #[test]
    fn gutter_width_has_a_three_digit_floor_and_grows_with_the_file() {
        assert_eq!(gutter_width(1), 4);
        assert_eq!(gutter_width(999), 4);
        assert_eq!(gutter_width(1000), 5);
    }

    #[test]
    fn paints_the_line_number_and_the_text() {
        let buffer = EditorBuffer::from_text("alpha\nbeta");
        let colors = EditorColors::default();
        let p = RowPaint::plain(0, gutter_width(2), None, &colors);
        let grid = paint(&buffer, &p, 1);
        assert!(grid_row_text(&grid, 0).starts_with("  2 beta"));
    }

    #[test]
    fn rows_past_the_end_of_the_buffer_are_blank() {
        let buffer = EditorBuffer::from_text("only");
        let colors = EditorColors::default();
        let p = RowPaint::plain(0, gutter_width(1), None, &colors);
        let grid = paint(&buffer, &p, 5);
        assert_eq!(grid_row_text(&grid, 0).trim(), "");
    }

    #[test]
    fn selection_paints_the_selected_cells_only() {
        let buffer = EditorBuffer::from_text("abcdef");
        let colors = EditorColors::default();
        let sel = Some((Position { line: 0, col: 1 }, Position { line: 0, col: 3 }));
        let p = RowPaint::plain(0, gutter_width(1), sel, &colors);
        let grid = paint(&buffer, &p, 0);
        let g = gutter_width(1);
        assert_eq!(grid.get_cell(0, g).unwrap().bg, Color::TRANSPARENT, "'a'");
        assert_eq!(
            grid.get_cell(0, g + 1).unwrap().bg,
            colors.selection_bg,
            "'b'"
        );
        assert_eq!(
            grid.get_cell(0, g + 2).unwrap().bg,
            colors.selection_bg,
            "'c'"
        );
        assert_eq!(
            grid.get_cell(0, g + 3).unwrap().bg,
            Color::TRANSPARENT,
            "'d'"
        );
    }

    /// Syntax spans must reach the cells: this is the whole point of
    /// threading them through the painter.
    #[test]
    fn token_spans_colour_the_cells_they_cover() {
        let buffer = EditorBuffer::from_text("let x = 1;");
        let colors = EditorColors::default();
        let spans = vec![
            Span {
                start: 0,
                end: 3,
                kind: TokenKind::Keyword,
            },
            Span {
                start: 3,
                end: 8,
                kind: TokenKind::Ident,
            },
            Span {
                start: 8,
                end: 9,
                kind: TokenKind::Number,
            },
            Span {
                start: 9,
                end: 10,
                kind: TokenKind::Punct,
            },
        ];
        let mut p = RowPaint::plain(0, gutter_width(1), None, &colors);
        p.spans = &spans;
        let grid = paint(&buffer, &p, 0);
        let g = gutter_width(1);
        assert_eq!(
            grid.get_cell(0, g).unwrap().fg,
            colors.keyword,
            "'l' of let"
        );
        assert_eq!(grid.get_cell(0, g + 8).unwrap().fg, colors.number, "the 1");
        assert_eq!(grid.get_cell(0, g + 9).unwrap().fg, colors.punct, "the ;");
    }

    /// Priority matters: a search hit inside a selection must not make
    /// the selection look broken.
    #[test]
    fn background_priority_is_selection_then_current_hit_then_hit_then_tint() {
        let buffer = EditorBuffer::from_text("aaaa bbbb cccc");
        let colors = EditorColors::default();
        let finds = vec![
            Match {
                line: 0,
                start: 0,
                end: 4,
            },
            Match {
                line: 0,
                start: 5,
                end: 9,
            },
            Match {
                line: 0,
                start: 10,
                end: 14,
            },
        ];
        let mut p = RowPaint::plain(
            0,
            gutter_width(1),
            Some((Position { line: 0, col: 0 }, Position { line: 0, col: 2 })),
            &colors,
        );
        p.finds = &finds;
        p.current_find = Some(finds[1]);
        p.row_bg = Some(colors.diff_added_bg);
        let grid = paint(&buffer, &p, 0);
        let g = gutter_width(1);
        assert_eq!(
            grid.get_cell(0, g).unwrap().bg,
            colors.selection_bg,
            "selected"
        );
        assert_eq!(
            grid.get_cell(0, g + 2).unwrap().bg,
            colors.find_bg,
            "other hit"
        );
        assert_eq!(
            grid.get_cell(0, g + 5).unwrap().bg,
            colors.find_current_bg,
            "current hit"
        );
        assert_eq!(
            grid.get_cell(0, g + 4).unwrap().bg,
            colors.diff_added_bg,
            "gap between hits keeps the row tint"
        );
    }

    /// A diff row's tint has to reach the end of the pane, or rows look
    /// ragged where the code happens to be short.
    #[test]
    fn a_row_tint_covers_the_full_width() {
        let buffer = EditorBuffer::from_text("short");
        let colors = EditorColors::default();
        let mut p = RowPaint::plain(0, gutter_width(1), None, &colors);
        p.row_bg = Some(colors.diff_removed_bg);
        let grid = paint(&buffer, &p, 0);
        let last = grid.cols() - 1;
        assert_eq!(grid.get_cell(0, last).unwrap().bg, colors.diff_removed_bg);
        assert_eq!(
            grid.get_cell(0, 0).unwrap().bg,
            colors.diff_removed_bg,
            "gutter"
        );
    }

    #[test]
    fn supplied_gutter_text_replaces_the_line_number() {
        let buffer = EditorBuffer::from_text("code");
        let colors = EditorColors::default();
        let mut p = RowPaint::plain(0, 10, None, &colors);
        p.gutter_text = Some(" 12  13 + ");
        let grid = paint(&buffer, &p, 0);
        assert!(
            grid_row_text(&grid, 0).starts_with(" 12  13 + code"),
            "got {:?}",
            grid_row_text(&grid, 0)
        );
    }

    #[test]
    fn fg_override_wins_over_spans() {
        let buffer = EditorBuffer::from_text("@@ -1 +1 @@");
        let colors = EditorColors::default();
        let spans = vec![Span {
            start: 0,
            end: 11,
            kind: TokenKind::Keyword,
        }];
        let mut p = RowPaint::plain(0, gutter_width(1), None, &colors);
        p.spans = &spans;
        p.fg_override = Some(colors.diff_hunk_fg);
        let grid = paint(&buffer, &p, 0);
        assert_eq!(
            grid.get_cell(0, gutter_width(1)).unwrap().fg,
            colors.diff_hunk_fg
        );
    }

    #[test]
    fn horizontal_offset_skips_leading_columns() {
        let buffer = EditorBuffer::from_text("0123456789");
        let colors = EditorColors::default();
        let p = RowPaint::plain(4, gutter_width(1), None, &colors);
        let grid = paint(&buffer, &p, 0);
        let g = gutter_width(1);
        assert_eq!(grid.get_cell(0, g).unwrap().ch, '4');
    }

    /// Scrolling by less than a viewport must reuse the rows that stay:
    /// that reuse is what keeps the renderer's line cache warm.
    #[test]
    fn plan_scroll_shifts_rows_and_reports_only_the_exposed_ones() {
        let mut grid = CellGrid::new(10, 20);
        assert_eq!(plan_scroll(&mut grid, 0, 3), ScrollPlan::Rows(7..10));
        assert_eq!(plan_scroll(&mut grid, 3, 0), ScrollPlan::Rows(0..3));
        assert_eq!(plan_scroll(&mut grid, 0, 0), ScrollPlan::Rows(0..0));
        assert_eq!(plan_scroll(&mut grid, 0, 10), ScrollPlan::Full);
        assert_eq!(plan_scroll(&mut grid, 50, 0), ScrollPlan::Full);
    }

    #[test]
    fn diff_gutter_width_holds_two_number_columns_and_a_marker() {
        // Three-digit floor on both sides: "  1   1 + " is 10 cells.
        assert_eq!(diff_gutter_width(1, 1), 10);
        assert_eq!(diff_gutter_width(1200, 1200), 12);
    }
}
