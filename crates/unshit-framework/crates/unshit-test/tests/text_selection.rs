use cosmic_text::FontSystem;
use unshit_core::element::*;
use unshit_core::layout;
use unshit_test::TestHarness;

fn make_text_tree() -> ElementTree {
    ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(ElementDef::new(Tag::Span).with_class("label").with_text("Hello, world!")),
    }
}

#[test]
fn click_on_text_creates_collapsed_selection() {
    let css = r#"
        .root { width: 100%; height: 100%; }
        .label { font-size: 16px; line-height: 1.2; padding: 4px; }
    "#;

    let mut h = TestHarness::new(css, make_text_tree, 800.0, 600.0);
    h.step();

    h.select_text(20.0, 10.0, 20.0, 10.0);

    let sel = h.text_selection();
    assert!(sel.is_some(), "Should have a selection after clicking text");
    let sel = sel.unwrap();
    assert!(sel.is_collapsed(), "Single click should create collapsed selection");
}

#[test]
fn drag_on_text_creates_range_selection() {
    let css = r#"
        .root { width: 100%; height: 100%; }
        .label { font-size: 16px; line-height: 1.2; padding: 4px; }
    "#;

    let mut h = TestHarness::new(css, make_text_tree, 800.0, 600.0);
    h.step();

    h.select_text(10.0, 10.0, 80.0, 10.0);

    let sel = h.text_selection();
    assert!(sel.is_some(), "Should have a selection after dragging");
    let sel = sel.unwrap();
    assert!(!sel.is_collapsed(), "Drag should create non-collapsed selection");
    let (start, end) = sel.ordered_range();
    assert!(end > start, "Selection should span multiple bytes");
}

fn make_multi_line_tree() -> ElementTree {
    ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(
                ElementDef::new(Tag::Span).with_class("line1").with_text("First line of text"),
            )
            .with_child(
                ElementDef::new(Tag::Span).with_class("line2").with_text("Second line of text"),
            ),
    }
}

fn make_button_tree() -> ElementTree {
    ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(ElementDef::new(Tag::Span).with_class("before").with_text("Before button"))
            .with_child(ElementDef::new(Tag::Button).with_class("btn").with_text("Click me"))
            .with_child(ElementDef::new(Tag::Span).with_class("after").with_text("After button")),
    }
}

#[test]
fn multi_line_selection_spans_elements() {
    let css = r#"
        .root { display: flex; flex-direction: column; width: 100%; height: 100%; }
        .line1, .line2 { font-size: 16px; line-height: 1.2; padding: 4px; }
    "#;
    let mut h = TestHarness::new(css, make_multi_line_tree, 800.0, 600.0);
    h.step();

    let line1 = h.query(".line1").unwrap();
    let line2 = h.query(".line2").unwrap();

    // Drag from middle of line1 to middle of line2
    let start_x = line1.layout_rect.x + 30.0;
    let start_y = line1.layout_rect.y + line1.layout_rect.height / 2.0;
    let end_x = line2.layout_rect.x + 30.0;
    let end_y = line2.layout_rect.y + line2.layout_rect.height / 2.0;

    h.select_text(start_x, start_y, end_x, end_y);

    let sel = h.text_selection().expect("Should have selection");
    assert!(!sel.is_collapsed(), "Selection should not be collapsed");
    // The selection should span two different elements
    assert_ne!(sel.anchor_element, sel.focus_element, "Selection should span different elements");
}

#[test]
fn selection_past_button() {
    let css = r#"
        .root { display: flex; flex-direction: column; width: 100%; height: 100%; }
        .before, .after { font-size: 16px; line-height: 1.2; padding: 4px; }
        .btn { font-size: 16px; padding: 4px 8px; cursor: pointer; }
    "#;
    let mut h = TestHarness::new(css, make_button_tree, 800.0, 600.0);
    h.step();

    let before = h.query(".before").unwrap();
    let after = h.query(".after").unwrap();

    // Drag from "Before button" text to "After button" text
    let start_x = before.layout_rect.x + 20.0;
    let start_y = before.layout_rect.y + before.layout_rect.height / 2.0;
    let end_x = after.layout_rect.x + 20.0;
    let end_y = after.layout_rect.y + after.layout_rect.height / 2.0;

    h.select_text(start_x, start_y, end_x, end_y);

    let sel = h.text_selection().expect("Should have selection after dragging past button");
    assert!(!sel.is_collapsed());
    // Focus should be in the "after" span, not stuck at the button
    assert_eq!(sel.focus_element, after.node_id, "Focus should reach the span after the button");
}

#[test]
fn single_element_selection_still_works() {
    let css = r#"
        .root { width: 100%; height: 100%; }
        .label { font-size: 16px; line-height: 1.2; padding: 4px; }
    "#;
    let mut h = TestHarness::new(css, make_text_tree, 800.0, 600.0);
    h.step();

    h.select_text(10.0, 10.0, 80.0, 10.0);

    let sel = h.text_selection().expect("Should have selection");
    assert!(!sel.is_collapsed());
    assert_eq!(
        sel.anchor_element, sel.focus_element,
        "Single-line drag should stay in same element"
    );
}

#[test]
fn click_on_non_text_clears_selection() {
    let css = r#"
        .root { width: 100%; height: 100%; }
        .label { font-size: 16px; line-height: 1.2; padding: 4px; width: 200px; height: 30px; }
    "#;

    let mut h = TestHarness::new(css, make_text_tree, 800.0, 600.0);
    h.step();

    // First select some text
    h.select_text(10.0, 10.0, 80.0, 10.0);
    assert!(h.text_selection().is_some());

    // Click on empty area far from text
    h.mouse_down(500.0, 500.0);
    h.step();
    h.mouse_up(500.0, 500.0);
    h.step();

    assert!(h.text_selection().is_none(), "Clicking non-text should clear selection");
}

// Regression tests: selection highlight should be seamless per-line rectangles,
// not individual per-glyph quads with visible gaps between them.

#[test]
fn selection_highlight_is_single_rect_per_line() {
    // Bug: text_glyph_ranges returned one rect per glyph cluster, causing
    // visible gaps between adjacent quads due to sub-pixel rounding/kerning.
    // Fix: text_line_ranges merges all selected glyphs on the same line
    // into a single contiguous rectangle.
    let mut font_system = FontSystem::new();
    let text = "Hello, world!";

    let ranges = layout::text_line_ranges(
        text,
        16.0,
        1.2,
        0.0,
        Some(400.0),
        0,
        text.len(),
        &mut font_system,
    );

    assert_eq!(
        ranges.len(),
        1,
        "Single-line text should produce exactly 1 LineSelectionRange, got {}",
        ranges.len()
    );

    let r = &ranges[0];
    assert!(r.width > 0.0, "Selection width should be positive");
    assert!(r.height > 0.0, "Selection height should be positive");

    // Width should approximately match the full text width (not just one glyph)
    let (full_w, _) = layout::measure_text(text, 16.0, 1.2, 0.0, Some(400.0), &mut font_system);
    assert!(
        r.width >= full_w * 0.8,
        "Merged rect width ({}) should be close to full text width ({})",
        r.width,
        full_w
    );
}

#[test]
fn partial_selection_is_contiguous() {
    // Selecting a sub-range (bytes 3..8) should still yield a single
    // contiguous rectangle, not multiple per-glyph rects.
    let mut font_system = FontSystem::new();
    let text = "Hello, world!";

    let ranges =
        layout::text_line_ranges(text, 16.0, 1.2, 0.0, Some(400.0), 3, 8, &mut font_system);

    assert_eq!(
        ranges.len(),
        1,
        "Partial single-line selection should produce 1 rect, got {}",
        ranges.len()
    );

    let r = &ranges[0];
    assert!(r.x > 0.0, "Partial selection should not start at x=0");
    assert!(r.width > 0.0, "Partial selection width should be positive");
}

#[test]
fn empty_selection_returns_no_ranges() {
    let mut font_system = FontSystem::new();
    let text = "Hello";

    // Collapsed selection (start == end)
    let ranges =
        layout::text_line_ranges(text, 16.0, 1.2, 0.0, Some(400.0), 3, 3, &mut font_system);
    assert!(ranges.is_empty(), "Collapsed selection should return no ranges");

    // Empty text
    let ranges = layout::text_line_ranges("", 16.0, 1.2, 0.0, Some(400.0), 0, 5, &mut font_system);
    assert!(ranges.is_empty(), "Empty text should return no ranges");
}
