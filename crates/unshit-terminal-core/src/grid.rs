use serde::{Deserialize, Serialize};

use crate::cell::Cell;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Grid {
    rows: usize,
    cols: usize,
    cells: Vec<Cell>,
    cursor_row: usize,
    cursor_col: usize,
    cursor_visible: bool,
}

impl Grid {
    pub fn new(rows: usize, cols: usize) -> Self {
        Self {
            rows,
            cols,
            cells: vec![Cell::BLANK; rows * cols],
            cursor_row: 0,
            cursor_col: 0,
            cursor_visible: true,
        }
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    pub fn cols(&self) -> usize {
        self.cols
    }

    pub fn cursor(&self) -> (usize, usize) {
        (self.cursor_row, self.cursor_col)
    }

    pub fn cursor_visible(&self) -> bool {
        self.cursor_visible
    }

    pub fn set_cursor(&mut self, row: usize, col: usize) {
        self.cursor_row = row.min(self.rows.saturating_sub(1));
        self.cursor_col = col.min(self.cols.saturating_sub(1));
    }

    pub fn set_cursor_visible(&mut self, visible: bool) {
        self.cursor_visible = visible;
    }

    pub fn get(&self, row: usize, col: usize) -> Option<&Cell> {
        if row < self.rows && col < self.cols {
            self.cells.get(row * self.cols + col)
        } else {
            None
        }
    }

    pub fn set(&mut self, row: usize, col: usize, cell: Cell) {
        if row < self.rows && col < self.cols {
            let idx = row * self.cols + col;
            self.cells[idx] = cell;
        }
    }

    pub fn row(&self, row: usize) -> Option<&[Cell]> {
        if row < self.rows {
            let start = row * self.cols;
            Some(&self.cells[start..start + self.cols])
        } else {
            None
        }
    }

    pub fn resize(&mut self, rows: usize, cols: usize) {
        if rows == self.rows && cols == self.cols {
            return;
        }
        let mut next = vec![Cell::BLANK; rows * cols];
        let copy_rows = rows.min(self.rows);
        let copy_cols = cols.min(self.cols);
        for r in 0..copy_rows {
            for c in 0..copy_cols {
                next[r * cols + c] = self.cells[r * self.cols + c];
            }
        }
        self.cells = next;
        self.rows = rows;
        self.cols = cols;
        if self.rows == 0 {
            self.cursor_row = 0;
        } else if self.cursor_row >= self.rows {
            self.cursor_row = self.rows - 1;
        }
        if self.cols == 0 {
            self.cursor_col = 0;
        } else if self.cursor_col >= self.cols {
            self.cursor_col = self.cols - 1;
        }
    }

    pub fn erase_to_line_end(&mut self, row: usize, col: usize) {
        if row >= self.rows {
            return;
        }
        let start = row * self.cols + col.min(self.cols);
        let end = (row + 1) * self.cols;
        for slot in &mut self.cells[start..end] {
            *slot = Cell::BLANK;
        }
    }

    pub fn erase_all(&mut self) {
        for slot in &mut self.cells {
            *slot = Cell::BLANK;
        }
    }

    pub fn scroll_up(&mut self) -> Vec<Cell> {
        if self.rows == 0 || self.cols == 0 {
            return Vec::new();
        }
        let evicted: Vec<Cell> = self.cells[..self.cols].to_vec();
        self.cells.copy_within(self.cols.., 0);
        let tail_start = (self.rows - 1) * self.cols;
        for slot in &mut self.cells[tail_start..] {
            *slot = Cell::BLANK;
        }
        evicted
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::CellAttrs;
    use crate::color::Color;

    #[test]
    fn new_grid_is_blank_with_cursor_at_origin() {
        let g = Grid::new(3, 5);
        assert_eq!(g.rows(), 3);
        assert_eq!(g.cols(), 5);
        assert_eq!(g.cursor(), (0, 0));
        assert!(g.cursor_visible());
        for r in 0..3 {
            for c in 0..5 {
                assert_eq!(g.get(r, c), Some(&Cell::BLANK));
            }
        }
    }

    #[test]
    fn set_and_get_round_trip() {
        let mut g = Grid::new(3, 5);
        let cell = Cell::new('z', Color::WHITE, Color::BLACK, CellAttrs::BOLD);
        g.set(1, 2, cell);
        assert_eq!(g.get(1, 2), Some(&cell));
        assert_eq!(g.get(0, 0), Some(&Cell::BLANK));
    }

    #[test]
    fn get_out_of_bounds_returns_none() {
        let g = Grid::new(2, 2);
        assert_eq!(g.get(2, 0), None);
        assert_eq!(g.get(0, 2), None);
    }

    #[test]
    fn row_slice_has_expected_length() {
        let g = Grid::new(2, 5);
        assert_eq!(g.row(0).unwrap().len(), 5);
        assert!(g.row(2).is_none());
    }

    #[test]
    fn set_cursor_clamps_into_bounds() {
        let mut g = Grid::new(3, 4);
        g.set_cursor(100, 100);
        assert_eq!(g.cursor(), (2, 3));
    }

    #[test]
    fn resize_preserves_top_left_and_clamps_cursor() {
        let mut g = Grid::new(2, 3);
        let x = Cell::new('x', Color::WHITE, Color::BLACK, CellAttrs::empty());
        g.set(1, 2, x);
        g.set_cursor(1, 2);

        g.resize(1, 2);
        assert_eq!(g.rows(), 1);
        assert_eq!(g.cols(), 2);
        assert_eq!(g.cursor(), (0, 1));
        // The cell at (1,2) is outside the new bounds so it was dropped.
        assert_eq!(g.get(0, 0), Some(&Cell::BLANK));
    }

    #[test]
    fn resize_growth_fills_new_area_with_blank() {
        let mut g = Grid::new(1, 2);
        let x = Cell::new('x', Color::WHITE, Color::BLACK, CellAttrs::empty());
        g.set(0, 0, x);
        g.resize(2, 4);
        assert_eq!(g.get(0, 0), Some(&x));
        assert_eq!(g.get(0, 3), Some(&Cell::BLANK));
        assert_eq!(g.get(1, 1), Some(&Cell::BLANK));
    }

    #[test]
    fn erase_to_line_end_only_clears_from_col() {
        let mut g = Grid::new(1, 5);
        let a = Cell::new('a', Color::WHITE, Color::BLACK, CellAttrs::empty());
        for c in 0..5 {
            g.set(0, c, a);
        }
        g.erase_to_line_end(0, 2);
        assert_eq!(g.get(0, 0), Some(&a));
        assert_eq!(g.get(0, 1), Some(&a));
        assert_eq!(g.get(0, 2), Some(&Cell::BLANK));
        assert_eq!(g.get(0, 4), Some(&Cell::BLANK));
    }

    #[test]
    fn erase_all_clears_grid() {
        let mut g = Grid::new(2, 2);
        let a = Cell::new('a', Color::WHITE, Color::BLACK, CellAttrs::empty());
        g.set(0, 0, a);
        g.set(1, 1, a);
        g.erase_all();
        for r in 0..2 {
            for c in 0..2 {
                assert_eq!(g.get(r, c), Some(&Cell::BLANK));
            }
        }
    }

    #[test]
    fn scroll_up_returns_evicted_row_and_blanks_bottom() {
        let mut g = Grid::new(3, 3);
        let top = Cell::new('t', Color::WHITE, Color::BLACK, CellAttrs::empty());
        let mid = Cell::new('m', Color::WHITE, Color::BLACK, CellAttrs::empty());
        let bot = Cell::new('b', Color::WHITE, Color::BLACK, CellAttrs::empty());
        for c in 0..3 {
            g.set(0, c, top);
            g.set(1, c, mid);
            g.set(2, c, bot);
        }

        let evicted = g.scroll_up();
        assert_eq!(evicted, vec![top, top, top]);
        assert_eq!(g.row(0).unwrap(), &[mid, mid, mid]);
        assert_eq!(g.row(1).unwrap(), &[bot, bot, bot]);
        assert_eq!(g.row(2).unwrap(), &[Cell::BLANK, Cell::BLANK, Cell::BLANK]);
    }

    #[test]
    fn set_cursor_visible_flips_state() {
        let mut g = Grid::new(1, 1);
        assert!(g.cursor_visible());
        g.set_cursor_visible(false);
        assert!(!g.cursor_visible());
        g.set_cursor_visible(true);
        assert!(g.cursor_visible());
    }
}
