/// Reproduction tests for hover blink bug.
///
/// The real bug: hovering a button causes laggy/blinking hover instead of
/// consistent hover until the mouse leaves. This happens because:
/// 1. Cursor moves to button -> hit_test finds it -> hover applied -> restyle
/// 2. Restyle changes layout (hover styles can shift element position)
/// 3. Cursor micro-moves (same spot) -> hit_test runs against NEW layout
/// 4. If element shifted, hit_test misses it -> hover removed -> restyle back
/// 5. Repeat = blink
///
/// The key insight: assert_hover_stable() doesn't catch this because it never
/// re-runs hit_test after a step. We need to simulate cursor "jitter" by
/// calling mouse_move(same_position) after each step.
use unshit_core::element::*;
use unshit_test::TestHarness;

/// Helper: step + re-hit-test at the same position (simulates cursor micro-movement)
fn step_and_rehit(h: &mut TestHarness, x: f32, y: f32) {
    h.step();
    h.mouse_move(x, y); // re-run hit_test against post-layout positions
}

// --------------------------------------------------------------------------
// Test with hello.rs btn-primary CSS (box-shadow changes on hover)
// --------------------------------------------------------------------------
#[test]
fn btn_primary_hover_no_blink() {
    let css = r#"
        .root {
            display: flex;
            flex-direction: column;
            width: 100%;
            height: 100%;
            padding: 50px;
            background: #0d1117;
        }
        .btn-primary {
            display: flex;
            align-items: center;
            padding: 14px 28px;
            background: #10b981;
            border-radius: 14px;
            color: #ffffff;
            font-size: 15px;
            font-weight: bold;
            box-shadow: 0px 8px 24px rgba(16, 185, 129, 0.25);
        }
        .btn-primary:hover {
            background: #14d892;
            box-shadow: 0px 10px 30px rgba(16, 185, 129, 0.35);
        }
        .btn-primary:active {
            background: #0e9d6e;
            box-shadow: 0px 4px 12px rgba(16, 185, 129, 0.2);
        }
    "#;

    let mut h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div).with_class("root").with_child(
                ElementDef::new(Tag::Div)
                    .with_class("btn-primary")
                    .with_child(ElementDef::new(Tag::Span).with_text("Build something")),
            ),
        },
        1280.0,
        900.0,
    );
    h.step();

    let btn = h.query(".btn-primary").unwrap();
    let cx = btn.layout_rect.x + btn.layout_rect.width / 2.0;
    let cy = btn.layout_rect.y + btn.layout_rect.height / 2.0;

    // Initial hover
    h.mouse_move(cx, cy);
    h.step();
    assert!(
        h.hovered_classes().contains(&"btn-primary".to_string()),
        "should be hovering btn-primary after first move"
    );

    // Simulate 10 frames of cursor micro-jitter at the same spot
    for frame in 0..10 {
        step_and_rehit(&mut h, cx, cy);
        assert!(
            h.hovered_classes().contains(&"btn-primary".to_string()),
            "hover blinked on frame {}: hovered {:?} instead of btn-primary",
            frame,
            h.hovered_classes()
        );
    }
}

// --------------------------------------------------------------------------
// Test with feature-card CSS (border-color + background change on hover)
// --------------------------------------------------------------------------
#[test]
fn feature_card_hover_no_blink() {
    let css = r#"
        .root {
            display: flex;
            flex-direction: row;
            width: 100%;
            height: 100%;
            gap: 16px;
            padding: 32px;
            background: #0d1117;
        }
        .feature-card {
            display: flex;
            flex-direction: column;
            flex-grow: 1;
            background: rgba(13, 17, 23, 0.4);
            border-radius: 20px;
            border-width: 1px;
            border-color: rgba(16, 185, 129, 0.12);
            padding: 24px;
            gap: 12px;
            box-shadow: 0px 10px 30px rgba(0, 0, 0, 0.25);
        }
        .feature-card:hover {
            border-color: rgba(16, 185, 129, 0.25);
            background: rgba(13, 17, 23, 0.55);
        }
        .label { color: #e6edf3; font-size: 16px; }
    "#;

    let mut h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(ElementDef::new(Tag::Div).with_class("feature-card").with_child(
                    ElementDef::new(Tag::Span).with_class("label").with_text("GPU Rendering"),
                ))
                .with_child(ElementDef::new(Tag::Div).with_class("feature-card").with_child(
                    ElementDef::new(Tag::Span).with_class("label").with_text("Flexbox Layout"),
                )),
        },
        1280.0,
        900.0,
    );
    h.step();

    let card = h.query(".feature-card").unwrap();
    let cx = card.layout_rect.x + card.layout_rect.width / 2.0;
    let cy = card.layout_rect.y + card.layout_rect.height / 2.0;

    h.mouse_move(cx, cy);
    h.step();

    for frame in 0..10 {
        step_and_rehit(&mut h, cx, cy);
        assert!(
            h.hovered_classes().contains(&"feature-card".to_string()),
            "feature-card hover blinked on frame {}: hovered {:?}",
            frame,
            h.hovered_classes()
        );
    }
}

// --------------------------------------------------------------------------
// Test with DPI scaling (scale_factor > 1 can cause layout rounding issues)
// --------------------------------------------------------------------------
#[test]
fn hover_no_blink_at_150_percent_scale() {
    let css = r#"
        .root {
            display: flex;
            flex-direction: column;
            width: 100%;
            height: 100%;
            padding: 32px;
        }
        .btn {
            display: flex;
            padding: 14px 28px;
            background: #10b981;
            border-radius: 14px;
            box-shadow: 0px 8px 24px rgba(16, 185, 129, 0.25);
        }
        .btn:hover {
            background: #14d892;
            box-shadow: 0px 10px 30px rgba(16, 185, 129, 0.35);
        }
    "#;

    let mut h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div).with_class("root").with_child(
                ElementDef::new(Tag::Div)
                    .with_class("btn")
                    .with_child(ElementDef::new(Tag::Span).with_text("Click me")),
            ),
        },
        1920.0,
        1080.0,
    );

    // Simulate 150% DPI scaling (common on Windows)
    h.set_scale_factor(1.5);
    h.step();

    let btn = h.query(".btn").unwrap();
    let cx = btn.layout_rect.x + btn.layout_rect.width / 2.0;
    let cy = btn.layout_rect.y + btn.layout_rect.height / 2.0;

    h.mouse_move(cx, cy);
    h.step();

    for frame in 0..10 {
        step_and_rehit(&mut h, cx, cy);
        assert!(
            h.hovered_classes().contains(&"btn".to_string()),
            "hover blinked at 1.5x scale on frame {}: hovered {:?}",
            frame,
            h.hovered_classes()
        );
    }
}

// --------------------------------------------------------------------------
// Edge case: hover at the very edge of an element
// --------------------------------------------------------------------------
#[test]
fn hover_at_edge_no_blink() {
    let css = r#"
        .root {
            display: flex;
            width: 100%;
            height: 100%;
            padding: 50px;
        }
        .box {
            width: 200px;
            height: 100px;
            background: #333;
        }
        .box:hover {
            background: #666;
            box-shadow: 0px 5px 15px rgba(0, 0, 0, 0.3);
        }
    "#;

    let mut h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(ElementDef::new(Tag::Div).with_class("box")),
        },
        800.0,
        600.0,
    );
    h.step();

    let b = h.query(".box").unwrap();
    // Hover at the right edge (1px inside)
    let edge_x = b.layout_rect.x + b.layout_rect.width - 1.0;
    let edge_y = b.layout_rect.y + b.layout_rect.height / 2.0;

    h.mouse_move(edge_x, edge_y);
    h.step();

    for frame in 0..10 {
        step_and_rehit(&mut h, edge_x, edge_y);
        assert!(
            h.hovered_classes().contains(&"box".to_string()),
            "edge hover blinked on frame {}: hovered {:?}",
            frame,
            h.hovered_classes()
        );
    }
}
