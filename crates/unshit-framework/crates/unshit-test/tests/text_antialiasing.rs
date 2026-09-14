//! Exercise actual rasterization, atlas upload, and GPU blending. Neutral
//! terminal text must not acquire orange/blue edges under the default policy.

use unshit_core::cell_grid::{Cell, CellGrid};
use unshit_core::element::{ElementDef, ElementTree, Tag};
use unshit_core::style::types::Color;
use unshit_test::TestHarness;

#[test]
fn default_terminal_antialiasing_preserves_neutral_edges() {
    for (background, foreground) in [(12, 204), (240, 32)] {
        let css = format!(
            ".terminal {{ width: 100%; height: 100%; font-size: 20px; \
             background: rgb({background}, {background}, {background}); }}"
        );
        let mut harness = TestHarness::new(
            &css,
            move || {
                let mut grid = CellGrid::new(1, 25);
                grid.set_cursor_visible(false);
                for (col, ch) in "Test of words, testing".chars().enumerate() {
                    let mut cell = Cell::with_char(ch);
                    cell.fg = Color::rgb(foreground, foreground, foreground);
                    grid.set_cell(0, col, cell);
                }
                ElementTree {
                    root: ElementDef::new(Tag::Div).with_class("terminal").with_grid(grid),
                }
            },
            400.0,
            50.0,
        )
        .with_gpu();
        harness.step();
        let pixels = harness.render();
        let mut ink = 0;
        let mut antialiased = 0;
        for pixel in pixels.as_chunks::<4>().0 {
            assert_eq!(pixel[0], pixel[1], "neutral text gained a red/green fringe");
            assert_eq!(pixel[1], pixel[2], "neutral text gained a green/blue fringe");
            if pixel[0].abs_diff(background) > 8 {
                ink += 1;
                if pixel[0].abs_diff(foreground) > 8 {
                    antialiased += 1;
                }
            }
        }
        assert!(ink > 100, "a blank frame must not pass the neutrality check");
        assert!(antialiased > 50, "glyph edges must be antialiased, not binary masks");
    }
}
