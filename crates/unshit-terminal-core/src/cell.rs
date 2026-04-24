use bitflags::bitflags;
use serde::{Deserialize, Serialize};

use crate::color::Color;

bitflags! {
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct CellAttrs: u8 {
        const BOLD          = 0b0000_0001;
        const ITALIC        = 0b0000_0010;
        const UNDERLINE     = 0b0000_0100;
        const STRIKETHROUGH = 0b0000_1000;
        const INVERSE       = 0b0001_0000;
        const DIM           = 0b0010_0000;
        const BLINK         = 0b0100_0000;
    }
}

// Per-field defaults paired with `skip_serializing_if` predicates so a
// blank cell serializes to `{}` (two bytes) instead of the full ~90-byte
// record. With a 1 MiB frame cap a 150-col x 145-row snapshot of mostly
// blank cells otherwise blows past the cap; keeping the common case
// compact is what lets `attach_session` round-trip realistic
// scrollback without tripping `FrameTooLarge`.
fn default_ch() -> char {
    ' '
}
fn is_default_ch(c: &char) -> bool {
    *c == ' '
}

fn default_fg() -> Color {
    Color::WHITE
}
fn is_default_fg(c: &Color) -> bool {
    *c == Color::WHITE
}

fn default_bg() -> Color {
    Color::TRANSPARENT
}
fn is_default_bg(c: &Color) -> bool {
    *c == Color::TRANSPARENT
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Cell {
    #[serde(default = "default_ch", skip_serializing_if = "is_default_ch")]
    pub ch: char,
    #[serde(default = "default_fg", skip_serializing_if = "is_default_fg")]
    pub fg: Color,
    #[serde(default = "default_bg", skip_serializing_if = "is_default_bg")]
    pub bg: Color,
    #[serde(default, skip_serializing_if = "CellAttrs::is_empty")]
    pub attrs: CellAttrs,
}

impl Cell {
    pub const BLANK: Cell = Cell {
        ch: ' ',
        fg: Color::WHITE,
        bg: Color::TRANSPARENT,
        attrs: CellAttrs::empty(),
    };

    pub fn new(ch: char, fg: Color, bg: Color, attrs: CellAttrs) -> Self {
        Self { ch, fg, bg, attrs }
    }
}

impl Default for Cell {
    fn default() -> Self {
        Self::BLANK
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_is_space_with_transparent_bg() {
        assert_eq!(Cell::BLANK.ch, ' ');
        assert_eq!(Cell::BLANK.bg, Color::TRANSPARENT);
        assert_eq!(Cell::BLANK.attrs, CellAttrs::empty());
    }

    #[test]
    fn new_constructs_requested_fields() {
        let c = Cell::new('x', Color::WHITE, Color::BLACK, CellAttrs::BOLD);
        assert_eq!(c.ch, 'x');
        assert_eq!(c.fg, Color::WHITE);
        assert_eq!(c.bg, Color::BLACK);
        assert_eq!(c.attrs, CellAttrs::BOLD);
    }

    #[test]
    fn default_equals_blank() {
        assert_eq!(Cell::default(), Cell::BLANK);
    }

    #[test]
    fn cell_attrs_serde_json_round_trip() {
        let a = CellAttrs::BOLD | CellAttrs::UNDERLINE | CellAttrs::INVERSE;
        let j = serde_json::to_string(&a).unwrap();
        let back: CellAttrs = serde_json::from_str(&j).unwrap();
        assert_eq!(back, a);
    }

    #[test]
    fn cell_attrs_bincode_round_trip() {
        let a = CellAttrs::ITALIC | CellAttrs::DIM | CellAttrs::BLINK;
        let bytes = bincode::serialize(&a).unwrap();
        let back: CellAttrs = bincode::deserialize(&bytes).unwrap();
        assert_eq!(back, a);
    }

    #[test]
    fn cell_serde_json_round_trip() {
        let c = Cell::new(
            '!',
            Color::rgb(1, 2, 3),
            Color::rgb(4, 5, 6),
            CellAttrs::BOLD,
        );
        let j = serde_json::to_string(&c).unwrap();
        let back: Cell = serde_json::from_str(&j).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn blank_cell_serializes_to_empty_object() {
        // A blank cell MUST collapse to `{}` on the wire so snapshots
        // stay under `MAX_FRAME_LEN` (1 MiB) even for large grids with
        // long scrollback. Each reinstated field is ~20-30 bytes of JSON
        // times thousands of cells, so regressing this tips snapshots
        // past the frame cap and deadlocks `attach_session`.
        let j = serde_json::to_string(&Cell::BLANK).unwrap();
        assert_eq!(j, "{}", "blank cell should serialize to empty object");
        let back: Cell = serde_json::from_str("{}").unwrap();
        assert_eq!(back, Cell::BLANK);
    }

    #[test]
    fn partial_cell_omits_default_fields() {
        // A cell that only differs from blank on `ch` should only write
        // `ch`. Same idea for each other field.
        let mut c = Cell::BLANK;
        c.ch = 'x';
        assert_eq!(serde_json::to_string(&c).unwrap(), "{\"ch\":\"x\"}");
        let round: Cell = serde_json::from_str("{\"ch\":\"x\"}").unwrap();
        assert_eq!(round, c);
    }

    #[test]
    fn snapshot_fits_within_one_mib_for_realistic_grid() {
        // Regression: a 150x45 grid plus 100 scrollback rows of mostly
        // blank cells was serializing to ~2 MiB with the verbose
        // per-cell format, tripping `ProtocolError::FrameTooLarge` on
        // the daemon and deadlocking the UI's `attach_session`. With
        // compact blank cells the same snapshot fits well under 1 MiB.
        use crate::grid::Grid;
        use crate::snapshot::Snapshot;
        let grid = Grid::new(45, 150);
        let scrollback: Vec<Vec<Cell>> = (0..100).map(|_| vec![Cell::BLANK; 150]).collect();
        let snap = Snapshot { grid, scrollback };
        let bytes = serde_json::to_vec(&snap).unwrap();
        assert!(
            bytes.len() < 1024 * 1024,
            "mostly-blank 150x45 + 100 scrollback snapshot is {} bytes \
             (over the 1 MiB frame cap)",
            bytes.len()
        );
    }
}
