use unshit_core::element::*;
use unshit_core::style::types::{Color, CursorStyle};
use unshit_test::TestHarness;

// ---- Extended Cursor Styles ----

#[test]
fn parse_cursor_grab() {
    let css = ".box { width: 100px; height: 100px; cursor: grab; }";
    let h = TestHarness::new(
        css,
        || ElementTree { root: ElementDef::new(Tag::Div).with_class("box") },
        800.0,
        600.0,
    );
    let snap = h.query(".box").unwrap();
    assert_eq!(snap.computed_style.cursor, CursorStyle::Grab);
}

#[test]
fn parse_cursor_grabbing() {
    let css = ".box { width: 100px; height: 100px; cursor: grabbing; }";
    let h = TestHarness::new(
        css,
        || ElementTree { root: ElementDef::new(Tag::Div).with_class("box") },
        800.0,
        600.0,
    );
    let snap = h.query(".box").unwrap();
    assert_eq!(snap.computed_style.cursor, CursorStyle::Grabbing);
}

#[test]
fn parse_cursor_not_allowed() {
    let css = ".box { width: 100px; height: 100px; cursor: not-allowed; }";
    let h = TestHarness::new(
        css,
        || ElementTree { root: ElementDef::new(Tag::Div).with_class("box") },
        800.0,
        600.0,
    );
    let snap = h.query(".box").unwrap();
    assert_eq!(snap.computed_style.cursor, CursorStyle::NotAllowed);
}

#[test]
fn parse_cursor_crosshair() {
    let css = ".box { width: 100px; height: 100px; cursor: crosshair; }";
    let h = TestHarness::new(
        css,
        || ElementTree { root: ElementDef::new(Tag::Div).with_class("box") },
        800.0,
        600.0,
    );
    let snap = h.query(".box").unwrap();
    assert_eq!(snap.computed_style.cursor, CursorStyle::Crosshair);
}

#[test]
fn parse_cursor_move() {
    let css = ".box { width: 100px; height: 100px; cursor: move; }";
    let h = TestHarness::new(
        css,
        || ElementTree { root: ElementDef::new(Tag::Div).with_class("box") },
        800.0,
        600.0,
    );
    let snap = h.query(".box").unwrap();
    assert_eq!(snap.computed_style.cursor, CursorStyle::Move);
}

#[test]
fn parse_cursor_wait() {
    let css = ".box { width: 100px; height: 100px; cursor: wait; }";
    let h = TestHarness::new(
        css,
        || ElementTree { root: ElementDef::new(Tag::Div).with_class("box") },
        800.0,
        600.0,
    );
    let snap = h.query(".box").unwrap();
    assert_eq!(snap.computed_style.cursor, CursorStyle::Wait);
}

#[test]
fn parse_cursor_help() {
    let css = ".box { width: 100px; height: 100px; cursor: help; }";
    let h = TestHarness::new(
        css,
        || ElementTree { root: ElementDef::new(Tag::Div).with_class("box") },
        800.0,
        600.0,
    );
    let snap = h.query(".box").unwrap();
    assert_eq!(snap.computed_style.cursor, CursorStyle::Help);
}

#[test]
fn parse_cursor_col_resize() {
    let css = ".box { width: 100px; height: 100px; cursor: col-resize; }";
    let h = TestHarness::new(
        css,
        || ElementTree { root: ElementDef::new(Tag::Div).with_class("box") },
        800.0,
        600.0,
    );
    let snap = h.query(".box").unwrap();
    assert_eq!(snap.computed_style.cursor, CursorStyle::ColResize);
}

#[test]
fn parse_cursor_row_resize() {
    let css = ".box { width: 100px; height: 100px; cursor: row-resize; }";
    let h = TestHarness::new(
        css,
        || ElementTree { root: ElementDef::new(Tag::Div).with_class("box") },
        800.0,
        600.0,
    );
    let snap = h.query(".box").unwrap();
    assert_eq!(snap.computed_style.cursor, CursorStyle::RowResize);
}

// ---- CSS Outline Properties ----

#[test]
fn parse_outline_color() {
    let css = ".box { width: 100px; height: 100px; outline-color: red; }";
    let h = TestHarness::new(
        css,
        || ElementTree { root: ElementDef::new(Tag::Div).with_class("box") },
        800.0,
        600.0,
    );
    let snap = h.query(".box").unwrap();
    assert_eq!(snap.computed_style.outline_color, Color::rgb(255, 0, 0));
}

#[test]
fn parse_outline_width() {
    let css = ".box { width: 100px; height: 100px; outline-width: 3px; }";
    let h = TestHarness::new(
        css,
        || ElementTree { root: ElementDef::new(Tag::Div).with_class("box") },
        800.0,
        600.0,
    );
    let snap = h.query(".box").unwrap();
    assert!((snap.computed_style.outline_width - 3.0).abs() < 0.01);
}

#[test]
fn parse_outline_offset() {
    let css = ".box { width: 100px; height: 100px; outline-offset: 5px; }";
    let h = TestHarness::new(
        css,
        || ElementTree { root: ElementDef::new(Tag::Div).with_class("box") },
        800.0,
        600.0,
    );
    let snap = h.query(".box").unwrap();
    assert!((snap.computed_style.outline_offset - 5.0).abs() < 0.01);
}

#[test]
fn parse_outline_shorthand() {
    let css = ".box { width: 100px; height: 100px; outline: 2px blue; }";
    let h = TestHarness::new(
        css,
        || ElementTree { root: ElementDef::new(Tag::Div).with_class("box") },
        800.0,
        600.0,
    );
    let snap = h.query(".box").unwrap();
    assert!((snap.computed_style.outline_width - 2.0).abs() < 0.01);
    assert_eq!(snap.computed_style.outline_color, Color::rgb(0, 0, 255));
}

#[test]
fn outline_defaults_to_zero() {
    let css = ".box { width: 100px; height: 100px; }";
    let h = TestHarness::new(
        css,
        || ElementTree { root: ElementDef::new(Tag::Div).with_class("box") },
        800.0,
        600.0,
    );
    let snap = h.query(".box").unwrap();
    assert_eq!(snap.computed_style.outline_width, 0.0);
    assert_eq!(snap.computed_style.outline_offset, 0.0);
    assert_eq!(snap.computed_style.outline_color, Color::TRANSPARENT);
}
