//! Tests for the CellGrid rendering primitive.

use unshit_core::cell_grid::{color_256, Cell, CellAttrs, CellGrid, LineDamage, ANSI_16};
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

#[test]
fn scroll_up_marks_every_row_fully_damaged_so_line_cache_reemits() {
    // Regression: the tier 3 line quad cache is keyed by
    // (NodeId, row_index, content_hash). When scroll_up preserved the
    // "clean" state of shifted rows, the renderer saw those rows as
    // clean, probed the cache with the NEW content hash at the OLD row
    // index, missed, and then skipped emission because the row was
    // clean. Result: after a terminal scroll the viewport rendered
    // empty even though cells held real content (visible by scrolling
    // back into the composed view that rebuilds via set_cell).
    let mut g = CellGrid::new(4, 3);
    for r in 0..4 {
        let ch = (b'A' + r as u8) as char;
        for c in 0..3 {
            g.set_cell(r, c, Cell::with_char(ch));
        }
    }
    g.clear_dirty();
    assert!(
        g.line_damage().iter().all(|ld| ld.is_clean()),
        "precondition: every row must be clean after clear_dirty",
    );

    g.scroll_up(1);

    for (row, ld) in g.line_damage().iter().enumerate() {
        assert!(
            !ld.is_clean(),
            "row {row} stayed clean after scroll_up; renderer will skip re-emit and the line quad cache will serve stale quads",
        );
        assert_eq!(ld.first_dirty_col, 0, "row {row} first_dirty_col");
        assert_eq!(ld.last_dirty_col, 2, "row {row} last_dirty_col");
    }
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

// ---------------------------------------------------------------------------
// Line damage tracking (Tier 2)
// ---------------------------------------------------------------------------

#[test]
fn line_damage_default_is_clean() {
    let ld = LineDamage::default();
    assert!(ld.is_clean());
    assert_eq!(ld.first_dirty_col, u16::MAX);
    assert_eq!(ld.seqno, 0);
}

#[test]
fn line_damage_mark_col_expands_range_and_bumps_seqno() {
    let mut ld = LineDamage::default();
    ld.mark_col(5);
    assert!(!ld.is_clean());
    assert_eq!(ld.first_dirty_col, 5);
    assert_eq!(ld.last_dirty_col, 5);
    assert_eq!(ld.seqno, 1);

    ld.mark_col(10);
    assert_eq!(ld.first_dirty_col, 5);
    assert_eq!(ld.last_dirty_col, 10);
    assert_eq!(ld.seqno, 2);

    ld.mark_col(2);
    assert_eq!(ld.first_dirty_col, 2);
    assert_eq!(ld.last_dirty_col, 10);
    assert_eq!(ld.seqno, 3);
}

#[test]
fn line_damage_clear_cols_preserves_seqno() {
    let mut ld = LineDamage::default();
    ld.mark_col(3);
    ld.mark_col(9);
    let seqno = ld.seqno;
    ld.clear_cols();
    assert!(ld.is_clean());
    assert_eq!(ld.seqno, seqno);
}

#[test]
fn new_grid_marks_every_line_fully_damaged() {
    let g = CellGrid::new(3, 5);
    let ld = g.line_damage();
    assert_eq!(ld.len(), 3);
    for row in ld {
        assert!(!row.is_clean());
        assert_eq!(row.first_dirty_col, 0);
        assert_eq!(row.last_dirty_col, 4);
    }
}

#[test]
fn set_cell_dirties_only_its_line() {
    let mut g = CellGrid::new(3, 5);
    g.clear_dirty();
    let seq_before_0 = g.line_damage()[0].seqno;
    let seq_before_1 = g.line_damage()[1].seqno;
    let seq_before_2 = g.line_damage()[2].seqno;

    g.set_cell(1, 3, Cell::with_char('X'));

    // Row 1 becomes dirty with col 3..=3 and its seqno bumps.
    assert!(!g.line_damage()[1].is_clean());
    assert_eq!(g.line_damage()[1].first_dirty_col, 3);
    assert_eq!(g.line_damage()[1].last_dirty_col, 3);
    assert_eq!(g.line_damage()[1].seqno, seq_before_1 + 1);

    // Other rows stay clean with unchanged seqno.
    assert!(g.line_damage()[0].is_clean());
    assert!(g.line_damage()[2].is_clean());
    assert_eq!(g.line_damage()[0].seqno, seq_before_0);
    assert_eq!(g.line_damage()[2].seqno, seq_before_2);
}

#[test]
fn clear_dirty_resets_cols_but_keeps_seqno_monotonic() {
    let mut g = CellGrid::new(2, 4);
    g.clear_dirty();
    g.set_cell(0, 1, Cell::with_char('A'));
    let bumped = g.line_damage()[0].seqno;
    assert!(bumped > 0);

    g.clear_dirty();
    assert!(g.line_damage()[0].is_clean());
    // Seqno must NOT be reset — renderers compare it to a checkpoint.
    assert_eq!(g.line_damage()[0].seqno, bumped);
}

#[test]
fn vte_write_bumps_seqno_for_touched_row_only() {
    // Simulates the monotonic seqno contract: each cell write increments the
    // row's seqno even if the cell value is the same.
    let mut g = CellGrid::new(2, 3);
    g.clear_dirty();
    let s0_initial = g.line_damage()[0].seqno;
    let s1_initial = g.line_damage()[1].seqno;

    // Write the same char 3x to row 0.
    g.set_cell(0, 0, Cell::with_char('A'));
    g.set_cell(0, 0, Cell::with_char('A'));
    g.set_cell(0, 0, Cell::with_char('A'));

    assert_eq!(g.line_damage()[0].seqno, s0_initial + 3);
    // Row 1 untouched.
    assert_eq!(g.line_damage()[1].seqno, s1_initial);
}

#[test]
fn renderer_can_skip_line_using_seqno_checkpoint() {
    // The checkpoint pattern: renderer stores last-seen seqno per row and
    // skips rows whose current seqno equals the checkpoint.
    let mut g = CellGrid::new(2, 3);
    g.set_cell(0, 0, Cell::with_char('A'));
    g.set_cell(1, 0, Cell::with_char('B'));

    // Renderer processes and checkpoints.
    let checkpoints: Vec<u64> = g.line_damage().iter().map(|ld| ld.seqno).collect();
    g.clear_dirty();

    // No writes happened. Renderer should see seqnos match.
    for (row, &expected) in checkpoints.iter().enumerate() {
        assert_eq!(g.line_damage()[row].seqno, expected);
    }

    // Now write to row 0 only.
    g.set_cell(0, 1, Cell::with_char('C'));
    assert_ne!(g.line_damage()[0].seqno, checkpoints[0]);
    assert_eq!(g.line_damage()[1].seqno, checkpoints[1]);
}

#[test]
fn scroll_up_marks_every_row_fully_damaged_and_bumps_seqno() {
    // scroll_up must mark every row fully damaged (not shift the per-row
    // damage along with the cells). The retained line quad cache is keyed
    // by (NodeId, row_index, content_hash); a shifted row carries new
    // content at the same row index, so its previously stored cache entry
    // is stale. Per-row seqnos must also bump so subscribers relying on
    // the generation counter re-paint.
    let mut g = CellGrid::new(3, 4);
    g.clear_dirty();

    g.set_cell(1, 2, Cell::with_char('M'));
    let ld_mid_before = g.line_damage()[1];
    assert!(!ld_mid_before.is_clean());

    let seqs_before: Vec<u64> = g.line_damage().iter().map(|ld| ld.seqno).collect();

    g.scroll_up(1);

    for (row, ld) in g.line_damage().iter().enumerate() {
        assert!(!ld.is_clean(), "row {row} must be damaged after scroll_up");
        assert_eq!(ld.first_dirty_col, 0, "row {row} first_dirty_col");
        assert_eq!(ld.last_dirty_col, 3, "row {row} last_dirty_col");
        assert!(
            ld.seqno > seqs_before[row],
            "row {row} seqno must advance so subscribers re-paint",
        );
    }
}

#[test]
fn scroll_up_zero_does_not_touch_damage() {
    let mut g = CellGrid::new(2, 2);
    g.set_cell(0, 0, Cell::with_char('A'));
    g.clear_dirty();
    let pre: Vec<LineDamage> = g.line_damage().to_vec();
    g.scroll_up(0);
    assert_eq!(g.line_damage(), pre.as_slice());
}

#[test]
fn resize_rebuilds_line_damage_and_marks_all() {
    let mut g = CellGrid::new(2, 3);
    g.clear_dirty();
    assert!(g.line_damage().iter().all(|ld| ld.is_clean()));

    g.resize(4, 5);
    assert_eq!(g.line_damage().len(), 4);
    for ld in g.line_damage() {
        assert!(!ld.is_clean());
        assert_eq!(ld.first_dirty_col, 0);
        assert_eq!(ld.last_dirty_col, 4);
    }
}

#[test]
fn clear_marks_every_line_damaged() {
    let mut g = CellGrid::new(2, 3);
    g.clear_dirty();
    g.clear();
    for ld in g.line_damage() {
        assert!(!ld.is_clean());
        assert_eq!(ld.first_dirty_col, 0);
        assert_eq!(ld.last_dirty_col, 2);
    }
}

#[test]
fn set_wide_cell_marks_both_columns() {
    let mut g = CellGrid::new(1, 4);
    g.clear_dirty();
    g.set_wide_cell(0, 1, Cell::with_char('漢'));
    let ld = g.line_damage()[0];
    assert!(!ld.is_clean());
    assert_eq!(ld.first_dirty_col, 1);
    assert_eq!(ld.last_dirty_col, 2);
}

#[test]
fn line_damage_for_out_of_bounds_returns_none() {
    let g = CellGrid::new(2, 2);
    assert!(g.line_damage_for(0).is_some());
    assert!(g.line_damage_for(1).is_some());
    assert!(g.line_damage_for(2).is_none());
}

// ---------------------------------------------------------------------------
// Style-run grouping (Tier 2 run-length batching)
// ---------------------------------------------------------------------------

fn styled(ch: char, fg: Color, bg: Color, attrs: CellAttrs) -> Cell {
    Cell { ch, fg, bg, attrs, wide_continuation: false }
}

#[test]
fn compute_style_runs_merges_adjacent_same_style() {
    let mut g = CellGrid::new(1, 4);
    let fg = Color::rgb(255, 255, 255);
    let bg = Color::rgb(0, 0, 0);
    for col in 0..4 {
        g.set_cell(0, col, styled('x', fg, bg, CellAttrs::empty()));
    }
    let runs = g.compute_style_runs(0);
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].start_col, 0);
    assert_eq!(runs[0].end_col, 4);
    assert_eq!(runs[0].style.fg, fg);
    assert_eq!(runs[0].style.bg, bg);
}

#[test]
fn compute_style_runs_splits_on_bg_change() {
    let mut g = CellGrid::new(1, 4);
    let fg = Color::rgb(255, 255, 255);
    let bg_a = Color::rgb(10, 10, 10);
    let bg_b = Color::rgb(20, 20, 20);
    g.set_cell(0, 0, styled('a', fg, bg_a, CellAttrs::empty()));
    g.set_cell(0, 1, styled('a', fg, bg_a, CellAttrs::empty()));
    g.set_cell(0, 2, styled('b', fg, bg_b, CellAttrs::empty()));
    g.set_cell(0, 3, styled('b', fg, bg_b, CellAttrs::empty()));
    let runs = g.compute_style_runs(0);
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].end_col, 2);
    assert_eq!(runs[1].start_col, 2);
    assert_eq!(runs[1].end_col, 4);
}

#[test]
fn compute_style_runs_splits_on_fg_change() {
    let mut g = CellGrid::new(1, 3);
    let bg = Color::rgb(0, 0, 0);
    let fg_a = Color::rgb(255, 0, 0);
    let fg_b = Color::rgb(0, 255, 0);
    g.set_cell(0, 0, styled('a', fg_a, bg, CellAttrs::empty()));
    g.set_cell(0, 1, styled('b', fg_b, bg, CellAttrs::empty()));
    g.set_cell(0, 2, styled('c', fg_b, bg, CellAttrs::empty()));
    let runs = g.compute_style_runs(0);
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].col_count(), 1);
    assert_eq!(runs[1].col_count(), 2);
}

#[test]
fn compute_style_runs_splits_on_attrs_change() {
    let mut g = CellGrid::new(1, 3);
    let fg = Color::rgb(255, 255, 255);
    let bg = Color::rgb(0, 0, 0);
    g.set_cell(0, 0, styled('a', fg, bg, CellAttrs::empty()));
    g.set_cell(0, 1, styled('b', fg, bg, CellAttrs::BOLD));
    g.set_cell(0, 2, styled('c', fg, bg, CellAttrs::BOLD));
    let runs = g.compute_style_runs(0);
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].style.attrs, CellAttrs::empty());
    assert_eq!(runs[1].style.attrs, CellAttrs::BOLD);
}

#[test]
fn compute_style_runs_inverse_normalizes_colors() {
    let mut g = CellGrid::new(1, 2);
    let fg = Color::rgb(255, 0, 0);
    let bg = Color::rgb(0, 0, 255);
    // Cell 0 is plain fg/bg, cell 1 is inverse of the same colors -> after
    // normalization these are NOT the same run (fg and bg swap).
    g.set_cell(0, 0, styled('a', fg, bg, CellAttrs::empty()));
    g.set_cell(0, 1, styled('b', fg, bg, CellAttrs::INVERSE));
    let runs = g.compute_style_runs(0);
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].style.fg, fg);
    assert_eq!(runs[0].style.bg, bg);
    assert_eq!(runs[1].style.fg, bg);
    assert_eq!(runs[1].style.bg, fg);
}

#[test]
fn compute_style_runs_never_cross_rows() {
    let mut g = CellGrid::new(2, 3);
    let fg = Color::rgb(255, 255, 255);
    let bg = Color::rgb(0, 0, 0);
    for r in 0..2 {
        for c in 0..3 {
            g.set_cell(r, c, styled('x', fg, bg, CellAttrs::empty()));
        }
    }
    let row0 = g.compute_style_runs(0);
    let row1 = g.compute_style_runs(1);
    assert_eq!(row0.len(), 1);
    assert_eq!(row1.len(), 1);
    assert_eq!(row0[0].end_col, 3);
    assert_eq!(row1[0].end_col, 3);
}

#[test]
fn compute_style_runs_in_range_honors_bounds() {
    let mut g = CellGrid::new(1, 5);
    let fg = Color::rgb(255, 255, 255);
    let bg = Color::rgb(0, 0, 0);
    for c in 0..5 {
        g.set_cell(0, c, styled('x', fg, bg, CellAttrs::empty()));
    }
    let runs = g.compute_style_runs_in_range(0, 1, 4);
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].start_col, 1);
    assert_eq!(runs[0].end_col, 4);
}

#[test]
fn compute_style_runs_empty_on_bad_inputs() {
    let g = CellGrid::new(2, 3);
    assert!(g.compute_style_runs(10).is_empty());
    assert!(g.compute_style_runs_in_range(0, 3, 3).is_empty());
    assert!(g.compute_style_runs_in_range(0, 5, 10).is_empty());
}

#[test]
fn compute_style_runs_wide_continuation_shares_style() {
    // A wide primary cell at col 0 and its continuation at col 1 share the
    // same (fg, bg, attrs) so they merge into a single run whose col range
    // covers both halves.
    let mut g = CellGrid::new(1, 3);
    let fg = Color::rgb(255, 255, 255);
    let bg = Color::rgb(0, 0, 0);
    // Use set_wide_cell to produce a primary + continuation pair.
    g.set_wide_cell(0, 0, styled('漢', fg, bg, CellAttrs::empty()));
    // Fill col 2 with the same style so the whole row is one run.
    g.set_cell(0, 2, styled('a', fg, bg, CellAttrs::empty()));
    let runs = g.compute_style_runs(0);
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].start_col, 0);
    assert_eq!(runs[0].end_col, 3);
}
