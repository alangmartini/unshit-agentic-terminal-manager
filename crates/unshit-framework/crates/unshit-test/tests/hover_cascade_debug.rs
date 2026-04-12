/// Debug: check if the button's computed style is correct at the transition point.
use unshit_core::element::*;
use unshit_core::style::types::Background;
use unshit_test::TestHarness;

const CSS: &str = r#"
    .root { display: flex; flex-direction: column; width: 100%; height: 100%; padding: 50px; background: #0d1117; }
    .btn {
        display: flex; align-items: center; padding: 14px 28px;
        background: #10b981; border-radius: 14px;
        box-shadow: 0px 8px 24px rgba(16, 185, 129, 0.25);
    }
    .btn:hover {
        background: #14d892;
        box-shadow: 0px 10px 30px rgba(16, 185, 129, 0.35);
    }
"#;

fn make_tree() -> ElementTree {
    ElementTree {
        root: ElementDef::new(Tag::Div).with_class("root").with_child(
            ElementDef::new(Tag::Div)
                .with_class("btn")
                .with_child(ElementDef::new(Tag::Span).with_text("Build something")),
        ),
    }
}

#[test]
fn check_computed_style_at_transition() {
    let mut h = TestHarness::new(CSS, make_tree, 800.0, 600.0);
    h.set_scale_factor(1.5);
    h.step();

    let btn = h.query(".btn").unwrap();
    let y = btn.layout_rect.y + btn.layout_rect.height / 2.0;

    // x=116: last pixel on padding (hovered = btn)
    // x=117: first pixel on text (hovered = span)
    for x in [116.0, 117.0, 118.0, 120.0] {
        h.mouse_move(btn.layout_rect.x + x, y);
        h.step();

        let hovered = h.hovered();
        let hovered_classes = h.hovered_classes();

        // Check the BUTTON's computed style (not the hovered element's style)
        let btn_snap = h.query(".btn").unwrap();
        let Background::Color(bg) = btn_snap.computed_style.background else {
            panic!("expected color");
        };

        eprintln!(
            "x={:.0}: hovered={:?} classes={:?} | btn bg=rgb({},{},{}) [expected hover: 20,216,146 base: 16,185,129]",
            btn.layout_rect.x + x, hovered, hovered_classes, bg.r, bg.g, bg.b
        );
    }
}
