use unshit_core::element::*;
use unshit_test::TestHarness;

// ---------------------------------------------------------------------------
// CSS parsing tests
// ---------------------------------------------------------------------------

#[test]
fn display_grid_is_parsed() {
    let css = r#"
        .grid { display: grid; width: 400px; height: 300px; }
        .item { width: 50px; height: 50px; }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("grid")
                .with_child(ElementDef::new(Tag::Div).with_class("item").with_id("a")),
        },
        800.0,
        600.0,
    );

    let a = h.query("#a").unwrap();
    // Grid container with one child: child should be laid out
    assert_eq!(a.layout_rect.width, 50.0, "grid child width should be 50px");
    assert_eq!(a.layout_rect.height, 50.0, "grid child height should be 50px");
}

// ---------------------------------------------------------------------------
// Grid template columns / rows
// ---------------------------------------------------------------------------

#[test]
fn grid_template_columns_fixed_px() {
    let css = r#"
        .grid {
            display: grid;
            grid-template-columns: 200px 300px;
            width: 500px;
            height: 200px;
        }
        .item { height: 50px; }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("grid")
                .with_child(ElementDef::new(Tag::Div).with_class("item").with_id("a"))
                .with_child(ElementDef::new(Tag::Div).with_class("item").with_id("b")),
        },
        800.0,
        600.0,
    );

    let a = h.query("#a").unwrap();
    let b = h.query("#b").unwrap();

    assert!(
        (a.layout_rect.width - 200.0).abs() < 1.0,
        "first column should be 200px, got {}",
        a.layout_rect.width
    );
    assert!(
        (b.layout_rect.width - 300.0).abs() < 1.0,
        "second column should be 300px, got {}",
        b.layout_rect.width
    );
    // Both items on the same row
    assert!((a.layout_rect.y - b.layout_rect.y).abs() < 1.0, "items should be on the same row");
    // b starts at x=200
    assert!(
        (b.layout_rect.x - a.layout_rect.x - 200.0).abs() < 1.0,
        "b should start at x=200, got {}",
        b.layout_rect.x
    );
}

#[test]
fn grid_template_columns_fr_units() {
    let css = r#"
        .grid {
            display: grid;
            grid-template-columns: 1fr 2fr;
            width: 600px;
            height: 200px;
        }
        .item { height: 50px; }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("grid")
                .with_child(ElementDef::new(Tag::Div).with_class("item").with_id("a"))
                .with_child(ElementDef::new(Tag::Div).with_class("item").with_id("b")),
        },
        800.0,
        600.0,
    );

    let a = h.query("#a").unwrap();
    let b = h.query("#b").unwrap();

    // 1fr + 2fr = 3fr total. 600/3 = 200 per fr
    assert!(
        (a.layout_rect.width - 200.0).abs() < 1.0,
        "1fr column should be 200px, got {}",
        a.layout_rect.width
    );
    assert!(
        (b.layout_rect.width - 400.0).abs() < 1.0,
        "2fr column should be 400px, got {}",
        b.layout_rect.width
    );
}

#[test]
fn grid_template_rows_fixed_px() {
    let css = r#"
        .grid {
            display: grid;
            grid-template-columns: 1fr;
            grid-template-rows: 100px 200px;
            width: 400px;
            height: 400px;
        }
        .item {}
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("grid")
                .with_child(ElementDef::new(Tag::Div).with_class("item").with_id("a"))
                .with_child(ElementDef::new(Tag::Div).with_class("item").with_id("b")),
        },
        800.0,
        600.0,
    );

    let a = h.query("#a").unwrap();
    let b = h.query("#b").unwrap();

    assert!(
        (a.layout_rect.height - 100.0).abs() < 1.0,
        "first row should be 100px, got {}",
        a.layout_rect.height
    );
    assert!(
        (b.layout_rect.height - 200.0).abs() < 1.0,
        "second row should be 200px, got {}",
        b.layout_rect.height
    );
    assert!(
        (b.layout_rect.y - a.layout_rect.y - 100.0).abs() < 1.0,
        "b should start after a (at y=100)"
    );
}

#[test]
fn grid_template_mixed_px_and_fr() {
    let css = r#"
        .grid {
            display: grid;
            grid-template-columns: 100px 1fr 100px;
            width: 500px;
            height: 200px;
        }
        .item { height: 50px; }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("grid")
                .with_child(ElementDef::new(Tag::Div).with_class("item").with_id("a"))
                .with_child(ElementDef::new(Tag::Div).with_class("item").with_id("b"))
                .with_child(ElementDef::new(Tag::Div).with_class("item").with_id("c")),
        },
        800.0,
        600.0,
    );

    let a = h.query("#a").unwrap();
    let b = h.query("#b").unwrap();
    let c = h.query("#c").unwrap();

    // Sidebar (100px) + Content (300px = 500-100-100) + Sidebar (100px)
    assert!(
        (a.layout_rect.width - 100.0).abs() < 1.0,
        "first column 100px, got {}",
        a.layout_rect.width
    );
    assert!(
        (b.layout_rect.width - 300.0).abs() < 1.0,
        "middle column 300px (1fr), got {}",
        b.layout_rect.width
    );
    assert!(
        (c.layout_rect.width - 100.0).abs() < 1.0,
        "last column 100px, got {}",
        c.layout_rect.width
    );
}

// ---------------------------------------------------------------------------
// repeat()
// ---------------------------------------------------------------------------

#[test]
fn grid_repeat_count() {
    let css = r#"
        .grid {
            display: grid;
            grid-template-columns: repeat(3, 1fr);
            width: 600px;
            height: 200px;
        }
        .item { height: 50px; }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("grid")
                .with_child(ElementDef::new(Tag::Div).with_class("item").with_id("a"))
                .with_child(ElementDef::new(Tag::Div).with_class("item").with_id("b"))
                .with_child(ElementDef::new(Tag::Div).with_class("item").with_id("c")),
        },
        800.0,
        600.0,
    );

    let a = h.query("#a").unwrap();
    let b = h.query("#b").unwrap();
    let c = h.query("#c").unwrap();

    // Each column should be 200px (600/3)
    let tolerance = 1.0;
    assert!(
        (a.layout_rect.width - 200.0).abs() < tolerance,
        "a width should be ~200, got {}",
        a.layout_rect.width
    );
    assert!(
        (b.layout_rect.width - 200.0).abs() < tolerance,
        "b width should be ~200, got {}",
        b.layout_rect.width
    );
    assert!(
        (c.layout_rect.width - 200.0).abs() < tolerance,
        "c width should be ~200, got {}",
        c.layout_rect.width
    );
    // All on the same row
    assert_eq!(a.layout_rect.y, b.layout_rect.y);
    assert_eq!(b.layout_rect.y, c.layout_rect.y);
}

// ---------------------------------------------------------------------------
// minmax()
// ---------------------------------------------------------------------------

#[test]
fn grid_minmax_basic() {
    let css = r#"
        .grid {
            display: grid;
            grid-template-columns: minmax(100px, 1fr) minmax(200px, 2fr);
            width: 600px;
            height: 200px;
        }
        .item { height: 50px; }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("grid")
                .with_child(ElementDef::new(Tag::Div).with_class("item").with_id("a"))
                .with_child(ElementDef::new(Tag::Div).with_class("item").with_id("b")),
        },
        800.0,
        600.0,
    );

    let a = h.query("#a").unwrap();
    let b = h.query("#b").unwrap();

    // 1fr + 2fr = 3fr. 600/3 = 200 per fr
    // a: max(100, 200) = 200px
    // b: max(200, 400) = 400px
    assert!(
        (a.layout_rect.width - 200.0).abs() < 1.0,
        "first column should be 200px, got {}",
        a.layout_rect.width
    );
    assert!(
        (b.layout_rect.width - 400.0).abs() < 1.0,
        "second column should be 400px, got {}",
        b.layout_rect.width
    );
}

// ---------------------------------------------------------------------------
// gap / row-gap / column-gap
// ---------------------------------------------------------------------------

#[test]
fn grid_gap_property() {
    let css = r#"
        .grid {
            display: grid;
            grid-template-columns: 1fr 1fr;
            grid-template-rows: 100px 100px;
            gap: 20px;
            width: 420px;
            height: 220px;
        }
        .item {}
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("grid")
                .with_child(ElementDef::new(Tag::Div).with_class("item").with_id("a"))
                .with_child(ElementDef::new(Tag::Div).with_class("item").with_id("b"))
                .with_child(ElementDef::new(Tag::Div).with_class("item").with_id("c"))
                .with_child(ElementDef::new(Tag::Div).with_class("item").with_id("d")),
        },
        800.0,
        600.0,
    );

    let a = h.query("#a").unwrap();
    let b = h.query("#b").unwrap();
    let c = h.query("#c").unwrap();

    // Column gap: b should start 20px after a ends
    let gap_between_ab = b.layout_rect.x - (a.layout_rect.x + a.layout_rect.width);
    assert!(
        (gap_between_ab - 20.0).abs() < 1.0,
        "column gap should be 20px, got {}",
        gap_between_ab
    );

    // Row gap: c should start 20px after a ends (row-wise)
    let gap_between_ac = c.layout_rect.y - (a.layout_rect.y + a.layout_rect.height);
    assert!((gap_between_ac - 20.0).abs() < 1.0, "row gap should be 20px, got {}", gap_between_ac);
}

#[test]
fn grid_row_gap_column_gap_separate() {
    let css = r#"
        .grid {
            display: grid;
            grid-template-columns: 1fr 1fr;
            grid-template-rows: 100px 100px;
            row-gap: 10px;
            column-gap: 30px;
            width: 430px;
            height: 210px;
        }
        .item {}
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("grid")
                .with_child(ElementDef::new(Tag::Div).with_class("item").with_id("a"))
                .with_child(ElementDef::new(Tag::Div).with_class("item").with_id("b"))
                .with_child(ElementDef::new(Tag::Div).with_class("item").with_id("c"))
                .with_child(ElementDef::new(Tag::Div).with_class("item").with_id("d")),
        },
        800.0,
        600.0,
    );

    let a = h.query("#a").unwrap();
    let b = h.query("#b").unwrap();
    let c = h.query("#c").unwrap();

    let col_gap = b.layout_rect.x - (a.layout_rect.x + a.layout_rect.width);
    assert!((col_gap - 30.0).abs() < 1.0, "column-gap should be 30px, got {}", col_gap);

    let row_gap = c.layout_rect.y - (a.layout_rect.y + a.layout_rect.height);
    assert!((row_gap - 10.0).abs() < 1.0, "row-gap should be 10px, got {}", row_gap);
}

// ---------------------------------------------------------------------------
// grid-auto-flow
// ---------------------------------------------------------------------------

#[test]
fn grid_auto_flow_column() {
    let css = r#"
        .grid {
            display: grid;
            grid-template-rows: 100px 100px;
            grid-auto-flow: column;
            width: 400px;
            height: 200px;
        }
        .item { width: 100px; }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("grid")
                .with_child(ElementDef::new(Tag::Div).with_class("item").with_id("a"))
                .with_child(ElementDef::new(Tag::Div).with_class("item").with_id("b"))
                .with_child(ElementDef::new(Tag::Div).with_class("item").with_id("c")),
        },
        800.0,
        600.0,
    );

    let a = h.query("#a").unwrap();
    let b = h.query("#b").unwrap();
    let c = h.query("#c").unwrap();

    // With column flow and 2 rows: a fills row 1 col 1, b fills row 2 col 1, c fills row 1 col 2
    assert!(a.layout_rect.y < b.layout_rect.y, "a should be above b (both in column 1)");
    assert!(
        (c.layout_rect.y - a.layout_rect.y).abs() < 1.0,
        "c should be at same y as a (both in row 1)"
    );
    assert!(c.layout_rect.x > a.layout_rect.x, "c should be in a later column than a");
}

// ---------------------------------------------------------------------------
// Grid item placement
// ---------------------------------------------------------------------------

#[test]
fn grid_column_start_end() {
    let css = r#"
        .grid {
            display: grid;
            grid-template-columns: 100px 100px 100px;
            grid-template-rows: 100px;
            width: 300px;
            height: 100px;
        }
        .wide {
            grid-column-start: 1;
            grid-column-end: 3;
            height: 50px;
        }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("grid")
                .with_child(ElementDef::new(Tag::Div).with_class("wide").with_id("a")),
        },
        800.0,
        600.0,
    );

    let a = h.query("#a").unwrap();
    // Item should span columns 1 and 2 (lines 1 to 3 exclusive = 200px)
    assert!(
        (a.layout_rect.width - 200.0).abs() < 1.0,
        "item spanning 2 columns should be 200px wide, got {}",
        a.layout_rect.width
    );
}

#[test]
fn grid_column_shorthand() {
    let css = r#"
        .grid {
            display: grid;
            grid-template-columns: 100px 100px 100px;
            grid-template-rows: 100px;
            width: 300px;
            height: 100px;
        }
        .spanning {
            grid-column: 2 / 4;
            height: 50px;
        }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("grid")
                .with_child(ElementDef::new(Tag::Div).with_class("spanning").with_id("a")),
        },
        800.0,
        600.0,
    );

    let a = h.query("#a").unwrap();
    // Spanning columns 2-3 (lines 2 to 4) = 200px, starting at x=100
    assert!(
        (a.layout_rect.width - 200.0).abs() < 1.0,
        "item spanning columns 2-3 should be 200px wide, got {}",
        a.layout_rect.width
    );
    assert!(
        (a.layout_rect.x - 100.0).abs() < 1.0,
        "item should start at x=100 (after first column), got {}",
        a.layout_rect.x
    );
}

#[test]
fn grid_row_shorthand() {
    let css = r#"
        .grid {
            display: grid;
            grid-template-columns: 100px;
            grid-template-rows: 50px 50px 50px;
            width: 100px;
            height: 150px;
        }
        .tall {
            grid-row: 1 / 3;
            width: 100px;
        }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("grid")
                .with_child(ElementDef::new(Tag::Div).with_class("tall").with_id("a")),
        },
        800.0,
        600.0,
    );

    let a = h.query("#a").unwrap();
    assert!(
        (a.layout_rect.height - 100.0).abs() < 1.0,
        "item spanning 2 rows should be 100px tall, got {}",
        a.layout_rect.height
    );
}

#[test]
fn grid_span_placement() {
    let css = r#"
        .grid {
            display: grid;
            grid-template-columns: 100px 100px 100px;
            grid-template-rows: 50px;
            width: 300px;
            height: 50px;
        }
        .span2 {
            grid-column: span 2;
            height: 50px;
        }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("grid")
                .with_child(ElementDef::new(Tag::Div).with_class("span2").with_id("a")),
        },
        800.0,
        600.0,
    );

    let a = h.query("#a").unwrap();
    assert!(
        (a.layout_rect.width - 200.0).abs() < 1.0,
        "span 2 should produce 200px width, got {}",
        a.layout_rect.width
    );
}

// ---------------------------------------------------------------------------
// grid-area shorthand
// ---------------------------------------------------------------------------

#[test]
fn grid_area_shorthand() {
    let css = r#"
        .grid {
            display: grid;
            grid-template-columns: 100px 100px 100px;
            grid-template-rows: 100px 100px;
            width: 300px;
            height: 200px;
        }
        .placed {
            grid-area: 1 / 2 / 3 / 4;
        }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("grid")
                .with_child(ElementDef::new(Tag::Div).with_class("placed").with_id("a")),
        },
        800.0,
        600.0,
    );

    let a = h.query("#a").unwrap();
    // row-start:1, col-start:2, row-end:3, col-end:4
    // Spans columns 2-3 (200px) and rows 1-2 (200px), starting at x=100, y=0
    assert!(
        (a.layout_rect.width - 200.0).abs() < 1.0,
        "grid-area should produce 200px width, got {}",
        a.layout_rect.width
    );
    assert!(
        (a.layout_rect.height - 200.0).abs() < 1.0,
        "grid-area should produce 200px height, got {}",
        a.layout_rect.height
    );
    assert!(
        (a.layout_rect.x - 100.0).abs() < 1.0,
        "grid-area item should start at x=100, got {}",
        a.layout_rect.x
    );
}

// ---------------------------------------------------------------------------
// Common grid patterns
// ---------------------------------------------------------------------------

#[test]
fn holy_grail_layout() {
    // Classic holy grail: header, sidebar-content-sidebar, footer
    let css = r#"
        .grid {
            display: grid;
            grid-template-columns: 100px 1fr 100px;
            grid-template-rows: 50px 1fr 30px;
            width: 500px;
            height: 300px;
        }
        .header {
            grid-column: 1 / 4;
            grid-row: 1;
        }
        .sidebar-left {}
        .content {}
        .sidebar-right {}
        .footer {
            grid-column: 1 / 4;
            grid-row: 3;
        }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("grid")
                .with_child(ElementDef::new(Tag::Div).with_class("header").with_id("header"))
                .with_child(ElementDef::new(Tag::Div).with_class("sidebar-left").with_id("sl"))
                .with_child(ElementDef::new(Tag::Div).with_class("content").with_id("content"))
                .with_child(ElementDef::new(Tag::Div).with_class("sidebar-right").with_id("sr"))
                .with_child(ElementDef::new(Tag::Div).with_class("footer").with_id("footer")),
        },
        800.0,
        600.0,
    );

    let header = h.query("#header").unwrap();
    let sl = h.query("#sl").unwrap();
    let content = h.query("#content").unwrap();
    let sr = h.query("#sr").unwrap();
    let footer = h.query("#footer").unwrap();

    // Header spans full width
    assert!(
        (header.layout_rect.width - 500.0).abs() < 1.0,
        "header width should be 500px, got {}",
        header.layout_rect.width
    );
    assert!(
        (header.layout_rect.height - 50.0).abs() < 1.0,
        "header height should be 50px, got {}",
        header.layout_rect.height
    );

    // Sidebar left
    assert!(
        (sl.layout_rect.width - 100.0).abs() < 1.0,
        "left sidebar should be 100px, got {}",
        sl.layout_rect.width
    );

    // Content fills remaining space
    assert!(
        (content.layout_rect.width - 300.0).abs() < 1.0,
        "content should be 300px, got {}",
        content.layout_rect.width
    );

    // Sidebar right
    assert!(
        (sr.layout_rect.width - 100.0).abs() < 1.0,
        "right sidebar should be 100px, got {}",
        sr.layout_rect.width
    );

    // Footer spans full width
    assert!(
        (footer.layout_rect.width - 500.0).abs() < 1.0,
        "footer width should be 500px, got {}",
        footer.layout_rect.width
    );
    assert!(
        (footer.layout_rect.height - 30.0).abs() < 1.0,
        "footer height should be 30px, got {}",
        footer.layout_rect.height
    );
}

#[test]
fn dashboard_grid() {
    // Dashboard: 3 equal columns with gap
    let css = r#"
        .grid {
            display: grid;
            grid-template-columns: repeat(3, 1fr);
            gap: 16px;
            width: 500px;
            height: 200px;
        }
        .card { height: 80px; }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("grid")
                .with_child(ElementDef::new(Tag::Div).with_class("card").with_id("a"))
                .with_child(ElementDef::new(Tag::Div).with_class("card").with_id("b"))
                .with_child(ElementDef::new(Tag::Div).with_class("card").with_id("c")),
        },
        800.0,
        600.0,
    );

    let a = h.query("#a").unwrap();
    let b = h.query("#b").unwrap();
    let c = h.query("#c").unwrap();

    // 500px - 2*16px gap = 468px. 468/3 = 156px per column
    let expected_width = (500.0 - 2.0 * 16.0) / 3.0;
    let tolerance = 1.0;
    assert!(
        (a.layout_rect.width - expected_width).abs() < tolerance,
        "card width should be ~{}, got {}",
        expected_width,
        a.layout_rect.width
    );
    assert!(
        (b.layout_rect.width - expected_width).abs() < tolerance,
        "card width should be ~{}, got {}",
        expected_width,
        b.layout_rect.width
    );
    assert!(
        (c.layout_rect.width - expected_width).abs() < tolerance,
        "card width should be ~{}, got {}",
        expected_width,
        c.layout_rect.width
    );

    // Verify gap between a and b
    let gap = b.layout_rect.x - (a.layout_rect.x + a.layout_rect.width);
    assert!((gap - 16.0).abs() < tolerance, "gap should be 16px, got {}", gap);
}

#[test]
fn sidebar_plus_content() {
    let css = r#"
        .grid {
            display: grid;
            grid-template-columns: 200px 1fr;
            width: 800px;
            height: 600px;
        }
        .sidebar {}
        .content {}
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("grid")
                .with_child(ElementDef::new(Tag::Div).with_class("sidebar").with_id("s"))
                .with_child(ElementDef::new(Tag::Div).with_class("content").with_id("c")),
        },
        800.0,
        600.0,
    );

    let s = h.query("#s").unwrap();
    let c = h.query("#c").unwrap();

    assert!(
        (s.layout_rect.width - 200.0).abs() < 1.0,
        "sidebar should be 200px, got {}",
        s.layout_rect.width
    );
    assert!(
        (c.layout_rect.width - 600.0).abs() < 1.0,
        "content should be 600px (800-200), got {}",
        c.layout_rect.width
    );
}

// ---------------------------------------------------------------------------
// Auto-placement wrapping
// ---------------------------------------------------------------------------

#[test]
fn grid_auto_placement_wraps_to_rows() {
    let css = r#"
        .grid {
            display: grid;
            grid-template-columns: repeat(2, 100px);
            width: 200px;
            height: 200px;
        }
        .item { height: 50px; }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("grid")
                .with_child(ElementDef::new(Tag::Div).with_class("item").with_id("a"))
                .with_child(ElementDef::new(Tag::Div).with_class("item").with_id("b"))
                .with_child(ElementDef::new(Tag::Div).with_class("item").with_id("c"))
                .with_child(ElementDef::new(Tag::Div).with_class("item").with_id("d")),
        },
        800.0,
        600.0,
    );

    let a = h.query("#a").unwrap();
    let b = h.query("#b").unwrap();
    let c = h.query("#c").unwrap();
    let d = h.query("#d").unwrap();

    // a and b on first row, c and d on second row
    assert!((a.layout_rect.y - b.layout_rect.y).abs() < 1.0, "a and b should be on the same row");
    assert!((c.layout_rect.y - d.layout_rect.y).abs() < 1.0, "c and d should be on the same row");
    assert!(c.layout_rect.y > a.layout_rect.y, "second row should be below first");
}

// ---------------------------------------------------------------------------
// gap backward compatibility with flexbox
// ---------------------------------------------------------------------------

#[test]
fn flex_gap_still_works() {
    // Ensure the gap refactor (row_gap/column_gap) does not break flexbox gap
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
        "flex gap still works: b.y ({}) should be ~{} (a.y + a.height + gap)",
        b.layout_rect.y,
        expected_y
    );
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

#[test]
fn empty_grid_container() {
    let css = r#"
        .grid {
            display: grid;
            grid-template-columns: 1fr 1fr;
            width: 200px;
            height: 100px;
        }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree { root: ElementDef::new(Tag::Div).with_class("grid").with_id("g") },
        800.0,
        600.0,
    );

    let g = h.query("#g").unwrap();
    assert_eq!(g.layout_rect.width, 200.0, "empty grid should still have its own size");
    assert_eq!(g.layout_rect.height, 100.0);
}

#[test]
fn grid_single_cell() {
    let css = r#"
        .grid {
            display: grid;
            grid-template-columns: 1fr;
            grid-template-rows: 1fr;
            width: 300px;
            height: 200px;
        }
        .item {}
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("grid")
                .with_child(ElementDef::new(Tag::Div).with_class("item").with_id("a")),
        },
        800.0,
        600.0,
    );

    let a = h.query("#a").unwrap();
    assert!(
        (a.layout_rect.width - 300.0).abs() < 1.0,
        "single cell should fill grid width, got {}",
        a.layout_rect.width
    );
    assert!(
        (a.layout_rect.height - 200.0).abs() < 1.0,
        "single cell should fill grid height, got {}",
        a.layout_rect.height
    );
}

// ---------------------------------------------------------------------------
// gap shorthand with two values
// ---------------------------------------------------------------------------

#[test]
fn gap_shorthand_two_values() {
    let css = r#"
        .grid {
            display: grid;
            grid-template-columns: 1fr 1fr;
            grid-template-rows: 100px 100px;
            gap: 10px 20px;
            width: 420px;
            height: 210px;
        }
        .item {}
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("grid")
                .with_child(ElementDef::new(Tag::Div).with_class("item").with_id("a"))
                .with_child(ElementDef::new(Tag::Div).with_class("item").with_id("b"))
                .with_child(ElementDef::new(Tag::Div).with_class("item").with_id("c"))
                .with_child(ElementDef::new(Tag::Div).with_class("item").with_id("d")),
        },
        800.0,
        600.0,
    );

    let a = h.query("#a").unwrap();
    let b = h.query("#b").unwrap();
    let c = h.query("#c").unwrap();

    // Column gap should be 20px (second value)
    let col_gap = b.layout_rect.x - (a.layout_rect.x + a.layout_rect.width);
    assert!(
        (col_gap - 20.0).abs() < 1.0,
        "column-gap should be 20px (from gap: 10px 20px), got {}",
        col_gap
    );

    // Row gap should be 10px (first value)
    let row_gap = c.layout_rect.y - (a.layout_rect.y + a.layout_rect.height);
    assert!(
        (row_gap - 10.0).abs() < 1.0,
        "row-gap should be 10px (from gap: 10px 20px), got {}",
        row_gap
    );
}

// ---------------------------------------------------------------------------
// grid-auto-columns / grid-auto-rows
// ---------------------------------------------------------------------------

#[test]
fn grid_auto_rows_sets_implicit_row_size() {
    let css = r#"
        .grid {
            display: grid;
            grid-template-columns: 1fr 1fr;
            grid-auto-rows: 80px;
            width: 200px;
            height: 300px;
        }
        .item {}
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("grid")
                .with_child(ElementDef::new(Tag::Div).with_class("item").with_id("a"))
                .with_child(ElementDef::new(Tag::Div).with_class("item").with_id("b"))
                .with_child(ElementDef::new(Tag::Div).with_class("item").with_id("c")),
        },
        800.0,
        600.0,
    );

    let a = h.query("#a").unwrap();
    let c = h.query("#c").unwrap();

    // Implicit rows should be 80px
    assert!(
        (a.layout_rect.height - 80.0).abs() < 1.0,
        "auto row should be 80px, got {}",
        a.layout_rect.height
    );
    assert!(
        (c.layout_rect.height - 80.0).abs() < 1.0,
        "implicit second row should also be 80px, got {}",
        c.layout_rect.height
    );
}
