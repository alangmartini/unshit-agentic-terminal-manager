use unshit_core::cell_grid::{Cell, CellGrid};
use unshit_core::element::{ElementDef, ElementTree, Tag};
use unshit_core::style::types::Color;
use unshit_test::TestHarness;

static GRID_METRICS: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn quadrant_and_full_blocks_join_without_font_bearing_gaps() {
    let _metrics = GRID_METRICS.lock().unwrap();
    const WIDTH: usize = 240;
    let mut harness = TestHarness::new(
        ".terminal { width: 100%; height: 100%; background: #000; font-size: 20px; line-height: 1.15; }",
        || {
            let mut grid = CellGrid::new(2, 4);
            grid.set_cursor_visible(false);
            // Missing quarters are at the four *outside* corners. The
            // middle horizontal band must be solid across both cell rows.
            for (row, text) in ["▟██▙", "▜██▛"].iter().enumerate() {
                for (col, ch) in text.chars().enumerate() {
                    let mut cell = Cell::with_char(ch);
                    cell.fg = Color::WHITE;
                    grid.set_cell(row, col, cell);
                }
            }
            ElementTree { root: ElementDef::new(Tag::Div).with_class("terminal").with_grid(grid) }
        },
        WIDTH as f32,
        100.0,
    )
    .with_gpu();
    for scale in [1.0, 1.25, 1.5, 2.0] {
        harness.set_scale_factor(scale);
        harness.step();
        let pixels = harness.render();
        let w = CellGrid::global_cell_w() as usize;
        let h = CellGrid::global_cell_h() as usize;
        for y in h.div_ceil(2) + 1..h + h / 2 - 1 {
            for x in 1..4 * w - 1 {
                assert!(pixels[(y * WIDTH + x) * 4] > 240, "gap at {x},{y}, scale {scale}");
            }
        }
        assert!(pixels[((h / 4) * WIDTH + w / 4) * 4] < 8, "empty outer quadrant filled");
    }
}

#[test]
fn rounded_box_corners_join_horizontal_and_vertical_rules() {
    let _metrics = GRID_METRICS.lock().unwrap();
    const WIDTH: usize = 320;
    let mut harness = TestHarness::new(
        ".terminal { width: 100%; height: 100%; background: #000; font-size: 20px; line-height: 1.15; }",
        || {
            let mut grid = CellGrid::new(3, 6);
            grid.set_cursor_visible(false);
            for (row, text) in ["╭────╮", "│    │", "╰────╯"].iter().enumerate() {
                for (col, ch) in text.chars().enumerate() {
                    let mut cell = Cell::with_char(ch);
                    cell.fg = Color::WHITE;
                    grid.set_cell(row, col, cell);
                }
            }
            ElementTree { root: ElementDef::new(Tag::Div).with_class("terminal").with_grid(grid) }
        },
        WIDTH as f32,
        180.0,
    ).with_gpu();
    for scale in [1.0, 1.25, 1.5, 2.0] {
        harness.set_scale_factor(scale);
        harness.step();
        let pixels = harness.render();
        let w = CellGrid::global_cell_w() as usize;
        let h = CellGrid::global_cell_h() as usize;
        let lit = |x: usize, y: usize| pixels[(y * WIDTH + x) * 4] > 32;
        for x in w..5 * w {
            assert!((0..h).any(|y| lit(x, y)), "top gap at {x}, scale {scale}");
            assert!((2 * h..3 * h).any(|y| lit(x, y)), "bottom gap at {x}, scale {scale}");
        }
        for y in h..2 * h {
            assert!((0..w).any(|x| lit(x, y)), "left gap at {y}, scale {scale}");
            assert!((5 * w..6 * w).any(|x| lit(x, y)), "right gap at {y}, scale {scale}");
        }
        // Follow the complete contour as an eight-connected pixel component.
        let mut visited = vec![false; WIDTH * 180];
        let start = (w..5 * w).find_map(|x| (0..h).find(|&y| lit(x, y)).map(|y| (x, y))).unwrap();
        let mut pending = vec![start];
        while let Some((x, y)) = pending.pop() {
            let index = y * WIDTH + x;
            if visited[index] || !lit(x, y) {
                continue;
            }
            visited[index] = true;
            for ny in y.saturating_sub(1)..=(y + 1).min(3 * h - 1) {
                for nx in x.saturating_sub(1)..=(x + 1).min(6 * w - 1) {
                    if !visited[ny * WIDTH + nx] {
                        pending.push((nx, ny));
                    }
                }
            }
        }
        for y in 0..3 * h {
            for x in 0..6 * w {
                assert!(
                    !lit(x, y) || visited[y * WIDTH + x],
                    "disconnected ink at {x},{y}, scale {scale}"
                );
            }
        }
    }
}

#[test]
fn rounded_box_overscan_offset_preserves_visible_pixels() {
    let _metrics = GRID_METRICS.lock().unwrap();
    let mut grid = CellGrid::new(4, 6);
    grid.set_cursor_visible(false);
    for (row, text) in ["╭────╮", "│    │", "╰────╯"].iter().enumerate()
    {
        for (col, ch) in text.chars().enumerate() {
            let mut cell = Cell::with_char(ch);
            cell.fg = Color::WHITE;
            grid.set_cell(row, col, cell);
        }
    }
    let tree = |g: CellGrid| ElementTree {
        root: ElementDef::new(Tag::Div).with_class("terminal").with_grid(g),
    };
    let mut harness = TestHarness::new(
        ".terminal { width: 100%; height: 100%; background: #000; font-size: 20px; line-height: 1.15; }",
        || tree(grid.clone()), 320.0, 220.0,
    ).with_gpu();
    for scale in [1.0, 1.25, 1.5, 2.0] {
        harness.set_scale_factor(scale);
        let root = harness.root();
        harness.arena_mut().get_mut(root).unwrap().content =
            unshit_core::element::ElementContent::Grid(grid.clone());
        harness
            .arena_mut()
            .get_mut(root)
            .unwrap()
            .dirty
            .insert(unshit_core::dirty::DirtyFlags::PAINT);
        harness.step();
        let reference = harness.render();
        let h = CellGrid::global_cell_h();
        let mut scrolled = grid.clone();
        scrolled.insert_overscan_row_top(None, u64::MAX);
        scrolled.set_overscan_rows(1);
        scrolled.set_render_offset_y(-h);
        harness.arena_mut().get_mut(root).unwrap().content =
            unshit_core::element::ElementContent::Grid(scrolled);
        harness
            .arena_mut()
            .get_mut(root)
            .unwrap()
            .dirty
            .insert(unshit_core::dirty::DirtyFlags::PAINT);
        harness.step();
        let actual = harness.render();
        let differences: Vec<_> = actual
            .iter()
            .zip(&reference)
            .enumerate()
            .filter(|(_, (a, b))| a.abs_diff(**b) > 1)
            .map(|(i, (a, b))| (i / 4 % 320, i / 4 / 320, *a, *b))
            .take(10)
            .collect();
        assert!(
            differences.is_empty(),
            "overscan changed box pixels at scale {scale}: {differences:?}"
        );
    }
}
