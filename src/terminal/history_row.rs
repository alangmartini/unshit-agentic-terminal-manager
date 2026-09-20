use super::{Cell, Color};
use std::borrow::Cow;

/// History owns either full cells or characters sharing one exact style.
/// Reading a compact row never installs a second, decoded copy in history.
#[derive(Clone, Debug)]
pub(super) enum HistoryRow {
    Cells(Vec<Cell>),
    Uniform { chars: Vec<char>, template: Cell },
}

impl HistoryRow {
    pub(super) fn from_cells(cells: &[Cell], reuse: Option<Self>) -> Self {
        if let Some(template) = Self::uniform_template(cells) {
            let mut chars = match reuse {
                Some(Self::Uniform { chars, .. }) if reusable(chars.capacity(), cells.len()) => {
                    chars
                }
                _ => Vec::new(),
            };
            chars.clear();
            chars.extend(cells.iter().map(|cell| cell.ch));
            return Self::Uniform { chars, template };
        }
        let mut stored = match reuse {
            Some(Self::Cells(stored)) if reusable(stored.capacity(), cells.len()) => stored,
            _ => Vec::new(),
        };
        stored.clear();
        stored.extend_from_slice(cells);
        Self::Cells(stored)
    }

    fn uniform_template(cells: &[Cell]) -> Option<Cell> {
        if let Some(first) = cells.first().copied() {
            let same_style = |cell: &Cell| {
                cell.fg == first.fg
                    && cell.bg == first.bg
                    && cell.attrs == first.attrs
                    && cell.wide_continuation == first.wide_continuation
            };
            // Cheap rejection for styled text followed by default blanks or
            // styles changing near the start. These probes only reject;
            // compression still requires checking every cell below.
            if same_style(&cells[cells.len() - 1]) && same_style(&cells[cells.len() / 8]) {
                let pack = |color: Color| u32::from_ne_bytes([color.r, color.g, color.b, color.a]);
                // OR reduction permits vectorization without allowing equal
                // differences at two positions to cancel each other out.
                let different = cells.iter().fold(0u32, |different, cell| {
                    different
                        | (pack(cell.fg) ^ pack(first.fg))
                        | (pack(cell.bg) ^ pack(first.bg))
                        | u32::from(cell.attrs.bits() ^ first.attrs.bits())
                        | u32::from(cell.wide_continuation != first.wide_continuation)
                });
                if different == 0 {
                    return Some(first);
                }
            }
        }
        None
    }

    pub(super) fn len(&self) -> usize {
        match self {
            Self::Cells(cells) => cells.len(),
            Self::Uniform { chars, .. } => chars.len(),
        }
    }

    pub(super) fn get(&self, index: usize) -> Option<Cell> {
        match self {
            Self::Cells(cells) => cells.get(index).copied(),
            Self::Uniform { chars, template } => {
                chars.get(index).map(|&ch| Cell { ch, ..*template })
            }
        }
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = Cell> + '_ {
        (0..self.len()).map(|index| self.get(index).unwrap())
    }

    pub(super) fn cells(&self) -> Cow<'_, [Cell]> {
        match self {
            Self::Cells(cells) => Cow::Borrowed(cells),
            Self::Uniform { .. } => Cow::Owned(self.iter().collect()),
        }
    }
}

fn reusable(capacity: usize, width: usize) -> bool {
    // Avoid retaining very wide allocations after a terminal narrows.
    capacity
        <= if width == 0 {
            0
        } else {
            width.saturating_mul(2).max(4)
        }
}

#[cfg(test)]
impl PartialEq for HistoryRow {
    fn eq(&self, other: &Self) -> bool {
        self.iter().eq(other.iter())
    }
}

impl From<Vec<Cell>> for HistoryRow {
    fn from(cells: Vec<Cell>) -> Self {
        match Self::uniform_template(&cells) {
            Some(template) => Self::Uniform {
                chars: cells.iter().map(|cell| cell.ch).collect(),
                template,
            },
            None => Self::Cells(cells),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::CellAttrs;

    fn sample(width: usize) -> Vec<Cell> {
        let chars = ['\0', ' ', 'x', '中', '🙂'];
        (0..width)
            .map(|i| Cell {
                ch: chars[i % chars.len()],
                fg: Color {
                    r: 1,
                    g: 2,
                    b: 3,
                    a: 4,
                },
                bg: Color {
                    r: 11,
                    g: 12,
                    b: 13,
                    a: 14,
                },
                attrs: CellAttrs::BOLD,
                wide_continuation: false,
            })
            .collect()
    }

    #[test]
    fn owned_rows_are_classified_without_copying_mixed_cells() {
        let uniform = HistoryRow::from(sample(125));
        assert!(matches!(uniform, HistoryRow::Uniform { .. }));
        let mut mixed = sample(125);
        mixed[30].bg.a ^= 128;
        let address = mixed.as_ptr();
        let expected = mixed.clone();
        let row = HistoryRow::from(mixed);
        assert!(matches!(&row, HistoryRow::Cells(cells) if cells.as_ptr() == address));
        assert_eq!(row.cells().as_ref(), expected);
    }

    #[test]
    fn uniform_rows_stay_compact_after_reads_and_clone() {
        for width in [1, 2, 7, 125, 1024] {
            let cells = sample(width);
            let row = HistoryRow::from_cells(&cells, None);
            for candidate in [&row, &row.clone()] {
                assert_eq!(candidate.cells().as_ref(), cells);
                assert_eq!(candidate.iter().collect::<Vec<_>>(), cells);
                assert_eq!(candidate.get(width), None);
                assert!(
                    matches!(candidate, HistoryRow::Uniform { chars, .. } if chars.len() == width)
                );
            }
        }
    }

    #[test]
    fn every_style_field_is_preserved_at_every_column() {
        for index in 0..125 {
            for field in 0..10 {
                let mut cells = sample(125);
                let cell = &mut cells[index];
                match field {
                    0 => cell.fg.r ^= 128,
                    1 => cell.fg.g ^= 128,
                    2 => cell.fg.b ^= 128,
                    3 => cell.fg.a ^= 128,
                    4 => cell.bg.r ^= 128,
                    5 => cell.bg.g ^= 128,
                    6 => cell.bg.b ^= 128,
                    7 => cell.bg.a ^= 128,
                    8 => cell.attrs |= CellAttrs::ITALIC,
                    _ => cell.wide_continuation = true,
                }
                let row = HistoryRow::from_cells(&cells, None);
                assert_eq!(row.cells().as_ref(), cells, "column={index} field={field}");
            }
        }
        let mut cells = sample(125);
        cells[21].fg.r ^= 128;
        cells[22].fg.r ^= 128;
        assert_eq!(HistoryRow::from_cells(&cells, None).cells().as_ref(), cells);
    }

    #[test]
    fn recycling_handles_style_changes_empty_rows_and_width_shrinks() {
        let mut reuse = None;
        for (width, mixed) in [
            (125, false),
            (125, false),
            (125, true),
            (125, true),
            (7, true),
            (7, false),
            (0, false),
        ] {
            let mut cells = sample(width);
            if mixed {
                cells[0].fg.r ^= 128;
            }
            let row = HistoryRow::from_cells(&cells, reuse);
            assert_eq!(row.cells().as_ref(), cells);
            let capacity = match &row {
                HistoryRow::Cells(c) => c.capacity(),
                HistoryRow::Uniform { chars, .. } => chars.capacity(),
            };
            assert!(capacity <= (width * 2).max(4));
            if width == 0 {
                assert_eq!(capacity, 0);
            }
            reuse = Some(row);
        }
    }

    #[test]
    fn recycling_retains_matching_allocations_without_stale_content() {
        for mixed in [false, true] {
            let mut before = sample(125);
            if mixed {
                before[0].fg.r ^= 128;
            }
            let old = HistoryRow::from_cells(&before, None);
            let address = match &old {
                HistoryRow::Cells(cells) => cells.as_ptr().cast::<u8>(),
                HistoryRow::Uniform { chars, .. } => chars.as_ptr().cast::<u8>(),
            };
            let mut after = before.clone();
            after[3].ch = 'Z';
            let new = HistoryRow::from_cells(&after, Some(old));
            let reused_address = match &new {
                HistoryRow::Cells(cells) => cells.as_ptr().cast::<u8>(),
                HistoryRow::Uniform { chars, .. } => chars.as_ptr().cast::<u8>(),
            };
            assert_eq!(address, reused_address);
            assert_eq!(new.cells().as_ref(), after);
        }
    }
}
