use cosmic_text::FontSystem;
use unshit_core::layout;

#[test]
fn hit_test_returns_some_for_valid_position() {
    let mut font_system = FontSystem::new();
    let text = "Hello, world!";

    let result = layout::hit_test_text_position(
        text,
        16.0,
        1.2,
        0.0,
        Some(200.0),
        5.0,
        10.0,
        &mut font_system,
    );
    assert!(result.is_some(), "Should find a character near start of text");
    assert!(result.unwrap() <= 2, "Position near start should be byte offset 0 or 1");
}

#[test]
fn hit_test_empty_text_returns_none() {
    let mut font_system = FontSystem::new();
    let result = layout::hit_test_text_position(
        "",
        16.0,
        1.2,
        0.0,
        Some(200.0),
        50.0,
        10.0,
        &mut font_system,
    );
    assert!(result.is_none());
}

#[test]
fn hit_test_middle_of_text() {
    let mut font_system = FontSystem::new();
    let text = "AAAA BBBB";

    let (total_w, _) = layout::measure_text(text, 16.0, 1.2, 0.0, Some(400.0), &mut font_system);

    let mid_x = total_w / 2.0;
    let result = layout::hit_test_text_position(
        text,
        16.0,
        1.2,
        0.0,
        Some(400.0),
        mid_x,
        10.0,
        &mut font_system,
    );
    assert!(result.is_some());
    let offset = result.unwrap();
    assert!((3..=6).contains(&offset), "Middle hit should be around byte 3-6, got {offset}");
}

#[test]
fn glyph_ranges_returns_entries() {
    let mut font_system = FontSystem::new();
    let text = "ABC";

    let ranges = layout::text_glyph_ranges(text, 16.0, 1.2, 0.0, Some(200.0), &mut font_system);

    assert_eq!(ranges.len(), 3, "Should have 3 glyph ranges for 'ABC'");

    assert!(ranges[0].x < ranges[1].x, "Glyphs should be left-to-right");
    assert!(ranges[1].x < ranges[2].x, "Glyphs should be left-to-right");

    assert_eq!(ranges[0].byte_start, 0);
    assert_eq!(ranges[2].byte_end, 3);

    for r in &ranges {
        assert!(r.width > 0.0, "Glyph width should be positive");
        assert!(r.height > 0.0, "Glyph height should be positive");
    }
}

#[test]
fn glyph_ranges_with_letter_spacing() {
    let mut font_system = FontSystem::new();
    let text = "AB";

    let ranges_no_spacing =
        layout::text_glyph_ranges(text, 16.0, 1.2, 0.0, Some(200.0), &mut font_system);
    let ranges_with_spacing =
        layout::text_glyph_ranges(text, 16.0, 1.2, 5.0, Some(200.0), &mut font_system);

    assert_eq!(ranges_no_spacing.len(), 2);
    assert_eq!(ranges_with_spacing.len(), 2);

    let gap_no = ranges_no_spacing[1].x - ranges_no_spacing[0].x;
    let gap_with = ranges_with_spacing[1].x - ranges_with_spacing[0].x;
    assert!(gap_with > gap_no, "Letter spacing should increase gap between glyphs");
}
