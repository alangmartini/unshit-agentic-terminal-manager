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

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Cell {
    pub ch: char,
    pub fg: Color,
    pub bg: Color,
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
}
