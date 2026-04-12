use unshit_core::element::*;
use unshit_test::{TestEvent, TestHarness};

/// Simulate cursor micro-jitter (common with real mice/trackpads).
/// The cursor moves tiny amounts around the same spot between redraws.
#[test]
fn micro_jitter_hover_stable() {
    let css = r#"
        .root { display: flex; width: 100%; height: 100%; padding: 50px; background: #0d1117; }
        .btn { width: 120px; height: 50px; background: #10b981; border-radius: 14px; }
        .btn:hover { background: #14d892; box-shadow: 0px 10px 30px rgba(16, 185, 129, 0.35); }
    "#;

    let mut h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(ElementDef::new(Tag::Div).with_class("btn")),
        },
        800.0,
        600.0,
    );
    h.step();

    let btn = h.query(".btn").unwrap();
    let cx = btn.layout_rect.x + btn.layout_rect.width / 2.0;
    let cy = btn.layout_rect.y + btn.layout_rect.height / 2.0;

    // Simulate: move to button, then tiny micro-movements with redraws between
    let scenario = vec![
        TestEvent::CursorMoved { x: cx, y: cy },
        TestEvent::Wait { frames: 1 },
        TestEvent::CursorMoved { x: cx + 0.5, y: cy }, // sub-pixel jitter
        TestEvent::Wait { frames: 1 },
        TestEvent::CursorMoved { x: cx, y: cy + 0.3 }, // sub-pixel jitter
        TestEvent::Wait { frames: 2 },                 // multiple redraws
        TestEvent::CursorMoved { x: cx - 0.2, y: cy + 0.1 }, // more jitter
        TestEvent::Wait { frames: 1 },
        TestEvent::CursorMoved { x: cx + 0.1, y: cy - 0.4 },
        TestEvent::Wait { frames: 3 },
        TestEvent::AssertHoverStable { frames: 5 },
    ];

    h.replay(&scenario);
    assert!(
        h.hovered_classes().contains(&"btn".to_string()),
        "should still be hovering btn after micro-jitter"
    );
}

/// Simulate rapid cursor enter/leave pattern.
/// The cursor quickly moves in and out of the element.
#[test]
fn rapid_enter_leave() {
    let css = r#"
        .root { display: flex; width: 100%; height: 100%; padding: 50px; background: #0d1117; }
        .box { width: 200px; height: 100px; background: #333; }
        .box:hover { background: #666; }
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
    let cx = b.layout_rect.x + b.layout_rect.width / 2.0;
    let cy = b.layout_rect.y + b.layout_rect.height / 2.0;
    let outside_x = b.layout_rect.x + b.layout_rect.width + 50.0;

    // Rapidly enter and leave
    let scenario = vec![
        TestEvent::CursorMoved { x: cx, y: cy }, // enter
        TestEvent::Wait { frames: 1 },
        TestEvent::CursorMoved { x: outside_x, y: cy }, // leave
        TestEvent::Wait { frames: 1 },
        TestEvent::CursorMoved { x: cx, y: cy }, // enter again
        TestEvent::Wait { frames: 1 },
        TestEvent::CursorMoved { x: outside_x, y: cy }, // leave again
        TestEvent::Wait { frames: 1 },
        TestEvent::CursorMoved { x: cx, y: cy }, // final enter
        TestEvent::Wait { frames: 1 },
        TestEvent::AssertHoverStable { frames: 5 }, // should stay stable
    ];

    h.replay(&scenario);
    assert!(h.hovered_classes().contains(&"box".to_string()));
}

/// Simulate hover followed by many frames without input.
/// This mimics the ControlFlow::Poll continuous redraw loop where
/// the app redraws every frame even without new events.
#[test]
fn hover_during_continuous_redraw() {
    let css = r#"
        .root { display: flex; width: 100%; height: 100%; padding: 50px; background: #0d1117; }
        .btn { width: 120px; height: 50px; background: #10b981; }
        .btn:hover { background: #14d892; box-shadow: 0px 10px 30px rgba(16, 185, 129, 0.35); }
    "#;

    let mut h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(ElementDef::new(Tag::Div).with_class("btn")),
        },
        800.0,
        600.0,
    );
    h.step();

    let btn = h.query(".btn").unwrap();
    let cx = btn.layout_rect.x + btn.layout_rect.width / 2.0;
    let cy = btn.layout_rect.y + btn.layout_rect.height / 2.0;

    // Move to button, then simulate 50 frames of continuous redraw (no input)
    let scenario = vec![
        TestEvent::CursorMoved { x: cx, y: cy },
        TestEvent::Wait { frames: 50 }, // 50 frames of poll-mode redraw
        TestEvent::AssertHoverStable { frames: 10 },
    ];

    h.replay(&scenario);
    assert!(
        h.hovered_classes().contains(&"btn".to_string()),
        "hover should persist through 50 frames of continuous redraw"
    );
}
