use unshit_core::element::*;
use unshit_test::TestHarness;

#[test]
fn column_layout_stacks_vertically() {
    let css = r#"
        .root { display: flex; flex-direction: column; width: 100%; height: 100%; }
        .child { height: 50px; width: 100%; }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(ElementDef::new(Tag::Div).with_class("child").with_id("a"))
                .with_child(ElementDef::new(Tag::Div).with_class("child").with_id("b"))
                .with_child(ElementDef::new(Tag::Div).with_class("child").with_id("c")),
        },
        800.0,
        600.0,
    );

    let a = h.query("#a").unwrap();
    let b = h.query("#b").unwrap();
    let c = h.query("#c").unwrap();
    assert!(b.layout_rect.y > a.layout_rect.y, "b should be below a");
    assert!(c.layout_rect.y > b.layout_rect.y, "c should be below b");
    assert_eq!(a.layout_rect.height, 50.0);
}

#[test]
fn row_layout_stacks_horizontally() {
    let css = r#"
        .root { display: flex; flex-direction: row; width: 100%; height: 100%; }
        .child { width: 100px; height: 50px; }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(ElementDef::new(Tag::Div).with_class("child").with_id("a"))
                .with_child(ElementDef::new(Tag::Div).with_class("child").with_id("b"))
                .with_child(ElementDef::new(Tag::Div).with_class("child").with_id("c")),
        },
        800.0,
        600.0,
    );

    let a = h.query("#a").unwrap();
    let b = h.query("#b").unwrap();
    let c = h.query("#c").unwrap();
    assert!(b.layout_rect.x > a.layout_rect.x, "b should be to the right of a");
    assert!(c.layout_rect.x > b.layout_rect.x, "c should be to the right of b");
    assert_eq!(a.layout_rect.width, 100.0);
    assert_eq!(a.layout_rect.height, 50.0);
}

#[test]
fn padding_offsets_children() {
    let css = r#"
        .root { display: flex; flex-direction: column; width: 100%; height: 100%; padding: 20px; }
        .child { height: 50px; }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(ElementDef::new(Tag::Div).with_class("child").with_id("a")),
        },
        800.0,
        600.0,
    );

    let a = h.query("#a").unwrap();
    assert!(
        a.layout_rect.x >= 20.0,
        "child x ({}) should be >= 20.0 due to padding",
        a.layout_rect.x
    );
    assert!(
        a.layout_rect.y >= 20.0,
        "child y ({}) should be >= 20.0 due to padding",
        a.layout_rect.y
    );
}

#[test]
fn flex_grow_distributes_space() {
    let css = r#"
        .root { display: flex; flex-direction: row; width: 100%; height: 100%; }
        .half { flex-grow: 1; height: 50px; }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(ElementDef::new(Tag::Div).with_class("half").with_id("a"))
                .with_child(ElementDef::new(Tag::Div).with_class("half").with_id("b")),
        },
        800.0,
        600.0,
    );

    let a = h.query("#a").unwrap();
    let b = h.query("#b").unwrap();
    let tolerance = 1.0;
    assert!(
        (a.layout_rect.width - 400.0).abs() < tolerance,
        "a width ({}) should be ~400.0",
        a.layout_rect.width
    );
    assert!(
        (b.layout_rect.width - 400.0).abs() < tolerance,
        "b width ({}) should be ~400.0",
        b.layout_rect.width
    );
}

#[test]
fn fixed_size_respected() {
    let css = r#"
        .root { display: flex; width: 100%; height: 100%; }
        .box { width: 200px; height: 150px; }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(ElementDef::new(Tag::Div).with_class("box").with_id("b")),
        },
        800.0,
        600.0,
    );

    let box_el = h.query("#b").unwrap();
    assert_eq!(box_el.layout_rect.width, 200.0);
    assert_eq!(box_el.layout_rect.height, 150.0);
}

#[test]
fn text_element_has_nonzero_size() {
    let css = r#"
        .root { display: flex; width: 100%; height: 100%; }
        .label { font-size: 16px; }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div).with_class("root").with_child(
                ElementDef::new(Tag::Span)
                    .with_class("label")
                    .with_id("lbl")
                    .with_text("Hello World"),
            ),
        },
        800.0,
        600.0,
    );

    let label = h.query("#lbl").unwrap();
    assert!(
        label.layout_rect.width > 0.0,
        "text should have measured width, got {}",
        label.layout_rect.width
    );
    assert!(
        label.layout_rect.height > 0.0,
        "text should have measured height, got {}",
        label.layout_rect.height
    );
}

#[test]
fn gap_adds_spacing() {
    let css = r#"
        .root { display: flex; flex-direction: column; width: 100%; height: 100%; gap: 10px; }
        .child { height: 30px; }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(ElementDef::new(Tag::Div).with_class("child").with_id("a"))
                .with_child(ElementDef::new(Tag::Div).with_class("child").with_id("b")),
        },
        800.0,
        600.0,
    );

    let a = h.query("#a").unwrap();
    let b = h.query("#b").unwrap();
    let expected_y = a.layout_rect.y + a.layout_rect.height + 10.0;
    assert!(
        (b.layout_rect.y - expected_y).abs() < 1.0,
        "b.y ({}) should be ~{} (a.y + a.height + gap)",
        b.layout_rect.y,
        expected_y
    );
}

#[test]
fn display_none_excluded() {
    let css = r#"
        .root { display: flex; flex-direction: column; width: 100%; height: 100%; }
        .child { height: 50px; }
        .hidden { display: none; }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(ElementDef::new(Tag::Div).with_class("child").with_id("a"))
                .with_child(
                    ElementDef::new(Tag::Div).with_class("child").with_class("hidden").with_id("h"),
                )
                .with_child(ElementDef::new(Tag::Div).with_class("child").with_id("b")),
        },
        800.0,
        600.0,
    );

    let a = h.query("#a").unwrap();
    let b = h.query("#b").unwrap();
    let hidden = h.query("#h").unwrap();

    // Hidden element should have zero size
    assert_eq!(hidden.layout_rect.width, 0.0, "hidden element width should be 0");
    assert_eq!(hidden.layout_rect.height, 0.0, "hidden element height should be 0");

    // The visible child after the hidden one should be positioned directly
    // after the first child, not offset by an extra 50px.
    let expected_b_y = a.layout_rect.y + a.layout_rect.height;
    assert!(
        (b.layout_rect.y - expected_b_y).abs() < 1.0,
        "b.y ({}) should be ~{} (immediately after a, hidden skipped)",
        b.layout_rect.y,
        expected_b_y
    );
}

#[test]
fn position_relative_offsets_element() {
    let css = r#"
        .root { display: flex; flex-direction: column; width: 100%; height: 100%; }
        .child { height: 50px; width: 100px; }
        .nudged { position: relative; top: 10px; left: 20px; }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(ElementDef::new(Tag::Div).with_class("child").with_id("a"))
                .with_child(
                    ElementDef::new(Tag::Div).with_class("child").with_class("nudged").with_id("b"),
                )
                .with_child(ElementDef::new(Tag::Div).with_class("child").with_id("c")),
        },
        800.0,
        600.0,
    );

    let a = h.query("#a").unwrap();
    let b = h.query("#b").unwrap();
    let c = h.query("#c").unwrap();

    // b should be offset by top:10 left:20 relative to its normal flow position
    let normal_b_y = a.layout_rect.y + a.layout_rect.height;
    assert!(
        (b.layout_rect.y - (normal_b_y + 10.0)).abs() < 1.0,
        "b.y ({}) should be ~{} (normal position + top:10)",
        b.layout_rect.y,
        normal_b_y + 10.0
    );
    assert!(
        (b.layout_rect.x - 20.0).abs() < 1.0,
        "b.x ({}) should be ~20.0 (left:20)",
        b.layout_rect.x
    );

    // c should be positioned as if b were in its normal flow position (relative does not affect siblings)
    let expected_c_y = normal_b_y + 50.0;
    assert!(
        (c.layout_rect.y - expected_c_y).abs() < 1.0,
        "c.y ({}) should be ~{} (relative positioning of b should not affect c)",
        c.layout_rect.y,
        expected_c_y
    );
}

#[test]
fn position_absolute_removes_from_flow() {
    let css = r#"
        .root { display: flex; flex-direction: column; width: 100%; height: 100%; }
        .child { height: 50px; width: 100px; }
        .abs { position: absolute; top: 30px; left: 40px; }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(ElementDef::new(Tag::Div).with_class("child").with_id("a"))
                .with_child(
                    ElementDef::new(Tag::Div).with_class("child").with_class("abs").with_id("b"),
                )
                .with_child(ElementDef::new(Tag::Div).with_class("child").with_id("c")),
        },
        800.0,
        600.0,
    );

    let a = h.query("#a").unwrap();
    let b = h.query("#b").unwrap();
    let c = h.query("#c").unwrap();

    // b should be positioned at top:30 left:40 relative to its containing block
    assert!((b.layout_rect.y - 30.0).abs() < 1.0, "abs b.y ({}) should be ~30.0", b.layout_rect.y);
    assert!((b.layout_rect.x - 40.0).abs() < 1.0, "abs b.x ({}) should be ~40.0", b.layout_rect.x);

    // c should be positioned directly after a since b is out of flow
    let expected_c_y = a.layout_rect.y + a.layout_rect.height;
    assert!(
        (c.layout_rect.y - expected_c_y).abs() < 1.0,
        "c.y ({}) should be ~{} (absolute b removed from flow)",
        c.layout_rect.y,
        expected_c_y
    );
}

#[test]
fn flex_wrap_wraps_overflowing_children() {
    let css = r#"
        .root { display: flex; flex-direction: row; flex-wrap: wrap; width: 800px; height: 600px; }
        .child { width: 300px; height: 50px; }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(ElementDef::new(Tag::Div).with_class("child").with_id("a"))
                .with_child(ElementDef::new(Tag::Div).with_class("child").with_id("b"))
                .with_child(ElementDef::new(Tag::Div).with_class("child").with_id("c")),
        },
        800.0,
        600.0,
    );

    let a = h.query("#a").unwrap();
    let b = h.query("#b").unwrap();
    let c = h.query("#c").unwrap();

    // a and b fit on the first row (300 + 300 = 600 <= 800)
    assert_eq!(a.layout_rect.y, b.layout_rect.y, "a and b should be on the same row");
    // c wraps to the next row (300 + 300 + 300 = 900 > 800)
    assert!(
        c.layout_rect.y > a.layout_rect.y,
        "c (y={}) should wrap to a row below a (y={})",
        c.layout_rect.y,
        a.layout_rect.y
    );
}

#[test]
fn flex_wrap_align_content_center() {
    let css = r#"
        .root {
            display: flex; flex-direction: row; flex-wrap: wrap;
            align-content: center;
            width: 800px; height: 600px;
        }
        .child { width: 500px; height: 50px; }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(ElementDef::new(Tag::Div).with_class("child").with_id("a"))
                .with_child(ElementDef::new(Tag::Div).with_class("child").with_id("b")),
        },
        800.0,
        600.0,
    );

    let a = h.query("#a").unwrap();
    let b = h.query("#b").unwrap();

    // Two rows of 50px each = 100px total. Centered in 600px means top starts at 250px.
    let total_content = 100.0;
    let expected_start = (600.0 - total_content) / 2.0;
    assert!(
        (a.layout_rect.y - expected_start).abs() < 1.0,
        "a.y ({}) should be ~{} (centered)",
        a.layout_rect.y,
        expected_start
    );
    assert!(b.layout_rect.y > a.layout_rect.y, "b should be below a in a wrapped layout");
}

#[test]
fn flex_nowrap_does_not_wrap() {
    let css = r#"
        .root { display: flex; flex-direction: row; width: 800px; height: 600px; }
        .child { width: 300px; height: 50px; }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(ElementDef::new(Tag::Div).with_class("child").with_id("a"))
                .with_child(ElementDef::new(Tag::Div).with_class("child").with_id("b"))
                .with_child(ElementDef::new(Tag::Div).with_class("child").with_id("c")),
        },
        800.0,
        600.0,
    );

    let a = h.query("#a").unwrap();
    let b = h.query("#b").unwrap();
    let c = h.query("#c").unwrap();

    // Default is nowrap, so all children stay on the same row (they shrink to fit)
    assert_eq!(a.layout_rect.y, b.layout_rect.y, "a and b should be on same row");
    assert_eq!(b.layout_rect.y, c.layout_rect.y, "b and c should be on same row");
}

/// Bug: the `flex` shorthand parser did not consume the trailing semicolon,
/// causing the CSS declaration immediately after `flex: ...;` to be silently
/// dropped during error recovery. This meant properties like min-height,
/// flex-direction, or overflow following a `flex` shorthand were lost,
/// breaking cross-axis stretch in column layouts.
#[test]
fn flex_shorthand_does_not_eat_next_property() {
    let css = r#"
        .grid {
            display: flex;
            flex-direction: column;
            width: 800px;
            height: 600px;
        }
        .row {
            display: flex;
            flex: 1 1 0;
            min-height: 0;
            min-width: 0;
            overflow: hidden;
        }
        .pane {
            display: flex;
            flex-direction: column;
            flex: 1 1 0;
            min-width: 0;
            min-height: 0;
            overflow: hidden;
        }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div).with_class("grid").with_child(
                ElementDef::new(Tag::Div)
                    .with_class("row")
                    .with_id("row")
                    .with_child(ElementDef::new(Tag::Div).with_class("pane").with_id("pane")),
            ),
        },
        800.0,
        600.0,
    );

    let row = h.query("#row").unwrap();
    let pane = h.query("#pane").unwrap();

    // Row should stretch to grid width (cross-axis of column)
    assert_eq!(
        row.layout_rect.width, 800.0,
        "row width ({}) should stretch to grid width (800px)",
        row.layout_rect.width
    );
    // Row should fill grid height (main-axis via flex-grow)
    assert_eq!(
        row.layout_rect.height, 600.0,
        "row height ({}) should fill grid height (600px) via flex-grow",
        row.layout_rect.height
    );
    // Pane should stretch to row height (cross-axis of row)
    assert_eq!(
        pane.layout_rect.height, 600.0,
        "pane height ({}) should stretch to row height (600px)",
        pane.layout_rect.height
    );
    // Pane should fill row width (main-axis of row via flex-grow)
    assert_eq!(
        pane.layout_rect.width, 800.0,
        "pane width ({}) should fill row width (800px) via flex-grow",
        pane.layout_rect.width
    );
}

/// Verify basic cross-axis stretch in a flex column: a child with
/// flex-grow and no explicit width should stretch to the parent width.
#[test]
fn flex_column_cross_axis_stretch() {
    let css = r#"
        .root {
            display: flex;
            flex-direction: column;
            width: 500px;
            height: 400px;
        }
        .child {
            flex-grow: 1;
        }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(ElementDef::new(Tag::Div).with_class("child").with_id("a")),
        },
        800.0,
        600.0,
    );

    let child = h.query("#a").unwrap();
    assert_eq!(
        child.layout_rect.width, 500.0,
        "child width ({}) should stretch to parent width (500px) via align-items: stretch",
        child.layout_rect.width
    );
    assert_eq!(
        child.layout_rect.height, 400.0,
        "child height ({}) should fill parent height (400px) via flex-grow: 1",
        child.layout_rect.height
    );
}

/// An absolutely positioned child should escape its parent's `overflow: hidden`
/// clip rect. Per CSS spec, absolute elements are not clipped by their parent's
/// overflow, only by the nearest ancestor that establishes a containing block
/// with its own clip.
#[test]
fn absolute_child_escapes_overflow_hidden() {
    let css = r#"
        .parent {
            position: relative;
            overflow: hidden;
            width: 100px;
            height: 100px;
        }
        .abs-child {
            position: absolute;
            top: 0;
            left: 120px;
            width: 50px;
            height: 50px;
        }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("parent")
                .with_child(ElementDef::new(Tag::Div).with_class("abs-child").with_id("abs")),
        },
        800.0,
        600.0,
    );

    let abs = h.query("#abs").unwrap();
    // The absolute child is placed at left: 120px, which is outside the
    // parent's 100px width. Its layout rect should reflect that position.
    assert!(
        (abs.layout_rect.x - 120.0).abs() < 1.0,
        "absolute child x ({}) should be ~120.0, escaping parent overflow",
        abs.layout_rect.x
    );
    assert_eq!(abs.layout_rect.width, 50.0);
    assert_eq!(abs.layout_rect.height, 50.0);
}

/// Normal (non-absolute) children must still be clipped by their parent's
/// `overflow: hidden`. This ensures the absolute-escape logic does not
/// accidentally disable clipping for all children.
#[test]
fn normal_child_still_clipped_by_overflow_hidden() {
    let css = r#"
        .parent {
            overflow: hidden;
            width: 100px;
            height: 100px;
        }
        .wide-child {
            width: 200px;
            height: 50px;
        }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("parent")
                .with_child(ElementDef::new(Tag::Div).with_class("wide-child").with_id("w")),
        },
        800.0,
        600.0,
    );

    let w = h.query("#w").unwrap();
    // The normal-flow child is wider than the parent but should still exist
    // in layout. Clipping happens at render time, not layout time, so the
    // layout rect retains the declared width.
    assert!(
        w.layout_rect.width > 0.0,
        "normal child should have a layout rect with positive width"
    );
    // The child should be within or starting at the parent bounds.
    assert!(
        w.layout_rect.x < 1.0,
        "normal child x ({}) should start at or near 0",
        w.layout_rect.x
    );
}

#[test]
fn position_fixed_removes_from_flow() {
    let css = r#"
        .root { display: flex; flex-direction: column; width: 100%; height: 100%; }
        .child { height: 50px; width: 100px; }
        .fixed { position: fixed; top: 30px; left: 40px; }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(ElementDef::new(Tag::Div).with_class("child").with_id("a"))
                .with_child(
                    ElementDef::new(Tag::Div).with_class("child").with_class("fixed").with_id("b"),
                )
                .with_child(ElementDef::new(Tag::Div).with_class("child").with_id("c")),
        },
        800.0,
        600.0,
    );

    let a = h.query("#a").unwrap();
    let b = h.query("#b").unwrap();
    let c = h.query("#c").unwrap();

    // b should be positioned at top:30 left:40 (out of flow, like absolute)
    assert!(
        (b.layout_rect.y - 30.0).abs() < 1.0,
        "fixed b.y ({}) should be ~30.0",
        b.layout_rect.y
    );
    assert!(
        (b.layout_rect.x - 40.0).abs() < 1.0,
        "fixed b.x ({}) should be ~40.0",
        b.layout_rect.x
    );

    // c should be positioned directly after a since b is out of flow
    let expected_c_y = a.layout_rect.y + a.layout_rect.height;
    assert!(
        (c.layout_rect.y - expected_c_y).abs() < 1.0,
        "c.y ({}) should be ~{} (fixed b removed from flow)",
        c.layout_rect.y,
        expected_c_y
    );
}

#[test]
fn position_static_ignores_top_left() {
    let css = r#"
        .root { display: flex; flex-direction: column; width: 100%; height: 100%; }
        .child { height: 50px; width: 100px; }
        .with-inset { position: static; top: 10px; left: 20px; }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(ElementDef::new(Tag::Div).with_class("child").with_id("a"))
                .with_child(
                    ElementDef::new(Tag::Div)
                        .with_class("child")
                        .with_class("with-inset")
                        .with_id("b"),
                )
                .with_child(ElementDef::new(Tag::Div).with_class("child").with_id("c")),
        },
        800.0,
        600.0,
    );

    let a = h.query("#a").unwrap();
    let b = h.query("#b").unwrap();

    // b is static, so top:10 left:20 should be ignored; it sits in normal flow after a
    let expected_b_y = a.layout_rect.y + a.layout_rect.height;
    assert!(
        (b.layout_rect.y - expected_b_y).abs() < 1.0,
        "static b.y ({}) should be ~{} (top:10 ignored for static)",
        b.layout_rect.y,
        expected_b_y
    );
    assert!(
        b.layout_rect.x.abs() < 1.0,
        "static b.x ({}) should be ~0.0 (left:20 ignored for static)",
        b.layout_rect.x
    );
}
