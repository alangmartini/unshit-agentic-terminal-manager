use super::Cell;
use std::ops::{Index, IndexMut, Range};
use std::sync::OnceLock;

/// Row-major logical cells backed by a circular buffer. Whole-screen line
/// shifts move the origin instead of copying every surviving row. Consumers
/// needing a contiguous snapshot pay for flattening only once per mutation.
#[derive(Debug)]
pub(super) struct Storage {
    cells: Vec<Cell>,
    cols: usize,
    origin: usize,
    contiguous: OnceLock<Vec<Cell>>,
}

impl Clone for Storage {
    fn clone(&self) -> Self {
        // Snapshots are commonly handed straight to the renderer. Flatten
        // while copying so cells() on the clone needs no second allocation.
        let mut cells = Vec::with_capacity(self.cells.len());
        cells.extend_from_slice(&self.cells[self.origin..]);
        cells.extend_from_slice(&self.cells[..self.origin]);
        Self::new(cells, self.cols)
    }
}

impl PartialEq for Storage {
    fn eq(&self, other: &Self) -> bool {
        self.cols == other.cols && self.as_slice() == other.as_slice()
    }
}

impl Storage {
    pub(super) fn new(cells: Vec<Cell>, cols: usize) -> Self {
        Self { cells, cols, origin: 0, contiguous: OnceLock::new() }
    }

    pub(super) fn as_slice(&self) -> &[Cell] {
        if self.origin == 0 {
            return &self.cells;
        }
        self.contiguous.get_or_init(|| {
            let mut cells = Vec::with_capacity(self.cells.len());
            cells.extend_from_slice(&self.cells[self.origin..]);
            cells.extend_from_slice(&self.cells[..self.origin]);
            cells
        })
    }

    fn normalize(&mut self) {
        self.contiguous.take();
        self.cells.rotate_left(self.origin);
        self.origin = 0;
    }

    fn physical_index(&self, index: usize) -> usize {
        assert!(index < self.cells.len());
        let shifted = index + self.origin;
        if shifted >= self.cells.len() {
            shifted - self.cells.len()
        } else {
            shifted
        }
    }

    pub(super) fn copy_within(&mut self, source: Range<usize>, destination: usize) {
        self.contiguous.take();
        if self.cols > 0
            && self.cells.len() > self.cols
            && source == (self.cols..self.cells.len())
            && destination == 0
        {
            // Preserve copy_within's duplicate final row, even though terminal
            // callers normally clear it next. The old first row becomes the
            // new final row; all surviving rows stay in their physical slots.
            let last = self.physical_index(self.cells.len() - self.cols);
            self.cells.copy_within(last..last + self.cols, self.origin);
            self.origin += self.cols;
            if self.origin == self.cells.len() {
                self.origin = 0;
            }
        } else {
            self.normalize();
            self.cells.copy_within(source, destination);
        }
    }

    pub(super) fn fill(&mut self, cell: Cell) {
        self.contiguous.take();
        self.cells.fill(cell);
        self.origin = 0;
    }

    pub(super) fn fill_from(&mut self, start: usize, cell: Cell) {
        // At most two physical slices cover a logical suffix.
        assert!(start <= self.cells.len());
        self.contiguous.take();
        if start == self.cells.len() {
            return;
        }
        let physical = self.physical_index(start);
        if physical < self.origin {
            self.cells[physical..self.origin].fill(cell);
        } else {
            self.cells[physical..].fill(cell);
            self.cells[..self.origin].fill(cell);
        }
    }

    pub(super) fn prepend_row(&mut self, cells: Vec<Cell>) {
        assert_eq!(cells.len(), self.cols);
        self.normalize();
        self.cells.splice(0..0, cells);
    }
}

impl Index<usize> for Storage {
    type Output = Cell;
    fn index(&self, index: usize) -> &Cell {
        &self.cells[self.physical_index(index)]
    }
}

impl IndexMut<usize> for Storage {
    fn index_mut(&mut self, index: usize) -> &mut Cell {
        self.contiguous.take();
        let physical = self.physical_index(index);
        &mut self.cells[physical]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrapped_storage_matches_contiguous_copy_and_mutation_semantics() {
        for rows in 1..8 {
            for cols in 1..9 {
                let mut expected: Vec<Cell> = (0..rows * cols)
                    .map(|i| Cell::with_char(char::from_u32(65 + i as u32).unwrap()))
                    .collect();
                let mut actual = Storage::new(expected.clone(), cols);
                for step in 0..40 {
                    let len = expected.len();
                    expected.copy_within(cols..len, 0);
                    actual.copy_within(cols..len, 0);
                    // Reading before a mutation must never leave stale cached ink.
                    assert_eq!(actual.as_slice(), expected);
                    let index = step % len;
                    expected[index] = Cell::with_char('!');
                    actual[index] = expected[index];
                    let suffix = (step * 7) % (len + 1);
                    expected[suffix..].fill(Cell::default());
                    actual.fill_from(suffix, Cell::default());
                    assert_eq!(actual.as_slice(), expected);
                    for (index, cell) in expected.iter().enumerate() {
                        assert_eq!(actual[index], *cell);
                    }
                    assert_eq!(actual.clone(), Storage::new(expected.clone(), cols));
                    if step % 5 == 0 {
                        // Partial and downward moves retain ordinary memmove behavior.
                        expected.copy_within(0..len - 1, 1);
                        actual.copy_within(0..len - 1, 1);
                        assert_eq!(actual.as_slice(), expected);
                    }
                }
                actual.prepend_row(vec![Cell::with_char('x'); cols]);
                expected.splice(0..0, vec![Cell::with_char('x'); cols]);
                assert_eq!(actual.as_slice(), expected);
                actual.fill(Cell::default());
                assert!(actual.as_slice().iter().all(|cell| *cell == Cell::default()));
            }
        }
    }

    #[test]
    fn empty_storage_supports_empty_operations() {
        let mut cells = Storage::new(Vec::new(), 0);
        cells.copy_within(0..0, 0);
        cells.fill_from(0, Cell::default());
        cells.prepend_row(Vec::new());
        cells.fill(Cell::default());
        assert!(cells.as_slice().is_empty());
    }
}
