use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::ops::{Index, IndexMut};

use crate::cell::Cell;

/// A flat cell sequence whose first row can rotate without moving the grid.
/// Serialization remains the original logical-order sequence of cells.
#[derive(Debug, Clone)]
pub(super) struct Storage {
    cells: Vec<Cell>,
    start: usize,
}

impl From<Vec<Cell>> for Storage {
    fn from(cells: Vec<Cell>) -> Self {
        Self { cells, start: 0 }
    }
}

impl Storage {
    pub(super) fn get(&self, index: usize) -> Option<&Cell> {
        if index >= self.cells.len() {
            return None;
        }
        self.cells.get(self.physical(index))
    }

    fn physical(&self, index: usize) -> usize {
        let tail = self.cells.len() - self.start;
        if index < tail {
            self.start + index
        } else {
            index - tail
        }
    }

    pub(super) fn row(&self, start: usize, cols: usize) -> &[Cell] {
        if cols == 0 {
            return &[];
        }
        let physical = self.physical(start);
        &self.cells[physical..physical + cols]
    }

    pub(super) fn row_mut(&mut self, start: usize, cols: usize) -> &mut [Cell] {
        if cols == 0 {
            return &mut [];
        }
        let physical = self.physical(start);
        &mut self.cells[physical..physical + cols]
    }

    pub(super) fn fill(&mut self, cell: Cell) {
        self.cells.fill(cell);
    }

    pub(super) fn scroll_up(&mut self, cols: usize) -> Vec<Cell> {
        // Grid rows always align with physical rows, so a row never straddles
        // the wrap. The old first row becomes the new blank last row.
        let evicted = self.row(0, cols).to_vec();
        self.advance_row(cols);
        evicted
    }

    pub(super) fn advance_row(&mut self, cols: usize) {
        self.row_mut(0, cols).fill(Cell::BLANK);
        self.start = self.physical(cols);
    }

    fn iter(&self) -> impl Iterator<Item = &Cell> {
        self.cells[self.start..]
            .iter()
            .chain(&self.cells[..self.start])
    }
}

impl Index<usize> for Storage {
    type Output = Cell;
    fn index(&self, index: usize) -> &Cell {
        assert!(index < self.cells.len());
        &self.cells[self.physical(index)]
    }
}

impl IndexMut<usize> for Storage {
    fn index_mut(&mut self, index: usize) -> &mut Cell {
        assert!(index < self.cells.len());
        let physical = self.physical(index);
        &mut self.cells[physical]
    }
}

impl PartialEq for Storage {
    fn eq(&self, other: &Self) -> bool {
        self.iter().eq(other.iter())
    }
}

impl Serialize for Storage {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.iter())
    }
}

impl<'de> Deserialize<'de> for Storage {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Vec::<Cell>::deserialize(deserializer).map(Self::from)
    }
}
