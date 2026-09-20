use unshit_core::cell_grid::{Cell, CellGrid};
use unshit_core::element::{ElementDef, ElementTree, Tag};
use unshit_test::TestHarness;

#[test]
fn repeated_terminal_letters_have_identical_pixels_across_rows_and_columns() {
    const WIDTH: usize = 320;
    let css = ".terminal { width: 100%; height: 100%; background: #000; \
             font-size: 15px; line-height: 1.25; }";
    let mut harness = TestHarness::new(
        css,
        || {
            let mut grid = CellGrid::new(3, 12);
            grid.set_cursor_visible(false);
            for row in 0..3 {
                for col in 0..12 {
                    grid.set_cell(row, col, Cell::with_char('M'));
                }
            }
            ElementTree { root: ElementDef::new(Tag::Div).with_class("terminal").with_grid(grid) }
        },
        WIDTH as f32,
        150.0,
    )
    .with_gpu();
    // Transition the existing renderer through fractional DPI values and
    // back, exercising style rescaling and cache invalidation as well as ink.
    for scale in [1.0, 1.25, 1.5, 1.75, 2.0, 1.0] {
        harness.set_scale_factor(scale);
        harness.step();
        let pixels = harness.render();
        let cell_w = CellGrid::global_cell_w();
        let cell_h = CellGrid::global_cell_h();
        assert_eq!(cell_w.fract(), 0.0, "published width must match device pixels");
        assert_eq!(cell_h.fract(), 0.0, "published height must match device pixels");
        let (w, h) = (cell_w as usize, cell_h as usize);
        assert_eq!(CellGrid::take_pending_resize(), Some(((WIDTH / w) as u16, (150 / h) as u16)));
        let tile = |row: usize, col: usize| -> Vec<u8> {
            (row * h..(row + 1) * h)
                .flat_map(|y| {
                    let start = (y * WIDTH + col * w) * 4;
                    pixels[start..start + w * 4].iter().copied()
                })
                .collect()
        };
        let reference = tile(0, 1);
        assert!(reference.as_chunks::<4>().0.iter().filter(|pixel| pixel[0] > 32).count() > 20);
        for row in 0..3 {
            for col in 2..11 {
                assert_eq!(
                    tile(row, col),
                    reference,
                    "scale {scale}, cell {row},{col} changed ink"
                );
            }
        }
    }
}
