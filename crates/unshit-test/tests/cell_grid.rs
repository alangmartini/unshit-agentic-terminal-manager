//! Tests for the CellGrid rendering primitive.

use unshit_core::cell_grid::{color_256, Cell, CellAttrs, CellGrid, ANSI_16};
use unshit_core::style::types::Color;

// ---------------------------------------------------------------------------
// Grid creation
// ---------------------------------------------------------------------------

#[test]
fn grid_creation_allocates_correct_number_of_cells() {
    let g = CellGrid::new(24, 80);
    assert_eq!(g.cells().len(), 24 * 80);
    assert_eq!(g.rows(), 24);
    assert_eq!(g.cols(), 80);
}

#[test]
fn grid_creation_all_cells_default_empty() {
    let g = CellGrid::new(4, 4);
    for cell in g.cells() {
        assert!(cell.is_empty());
        assert_eq!(cell.fg, Color::WHITE);
        assert_eq!(cell.bg, Color::TRANSPARENT);
        assert_eq!(cell.attrs, CellAttrs::empty());
    }
}

// ---------------------------------------------------------------------------
// set_cell / get_cell roundtrip
// ---------------------------------------------------------------------------

#[test]
fn set_get_cell_roundtrip() {
    let mut g = CellGrid::new(10, 20);
    let cell = Cell {
        ch: 'A',
        fg: Color::rgb(255, 0, 0),
        bg: Color::rgb(0, 0, 255),
        attrs: CellAttrs::BOLD | CellAttrs::UNDERLINE,
        wide_continuation: false,
    };
    g.set_cell(3, 7, cell);
    let got = g.get_cell(3, 7).unwrap();
    assert_eq!(got.ch, 'A');
    assert_eq!(got.fg, Color::rgb(255, 0, 0));
    assert_eq!(got.bg, Color::rgb(0, 0, 255));
    assert_eq!(got.attrs, CellAttrs::BOLD | CellAttrs::UNDERLINE);
}

#[test]
fn set_cell_out_of_bounds_ignored() {
    let mut g = CellGrid::new(5, 5);
    // Should not panic
    g.set_cell(99, 99, Cell::with_char('X'));
    assert!(g.get_cell(99, 99).is_none());
}

// ---------------------------------------------------------------------------
// ANSI 16 color mapping
// ---------------------------------------------------------------------------

#[test]
fn ansi_16_standard_black() {
    assert_eq!(ANSI_16[0], Color::rgb(0, 0, 0));
}

#[test]
fn ansi_16_standard_red() {
    assert_eq!(ANSI_16[1], Color::rgb(170, 0, 0));
}

#[test]
fn ansi_16_bright_white() {
    assert_eq!(ANSI_16[15], Color::rgb(255, 255, 255));
}

#[test]
fn ansi_16_count() {
    assert_eq!(ANSI_16.len(), 16);
}

// ---------------------------------------------------------------------------
// 256-color palette lookup
// ---------------------------------------------------------------------------

#[test]
fn color_256_covers_ansi_range() {
    for i in 0..16u8 {
        assert_eq!(color_256(i), ANSI_16[i as usize], "index {i} mismatch");
    }
}

#[test]
fn color_256_color_cube_boundaries() {
    // Index 16 is the first color cube entry: rgb(0,0,0)
    let c16 = color_256(16);
    assert_eq!(c16.r, 0);
    assert_eq!(c16.g, 0);
    assert_eq!(c16.b, 0);

    // Index 231 is the last color cube entry: rgb(255,255,255)
    let c231 = color_256(231);
    assert_eq!(c231.r, 255);
    assert_eq!(c231.g, 255);
    assert_eq!(c231.b, 255);
}

#[test]
fn color_256_grayscale_ramp() {
    // 232 = first grayscale: 8
    let first = color_256(232);
    assert_eq!(first.r, 8);
    assert_eq!(first.g, 8);
    assert_eq!(first.b, 8);

    // 255 = last grayscale: 238
    let last = color_256(255);
    assert_eq!(last.r, 238);
    assert_eq!(last.g, 238);
    assert_eq!(last.b, 238);
}

#[test]
fn color_256_all_indices_produce_opaque_colors() {
    for i in 0..=255u8 {
        let c = color_256(i);
        assert_eq!(c.a, 255, "index {i} should be fully opaque");
    }
}

// ---------------------------------------------------------------------------
// True color (24-bit RGB)
// ---------------------------------------------------------------------------

#[test]
fn true_color_preserves_exact_rgb() {
    let mut g = CellGrid::new(1, 1);
    let cell = Cell {
        ch: '#',
        fg: Color::rgb(0xDE, 0xAD, 0xBE),
        bg: Color::rgb(0xCA, 0xFE, 0x42),
        attrs: CellAttrs::empty(),
        wide_continuation: false,
    };
    g.set_cell(0, 0, cell);
    let got = g.get_cell(0, 0).unwrap();
    assert_eq!(got.fg, Color::rgb(0xDE, 0xAD, 0xBE));
    assert_eq!(got.bg, Color::rgb(0xCA, 0xFE, 0x42));
}

#[test]
fn true_color_independent_fg_bg() {
    let cell = Cell {
        ch: '@',
        fg: Color::rgb(1, 2, 3),
        bg: Color::rgb(254, 253, 252),
        attrs: CellAttrs::empty(),
        wide_continuation: false,
    };
    assert_ne!(cell.fg, cell.bg);
    assert_eq!(cell.fg.r, 1);
    assert_eq!(cell.bg.r, 254);
}

// ---------------------------------------------------------------------------
// Attribute flags
// ---------------------------------------------------------------------------

#[test]
fn bold_attribute_flag() {
    let mut g = CellGrid::new(1, 1);
    let cell = Cell {
        ch: 'B',
        fg: Color::WHITE,
        bg: Color::BLACK,
        attrs: CellAttrs::BOLD,
        wide_continuation: false,
    };
    g.set_cell(0, 0, cell);
    let got = g.get_cell(0, 0).unwrap();
    assert!(got.attrs.contains(CellAttrs::BOLD));
    assert!(!got.attrs.contains(CellAttrs::ITALIC));
}

#[test]
fn italic_attribute_flag() {
    let mut g = CellGrid::new(1, 1);
    let cell = Cell {
        ch: 'I',
        fg: Color::WHITE,
        bg: Color::BLACK,
        attrs: CellAttrs::ITALIC,
        wide_continuation: false,
    };
    g.set_cell(0, 0, cell);
    let got = g.get_cell(0, 0).unwrap();
    assert!(got.attrs.contains(CellAttrs::ITALIC));
    assert!(!got.attrs.contains(CellAttrs::BOLD));
}

#[test]
fn combined_attributes() {
    let attrs = CellAttrs::BOLD | CellAttrs::UNDERLINE | CellAttrs::STRIKETHROUGH;
    assert!(attrs.contains(CellAttrs::BOLD));
    assert!(attrs.contains(CellAttrs::UNDERLINE));
    assert!(attrs.contains(CellAttrs::STRIKETHROUGH));
    assert!(!attrs.contains(CellAttrs::DIM));
    assert!(!attrs.contains(CellAttrs::BLINK));
}

// ---------------------------------------------------------------------------
// Wide character (CJK) support
// ---------------------------------------------------------------------------

#[test]
fn wide_char_spans_two_columns() {
    let mut g = CellGrid::new(1, 10);
    let cell = Cell {
        ch: '\u{4E16}', // CJK character (U+4E16 "world")
        fg: Color::WHITE,
        bg: Color::BLACK,
        attrs: CellAttrs::empty(),
        wide_continuation: false,
    };
    g.set_wide_cell(0, 3, cell);

    // Primary cell at col 3
    let primary = g.get_cell(0, 3).unwrap();
    assert_eq!(primary.ch, '\u{4E16}');
    assert!(!primary.wide_continuation);

    // Continuation cell at col 4
    let cont = g.get_cell(0, 4).unwrap();
    assert!(cont.wide_continuation);
    assert_eq!(cont.ch, '\0'); // empty
}

#[test]
fn wide_char_at_last_column_is_ignored() {
    // If there is no room for the second column, do not place it
    let mut g = CellGrid::new(1, 5);
    let original = g.get_cell(0, 4).unwrap().clone();
    g.set_wide_cell(0, 4, Cell::with_char('\u{4E16}'));
    // Should not have changed because col+1 would be 5, which is out of bounds
    let after = g.get_cell(0, 4).unwrap();
    assert_eq!(*after, original);
}

// ---------------------------------------------------------------------------
// scroll_up
// ---------------------------------------------------------------------------

#[test]
fn scroll_up_shifts_rows() {
    let mut g = CellGrid::new(4, 2);
    g.set_cell(0, 0, Cell::with_char('A'));
    g.set_cell(1, 0, Cell::with_char('B'));
    g.set_cell(2, 0, Cell::with_char('C'));
    g.set_cell(3, 0, Cell::with_char('D'));

    g.scroll_up(1);

    assert_eq!(g.get_cell(0, 0).unwrap().ch, 'B');
    assert_eq!(g.get_cell(1, 0).unwrap().ch, 'C');
    assert_eq!(g.get_cell(2, 0).unwrap().ch, 'D');
    // Bottom row should be cleared
    assert!(g.get_cell(3, 0).unwrap().is_empty());
}

#[test]
fn scroll_up_by_more_than_rows_clears_all() {
    let mut g = CellGrid::new(3, 2);
    g.set_cell(0, 0, Cell::with_char('X'));
    g.set_cell(1, 0, Cell::with_char('Y'));
    g.set_cell(2, 0, Cell::with_char('Z'));

    g.scroll_up(5); // more than 3 rows

    for r in 0..3 {
        assert!(g.get_cell(r, 0).unwrap().is_empty());
    }
}

#[test]
fn scroll_up_zero_is_noop() {
    let mut g = CellGrid::new(2, 2);
    g.set_cell(0, 0, Cell::with_char('A'));
    g.clear_dirty();
    g.scroll_up(0);
    assert_eq!(g.get_cell(0, 0).unwrap().ch, 'A');
    // Dirty flags should not have been set
    assert!(!g.has_dirty_cells());
}

// ---------------------------------------------------------------------------
// resize
// ---------------------------------------------------------------------------

#[test]
fn resize_preserves_overlapping_content() {
    let mut g = CellGrid::new(3, 3);
    g.set_cell(0, 0, Cell::with_char('A'));
    g.set_cell(1, 1, Cell::with_char('B'));
    g.set_cell(2, 2, Cell::with_char('C'));

    g.resize(5, 5);

    assert_eq!(g.rows(), 5);
    assert_eq!(g.cols(), 5);
    assert_eq!(g.get_cell(0, 0).unwrap().ch, 'A');
    assert_eq!(g.get_cell(1, 1).unwrap().ch, 'B');
    assert_eq!(g.get_cell(2, 2).unwrap().ch, 'C');
    // New cells are empty
    assert!(g.get_cell(3, 3).unwrap().is_empty());
    assert!(g.get_cell(4, 4).unwrap().is_empty());
}

#[test]
fn resize_shrink_clips_content() {
    let mut g = CellGrid::new(4, 4);
    g.set_cell(0, 0, Cell::with_char('A'));
    g.set_cell(3, 3, Cell::with_char('D'));

    g.resize(2, 2);

    assert_eq!(g.rows(), 2);
    assert_eq!(g.cols(), 2);
    assert_eq!(g.get_cell(0, 0).unwrap().ch, 'A');
    // (3,3) is now out of bounds
    assert!(g.get_cell(3, 3).is_none());
}

#[test]
fn resize_same_dimensions_is_noop() {
    let mut g = CellGrid::new(5, 5);
    g.set_cell(2, 2, Cell::with_char('M'));
    g.clear_dirty();

    g.resize(5, 5);

    assert_eq!(g.get_cell(2, 2).unwrap().ch, 'M');
    // No dirty flags because dimensions did not change
    assert!(!g.has_dirty_cells());
}

// ---------------------------------------------------------------------------
// Empty cells should not produce glyph instances
// ---------------------------------------------------------------------------

#[test]
fn empty_cell_reports_is_empty() {
    let empty = Cell::default();
    assert!(empty.is_empty());

    let space = Cell { ch: ' ', ..Cell::default() };
    assert!(space.is_empty());

    let non_empty = Cell::with_char('X');
    assert!(!non_empty.is_empty());
}

// ---------------------------------------------------------------------------
// Damage tracking
// ---------------------------------------------------------------------------

#[test]
fn damage_tracking_marks_changed_cells() {
    let mut g = CellGrid::new(3, 3);
    // All cells start dirty after creation
    assert!(g.has_dirty_cells());

    // Clear dirty flags (simulates renderer having processed them)
    g.clear_dirty();
    assert!(!g.has_dirty_cells());

    // Modify one cell
    g.set_cell(1, 1, Cell::with_char('X'));

    // Only the modified cell should be dirty
    let dirty = g.dirty_flags();
    let idx = 1 * 3 + 1; // row 1, col 1
    assert!(dirty[idx], "modified cell should be dirty");

    // All other cells should be clean
    for (i, &d) in dirty.iter().enumerate() {
        if i != idx {
            assert!(!d, "cell at flat index {i} should not be dirty");
        }
    }
}

#[test]
fn damage_tracking_clear_dirty_resets_all() {
    let mut g = CellGrid::new(2, 2);
    g.set_cell(0, 0, Cell::with_char('A'));
    g.set_cell(1, 1, Cell::with_char('B'));
    assert!(g.has_dirty_cells());

    g.clear_dirty();
    assert!(!g.has_dirty_cells());
    for &d in g.dirty_flags() {
        assert!(!d);
    }
}

// ---------------------------------------------------------------------------
// clear()
// ---------------------------------------------------------------------------

#[test]
fn clear_resets_all_cells() {
    let mut g = CellGrid::new(3, 3);
    g.set_cell(0, 0, Cell::with_char('A'));
    g.set_cell(2, 2, Cell::with_char('Z'));
    g.clear_dirty();

    g.clear();

    for cell in g.cells() {
        assert!(cell.is_empty());
    }
    // All cells should be dirty after clear
    assert!(g.has_dirty_cells());
}

// ---------------------------------------------------------------------------
// ElementContent::Grid integration
// ---------------------------------------------------------------------------

#[test]
fn element_content_grid_variant_equality() {
    use unshit_core::element::ElementContent;

    let g1 = CellGrid::new(2, 2);
    let g2 = CellGrid::new(2, 2);
    assert_eq!(ElementContent::Grid(g1.clone()), ElementContent::Grid(g2));
    assert_ne!(ElementContent::Grid(g1), ElementContent::None);
}
