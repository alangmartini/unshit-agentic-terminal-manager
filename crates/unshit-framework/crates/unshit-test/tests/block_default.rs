//! Layout defaults must match the browser CSS model.
//!
//! * A `<Div>` with no explicit `display` is a block container. Children lay
//!   out in block flow (one per line, stacking vertically), each taking the
//!   container width. This is the default for the HTML `<div>` element.
//! * A container with `display: flex` and no explicit `flex-direction` uses
//!   the CSS spec default of `row`.
//! * A container with `display: flex; flex-direction: column` stacks children
//!   vertically inside a flex formatting context.
//!
//! Regression guard for the framework-first cleanup that flipped the default
//! away from `flex` (see SPEC.md, F1).

use unshit_core::element::*;
use unshit_test::TestHarness;

fn tree_two_children(root_class: &'static str) -> impl Fn() -> ElementTree {
    move || ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class(root_class)
            .with_child(ElementDef::new(Tag::Div).with_class("child").with_id("a"))
            .with_child(ElementDef::new(Tag::Div).with_class("child").with_id("b")),
    }
}

#[test]
fn div_without_display_stacks_children_in_block_flow() {
    // A bare container with sized children. Block flow means each child sits
    // on its own line, so `b` starts at `a.y + a.height` and both children
    // share the container's left edge.
    let css = r#"
        .root { width: 400px; height: 300px; }
        .child { width: 120px; height: 40px; }
    "#;
    let h = TestHarness::new(css, tree_two_children("root"), 800.0, 600.0);

    let a = h.query("#a").unwrap();
    let b = h.query("#b").unwrap();

    assert!(
        (b.layout_rect.y - (a.layout_rect.y + a.layout_rect.height)).abs() < 1.0,
        "block flow: b should sit directly below a. a.y+h = {}, b.y = {}",
        a.layout_rect.y + a.layout_rect.height,
        b.layout_rect.y,
    );
    assert!(
        (a.layout_rect.x - b.layout_rect.x).abs() < 1.0,
        "block flow: children should share the same x. a.x = {}, b.x = {}",
        a.layout_rect.x,
        b.layout_rect.x,
    );
}

#[test]
fn display_flex_without_direction_is_row() {
    // Explicit `display: flex` with no `flex-direction` must follow the CSS
    // spec default of `row`. Children sit side by side and share y.
    let css = r#"
        .root { display: flex; width: 400px; height: 300px; }
        .child { width: 120px; height: 40px; }
    "#;
    let h = TestHarness::new(css, tree_two_children("root"), 800.0, 600.0);

    let a = h.query("#a").unwrap();
    let b = h.query("#b").unwrap();

    assert!(
        (a.layout_rect.y - b.layout_rect.y).abs() < 1.0,
        "flex row: children should share y. a.y = {}, b.y = {}",
        a.layout_rect.y,
        b.layout_rect.y,
    );
    assert!(
        (b.layout_rect.x - (a.layout_rect.x + a.layout_rect.width)).abs() < 1.0,
        "flex row: b should sit directly right of a. a.x+w = {}, b.x = {}",
        a.layout_rect.x + a.layout_rect.width,
        b.layout_rect.x,
    );
}

#[test]
fn display_flex_column_stacks_vertically() {
    // Opting into column direction explicitly must stack children vertically
    // inside a flex formatting context (distinct from block flow because
    // `flex-grow` on children now resolves against this container).
    let css = r#"
        .root { display: flex; flex-direction: column; width: 400px; height: 300px; }
        .child { width: 120px; height: 40px; }
    "#;
    let h = TestHarness::new(css, tree_two_children("root"), 800.0, 600.0);

    let a = h.query("#a").unwrap();
    let b = h.query("#b").unwrap();

    assert!(
        (b.layout_rect.y - (a.layout_rect.y + a.layout_rect.height)).abs() < 1.0,
        "flex column: b should sit directly below a. a.y+h = {}, b.y = {}",
        a.layout_rect.y + a.layout_rect.height,
        b.layout_rect.y,
    );
    assert!(
        (a.layout_rect.x - b.layout_rect.x).abs() < 1.0,
        "flex column: children should share x. a.x = {}, b.x = {}",
        a.layout_rect.x,
        b.layout_rect.x,
    );
}

#[test]
fn flex_child_grows_under_column_direction() {
    // A child with `flex: 1` should consume remaining space along the main
    // axis (height here). Guards against regressing flex behavior when the
    // new default kicks in elsewhere.
    let css = r#"
        .root { display: flex; flex-direction: column; width: 200px; height: 400px; }
        .fixed { height: 100px; }
        .grow { flex: 1; }
    "#;
    let h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(ElementDef::new(Tag::Div).with_class("fixed").with_id("top"))
                .with_child(ElementDef::new(Tag::Div).with_class("grow").with_id("fill")),
        },
        800.0,
        600.0,
    );

    let top = h.query("#top").unwrap();
    let fill = h.query("#fill").unwrap();

    assert!(
        (top.layout_rect.height - 100.0).abs() < 1.0,
        "fixed child should be 100px tall, got {}",
        top.layout_rect.height,
    );
    assert!(
        (fill.layout_rect.height - 300.0).abs() < 1.0,
        "flex: 1 child should consume remaining 300px, got {}",
        fill.layout_rect.height,
    );
}
