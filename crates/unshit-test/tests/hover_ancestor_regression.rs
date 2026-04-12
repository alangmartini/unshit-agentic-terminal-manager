/// Regression tests for ancestor-based :hover/:active matching.
///
/// The cascade must walk UP the tree: when hit_test returns a deep child,
/// every ancestor with a :hover rule should match. These tests cover
/// patterns that the original exact-equality bug would break.
use unshit_core::element::*;
use unshit_core::style::types::Background;
use unshit_test::TestHarness;

fn assert_bg(snap: &unshit_test::ElementSnapshot, r: u8, g: u8, b: u8, msg: &str) {
    match &snap.computed_style.background {
        Background::Color(c) => {
            assert_eq!((c.r, c.g, c.b), (r, g, b), "{msg}: got rgb({},{},{})", c.r, c.g, c.b);
        }
        other => panic!("{msg}: expected Color, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// 1. Deep nesting: 3+ levels, cursor on the leaf, all ancestors get :hover
// ---------------------------------------------------------------------------

fn deep_tree() -> ElementTree {
    // grandparent > parent > child(span)
    ElementTree {
        root: ElementDef::new(Tag::Div).with_class("root").with_child(
            ElementDef::new(Tag::Div).with_class("grandparent").with_child(
                ElementDef::new(Tag::Div)
                    .with_class("parent")
                    .with_child(ElementDef::new(Tag::Span).with_class("leaf").with_text("deep")),
            ),
        ),
    }
}

const DEEP_CSS: &str = r#"
    .root { width: 100%; height: 100%; }
    .grandparent { width: 300px; height: 200px; padding: 20px; background: #111111; }
    .grandparent:hover { background: #22aa22; }
    .parent { width: 260px; height: 160px; padding: 20px; background: #222222; }
    .parent:hover { background: #2222aa; }
    .leaf { color: #ffffff; font-size: 14px; }
"#;

#[test]
fn deep_nesting_all_ancestors_get_hover() {
    let mut h = TestHarness::new(DEEP_CSS, deep_tree, 800.0, 600.0);
    h.step();

    // Before hover: base colors
    assert_bg(&h.query(".grandparent").unwrap(), 0x11, 0x11, 0x11, "gp before");
    assert_bg(&h.query(".parent").unwrap(), 0x22, 0x22, 0x22, "parent before");

    // Hover the leaf text (deepest element). Both parent and grandparent should get :hover.
    let leaf = h.query(".leaf").unwrap();
    h.mouse_move(
        leaf.layout_rect.x + leaf.layout_rect.width / 2.0,
        leaf.layout_rect.y + leaf.layout_rect.height / 2.0,
    );
    h.step();

    assert_bg(&h.query(".grandparent").unwrap(), 0x22, 0xaa, 0x22, "gp hovered via leaf");
    assert_bg(&h.query(".parent").unwrap(), 0x22, 0x22, 0xaa, "parent hovered via leaf");
}

#[test]
fn deep_nesting_hover_stable_across_frames() {
    let mut h = TestHarness::new(DEEP_CSS, deep_tree, 800.0, 600.0);
    h.step();

    let leaf = h.query(".leaf").unwrap();
    h.mouse_move(
        leaf.layout_rect.x + leaf.layout_rect.width / 2.0,
        leaf.layout_rect.y + leaf.layout_rect.height / 2.0,
    );
    h.step();

    // Run 10 more frames: ancestors should keep :hover the whole time
    for frame in 0..10 {
        h.step();
        assert_bg(
            &h.query(".grandparent").unwrap(),
            0x22,
            0xaa,
            0x22,
            &format!("gp stable frame {frame}"),
        );
        assert_bg(
            &h.query(".parent").unwrap(),
            0x22,
            0x22,
            0xaa,
            &format!("parent stable frame {frame}"),
        );
    }
}

// ---------------------------------------------------------------------------
// 2. :active on child span inside button (ancestor traversal for :active)
// ---------------------------------------------------------------------------

fn btn_span_tree() -> ElementTree {
    ElementTree {
        root: ElementDef::new(Tag::Div).with_class("root").with_child(
            ElementDef::new(Tag::Div)
                .with_class("btn")
                .with_child(ElementDef::new(Tag::Span).with_text("Click me")),
        ),
    }
}

const BTN_ACTIVE_CSS: &str = r#"
    .root { width: 100%; height: 100%; }
    .btn { display: flex; align-items: center; width: 200px; height: 50px; padding: 0 20px; background: #0000ff; }
    .btn:hover { background: #00ff00; }
    .btn:active { background: #ff0000; }
"#;

#[test]
fn active_on_child_span_applies_to_button() {
    let mut h = TestHarness::new(BTN_ACTIVE_CSS, btn_span_tree, 800.0, 600.0);
    h.step();

    // Find the text area (center of button hits the span child)
    let btn = h.query(".btn").unwrap();
    let cx = btn.layout_rect.x + btn.layout_rect.width / 2.0;
    let cy = btn.layout_rect.y + btn.layout_rect.height / 2.0;

    // Hover text: button should get :hover
    h.mouse_move(cx, cy);
    h.step();
    assert_bg(&h.query(".btn").unwrap(), 0, 255, 0, "hover via span");

    // Mousedown on text: button should get :active
    h.mouse_down(cx, cy);
    h.step();
    assert_bg(&h.query(".btn").unwrap(), 255, 0, 0, "active via span");

    // Mouseup: back to :hover
    h.mouse_up(cx, cy);
    h.step();
    assert_bg(&h.query(".btn").unwrap(), 0, 255, 0, "hover after active release via span");
}

// ---------------------------------------------------------------------------
// 3. Sibling isolation: hovering one card must NOT affect siblings
// ---------------------------------------------------------------------------

fn sibling_cards_tree() -> ElementTree {
    ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(
                ElementDef::new(Tag::Div)
                    .with_class("card")
                    .with_id("card-a")
                    .with_child(ElementDef::new(Tag::Span).with_text("Card A")),
            )
            .with_child(
                ElementDef::new(Tag::Div)
                    .with_class("card")
                    .with_id("card-b")
                    .with_child(ElementDef::new(Tag::Span).with_text("Card B")),
            )
            .with_child(
                ElementDef::new(Tag::Div)
                    .with_class("card")
                    .with_id("card-c")
                    .with_child(ElementDef::new(Tag::Span).with_text("Card C")),
            ),
    }
}

const SIBLING_CSS: &str = r#"
    .root { display: flex; flex-direction: column; width: 100%; height: 100%; gap: 10px; padding: 10px; }
    .card { display: flex; width: 200px; height: 60px; padding: 10px; background: #333333; }
    .card:hover { background: #66ff66; }
"#;

#[test]
fn sibling_hover_isolation() {
    let mut h = TestHarness::new(SIBLING_CSS, sibling_cards_tree, 800.0, 600.0);
    h.step();

    let card_b = h.query("#card-b").unwrap();

    // Hover card B's text (child span)
    h.mouse_move(
        card_b.layout_rect.x + card_b.layout_rect.width / 2.0,
        card_b.layout_rect.y + card_b.layout_rect.height / 2.0,
    );
    h.step();

    // Card B should have hover color
    assert_bg(&h.query("#card-b").unwrap(), 0x66, 0xff, 0x66, "card-b hovered");

    // Cards A and C must stay at base color
    assert_bg(&h.query("#card-a").unwrap(), 0x33, 0x33, 0x33, "card-a should not be hovered");
    assert_bg(&h.query("#card-c").unwrap(), 0x33, 0x33, 0x33, "card-c should not be hovered");
}

#[test]
fn sibling_hover_switches_correctly() {
    let mut h = TestHarness::new(SIBLING_CSS, sibling_cards_tree, 800.0, 600.0);
    h.step();

    let card_a = h.query("#card-a").unwrap();
    let card_b = h.query("#card-b").unwrap();

    // Hover card A
    h.mouse_move(
        card_a.layout_rect.x + card_a.layout_rect.width / 2.0,
        card_a.layout_rect.y + card_a.layout_rect.height / 2.0,
    );
    h.step();
    assert_bg(&h.query("#card-a").unwrap(), 0x66, 0xff, 0x66, "card-a hovered");
    assert_bg(&h.query("#card-b").unwrap(), 0x33, 0x33, 0x33, "card-b base");

    // Move to card B
    h.mouse_move(
        card_b.layout_rect.x + card_b.layout_rect.width / 2.0,
        card_b.layout_rect.y + card_b.layout_rect.height / 2.0,
    );
    h.step();
    assert_bg(&h.query("#card-a").unwrap(), 0x33, 0x33, 0x33, "card-a no longer hovered");
    assert_bg(&h.query("#card-b").unwrap(), 0x66, 0xff, 0x66, "card-b now hovered");
}

// ---------------------------------------------------------------------------
// 4. Descendant combinator: .parent:hover .child applies only when parent hovered
// ---------------------------------------------------------------------------

fn descendant_combinator_tree() -> ElementTree {
    ElementTree {
        root: ElementDef::new(Tag::Div).with_class("root").with_child(
            ElementDef::new(Tag::Div)
                .with_class("wrapper")
                .with_child(ElementDef::new(Tag::Span).with_class("label").with_text("Hover me")),
        ),
    }
}

const DESCENDANT_CSS: &str = r#"
    .root { width: 100%; height: 100%; }
    .wrapper { width: 200px; height: 100px; padding: 20px; background: #444444; }
    .label { color: #888888; font-size: 14px; }
    .wrapper:hover .label { color: #ff0000; }
"#;

#[test]
fn descendant_combinator_hover() {
    let mut h = TestHarness::new(DESCENDANT_CSS, descendant_combinator_tree, 800.0, 600.0);
    h.step();

    // Before hover: label has base color
    let label = h.query(".label").unwrap();
    assert_eq!(
        (label.computed_style.color.r, label.computed_style.color.g, label.computed_style.color.b),
        (0x88, 0x88, 0x88),
        "label base color"
    );

    // Hover the label text (deepest hit). .wrapper is ancestor, so .wrapper:hover matches.
    // Then .wrapper:hover .label should also match.
    h.mouse_move(
        label.layout_rect.x + label.layout_rect.width / 2.0,
        label.layout_rect.y + label.layout_rect.height / 2.0,
    );
    h.step();

    let label = h.query(".label").unwrap();
    assert_eq!(
        (label.computed_style.color.r, label.computed_style.color.g, label.computed_style.color.b),
        (0xff, 0x00, 0x00),
        "label should get red color via .wrapper:hover .label when hovering text"
    );
}

#[test]
fn descendant_combinator_clears_on_leave() {
    let mut h = TestHarness::new(DESCENDANT_CSS, descendant_combinator_tree, 800.0, 600.0);
    h.step();

    // Hover
    let label = h.query(".label").unwrap();
    h.mouse_move(
        label.layout_rect.x + label.layout_rect.width / 2.0,
        label.layout_rect.y + label.layout_rect.height / 2.0,
    );
    h.step();

    let label = h.query(".label").unwrap();
    assert_eq!(
        (label.computed_style.color.r, label.computed_style.color.g, label.computed_style.color.b),
        (0xff, 0x00, 0x00),
        "hover active"
    );

    // Move away
    h.mouse_move(700.0, 500.0);
    h.step();

    let label = h.query(".label").unwrap();
    assert_eq!(
        (label.computed_style.color.r, label.computed_style.color.g, label.computed_style.color.b),
        (0x88, 0x88, 0x88),
        "label should revert after cursor leaves wrapper"
    );
}

// ---------------------------------------------------------------------------
// 5. No-hover ancestor: element without :hover rule in the chain doesn't interfere
// ---------------------------------------------------------------------------

fn no_hover_ancestor_tree() -> ElementTree {
    ElementTree {
        root: ElementDef::new(Tag::Div).with_class("root").with_child(
            ElementDef::new(Tag::Div).with_class("outer").with_child(
                ElementDef::new(Tag::Div)
                    .with_class("inner")
                    .with_child(ElementDef::new(Tag::Span).with_text("text")),
            ),
        ),
    }
}

const NO_HOVER_ANCESTOR_CSS: &str = r#"
    .root { width: 100%; height: 100%; }
    .outer { width: 300px; height: 200px; padding: 20px; background: #111111; }
    .inner { width: 260px; height: 100px; padding: 10px; background: #333333; }
    .inner:hover { background: #00ff00; }
"#;

#[test]
fn no_hover_ancestor_does_not_interfere() {
    let mut h = TestHarness::new(NO_HOVER_ANCESTOR_CSS, no_hover_ancestor_tree, 800.0, 600.0);
    h.step();

    // Hover the text inside .inner (through .outer which has no :hover rule)
    let inner = h.query(".inner").unwrap();
    h.mouse_move(
        inner.layout_rect.x + inner.layout_rect.width / 2.0,
        inner.layout_rect.y + inner.layout_rect.height / 2.0,
    );
    h.step();

    // .inner should get hover color
    assert_bg(&h.query(".inner").unwrap(), 0x00, 0xff, 0x00, "inner hovered");
    // .outer stays at base (no :hover rule)
    assert_bg(&h.query(".outer").unwrap(), 0x11, 0x11, 0x11, "outer unchanged");
}
