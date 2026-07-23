//! Paints an `EditorBuffer` viewport into a `CellGrid`.
//!
//! The editor reuses the terminal's grid primitive so it inherits the
//! renderer's damage tracking and the `LineQuadCache`'s stable-line-id
//! replay. Only the visible window is ever painted; scrolls move rows
//! with `shift_rows` so unchanged lines keep their identity (and their
//! cached quads) across the move. Selection is painted as cell
//! background directly into the live grid — selection changes damage
//! exactly the lines they touch.

use unshit::core::cell_grid::{Cell, CellGrid, ANSI_16};
use unshit::core::style::types::Color;

use super::buffer::{EditorBuffer, Position};

/// Dim gray for gutter line numbers (ANSI bright black).
const GUTTER_FG: Color = ANSI_16[8];
const TEXT_FG: Color = Color::WHITE;
/// Selection background (VS Code dark `#264f78`).
const SELECTION_BG: Color = Color {
    r: 0x26,
    g: 0x4f,
    b: 0x78,
    a: 0xff,
};

/// Gutter width in cells: right-aligned line number plus one space,
/// with a 3-digit floor so short files don't jitter the text column.
pub fn gutter_width(line_count: usize) -> usize {
    let digits = line_count.max(1).ilog10() as usize + 1;
    digits.max(3) + 1
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

/// Paint one grid row from `line_idx` of the buffer. Rows past the end
/// of the buffer are blank (no gutter number, mirroring code editors).
pub fn paint_row(
    grid: &mut CellGrid,
    row: usize,
    line_idx: usize,
    buffer: &EditorBuffer,
    h_offset: usize,
    gutter_w: usize,
    selection: Option<(Position, Position)>,
) {
    let cols = grid.cols();
    let Some(line) = buffer.line(line_idx) else {
        for col in 0..cols {
            grid.set_cell(row, col, Cell::default());
        }
        return;
    };

    // Right-aligned line number, one trailing space of separation.
    let number = (line_idx + 1).to_string();
    let digit_cells = gutter_w.saturating_sub(1);
    let pad = digit_cells.saturating_sub(number.len());
    for col in 0..gutter_w.min(cols) {
        let ch = if col >= pad && col < pad + number.len() {
            number.as_bytes()[col - pad] as char
        } else {
            ' '
        };
        grid.set_cell(
            row,
            col,
            Cell {
                ch,
                fg: GUTTER_FG,
                ..Default::default()
            },
        );
    }

    // Content cells, skipping `h_offset` leading characters. Byte
    // offsets ride along so selection membership is exact.
    let mut col = gutter_w;
    for (byte, ch) in line.char_indices().skip(h_offset) {
        if col >= cols {
            break;
        }
        let bg = if in_selection(selection, line_idx, byte) {
            SELECTION_BG
        } else {
            Color::TRANSPARENT
        };
        grid.set_cell(
            row,
            col,
            Cell {
                ch,
                fg: TEXT_FG,
                bg,
                ..Default::default()
            },
        );
        col += 1;
    }
    // A selection that continues past the end of this line paints one
    // trailing marker cell (the newline), like every code editor.
    if col < cols && in_selection(selection, line_idx, line.len()) {
        grid.set_cell(
            row,
            col,
            Cell {
                ch: ' ',
                fg: TEXT_FG,
                bg: SELECTION_BG,
                ..Default::default()
            },
        );
        col += 1;
    }
    for c in col..cols {
        grid.set_cell(row, c, Cell::default());
    }
}

/// Repaint every viewport row from scratch, resetting each row's line
/// identity. Used on load, resize, horizontal scroll, and viewport jumps
/// larger than the viewport itself, where identity continuity is moot.
pub fn repaint_all(
    grid: &mut CellGrid,
    buffer: &EditorBuffer,
    top_line: usize,
    h_offset: usize,
    selection: Option<(Position, Position)>,
) {
    let gutter_w = gutter_width(buffer.line_count());
    for row in 0..grid.rows() {
        grid.reset_line_identity(row);
        paint_row(
            grid,
            row,
            top_line + row,
            buffer,
            h_offset,
            gutter_w,
            selection,
        );
    }
}

/// Scroll the painted viewport from `old_top` to `new_top`, shifting
/// surviving rows (stable line ids ride along) and painting only the
/// newly exposed rows.
pub fn scroll_viewport(
    grid: &mut CellGrid,
    buffer: &EditorBuffer,
    old_top: usize,
    new_top: usize,
    h_offset: usize,
    selection: Option<(Position, Position)>,
) {
    if new_top == old_top {
        return;
    }
    let rows = grid.rows();
    let delta = new_top.abs_diff(old_top);
    if delta >= rows {
        repaint_all(grid, buffer, new_top, h_offset, selection);
        return;
    }
    let gutter_w = gutter_width(buffer.line_count());
    let keep = rows - delta;
    if new_top > old_top {
        // Content moves up: rows delta..rows shift to 0..keep.
        grid.shift_rows(0, delta, keep);
        for row in keep..rows {
            grid.reset_line_identity(row);
            paint_row(
                grid,
                row,
                new_top + row,
                buffer,
                h_offset,
                gutter_w,
                selection,
            );
        }
    } else {
        // Content moves down: rows 0..keep shift to delta..rows.
        grid.shift_rows(delta, 0, keep);
        for row in 0..delta {
            grid.reset_line_identity(row);
            paint_row(
                grid,
                row,
                new_top + row,
                buffer,
                h_offset,
                gutter_w,
                selection,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buffer_of(n: usize) -> EditorBuffer {
        let text: Vec<String> = (0..n).map(|i| format!("line {}", i)).collect();
        EditorBuffer::from_text(&text.join("\n"))
    }

    fn row_text(grid: &CellGrid, row: usize) -> String {
        (0..grid.cols())
            .map(|c| {
                let ch = grid.get_cell(row, c).map(|cell| cell.ch).unwrap_or('\0');
                if ch == '\0' {
                    ' '
                } else {
                    ch
                }
            })
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    #[test]
    fn gutter_width_scales_with_line_count() {
        assert_eq!(gutter_width(1), 4);
        assert_eq!(gutter_width(999), 4);
        assert_eq!(gutter_width(1000), 5);
    }

    #[test]
    fn repaint_renders_gutter_and_content() {
        let buffer = buffer_of(50);
        let mut grid = CellGrid::new(4, 20);
        repaint_all(&mut grid, &buffer, 0, 0, None);
        assert_eq!(row_text(&grid, 0), "  1 line 0");
        assert_eq!(row_text(&grid, 3), "  4 line 3");
    }

    #[test]
    fn rows_past_buffer_end_are_blank() {
        let buffer = buffer_of(2);
        let mut grid = CellGrid::new(4, 20);
        repaint_all(&mut grid, &buffer, 0, 0, None);
        assert_eq!(row_text(&grid, 2), "");
        assert_eq!(row_text(&grid, 3), "");
    }

    #[test]
    fn scroll_down_keeps_line_ids_of_surviving_rows() {
        let buffer = buffer_of(100);
        let mut grid = CellGrid::new(10, 20);
        repaint_all(&mut grid, &buffer, 0, 0, None);
        let ids_before = grid.line_ids().to_vec();
        scroll_viewport(&mut grid, &buffer, 0, 3, 0, None);
        let ids_after = grid.line_ids().to_vec();
        // Rows 3..10 moved to 0..7 and kept their identities.
        assert_eq!(&ids_after[0..7], &ids_before[3..10]);
        // Content matches the new window.
        assert_eq!(row_text(&grid, 0), "  4 line 3");
        assert_eq!(row_text(&grid, 9), " 13 line 12");
    }

    #[test]
    fn scroll_up_restores_earlier_lines() {
        let buffer = buffer_of(100);
        let mut grid = CellGrid::new(10, 20);
        repaint_all(&mut grid, &buffer, 20, 0, None);
        scroll_viewport(&mut grid, &buffer, 20, 18, 0, None);
        assert_eq!(row_text(&grid, 0), " 19 line 18");
        assert_eq!(row_text(&grid, 2), " 21 line 20");
    }

    #[test]
    fn jump_larger_than_viewport_repaints() {
        let buffer = buffer_of(100);
        let mut grid = CellGrid::new(10, 20);
        repaint_all(&mut grid, &buffer, 0, 0, None);
        scroll_viewport(&mut grid, &buffer, 0, 50, 0, None);
        assert_eq!(row_text(&grid, 0), " 51 line 50");
    }

    #[test]
    fn h_offset_skips_leading_chars() {
        let buffer = EditorBuffer::from_text("abcdefgh");
        let mut grid = CellGrid::new(1, 8);
        repaint_all(&mut grid, &buffer, 0, 2, None);
        // gutter "  1 " then content starting at 'c'.
        assert_eq!(row_text(&grid, 0), "  1 cdef");
    }

    #[test]
    fn selection_paints_background_on_selected_cells() {
        let buffer = EditorBuffer::from_text("hello\nworld");
        let mut grid = CellGrid::new(2, 12);
        let sel = Some((Position { line: 0, col: 1 }, Position { line: 1, col: 2 }));
        repaint_all(&mut grid, &buffer, 0, 0, sel);
        let gutter_w = gutter_width(2);
        // 'h' unselected, 'e'..'o' selected on line 0, plus the
        // trailing newline marker cell.
        let bg_of = |row: usize, col: usize| grid.get_cell(row, col).unwrap().bg;
        assert_eq!(bg_of(0, gutter_w), Color::TRANSPARENT);
        assert_eq!(bg_of(0, gutter_w + 1), SELECTION_BG);
        assert_eq!(bg_of(0, gutter_w + 4), SELECTION_BG);
        assert_eq!(bg_of(0, gutter_w + 5), SELECTION_BG); // newline marker
        assert_eq!(bg_of(1, gutter_w), SELECTION_BG); // 'w'
        assert_eq!(bg_of(1, gutter_w + 1), SELECTION_BG); // 'o'
        assert_eq!(bg_of(1, gutter_w + 2), Color::TRANSPARENT); // 'r'
    }
}
