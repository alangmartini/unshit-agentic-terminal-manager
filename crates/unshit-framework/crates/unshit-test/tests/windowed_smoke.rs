use unshit_core::element::*;
use unshit_test::WindowedTest;

/// Smoke test: create a WindowedTest, pump 1 frame, verify the harness
/// initializes and the element tree is queryable.
///
/// This test is ignored by default because it creates a real OS window,
/// which requires a display and may not work in headless CI.
/// Run with: cargo test -p unshit-test --test windowed_smoke -- --ignored
#[test]
#[ignore]
fn windowed_test_creates_and_pumps() {
    let css = r#"
        .root { display: flex; width: 100%; height: 100%; background: #1a1a2e; }
        .box { width: 200px; height: 200px; background: #e94560; }
    "#;

    let mut wt = WindowedTest::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(ElementDef::new(Tag::Div).with_class("box")),
        },
        800,
        600,
    );

    assert!(wt.is_initialized(), "WindowedTest should be initialized after new()");

    // Pump one frame to run the full style/layout/render pipeline
    wt.pump(1);

    // Query the element tree to verify it was built correctly
    let root = wt.query(".root").expect("root element should exist");
    assert!(root.layout_rect.width > 0.0, "root should have nonzero width");

    let child = wt.query(".box").expect("box element should exist");
    // The box is 200px in CSS, but the WindowedTest applies the OS scale
    // factor (e.g. 150% DPI => 300px). Just verify it has nonzero width.
    assert!(
        child.layout_rect.width > 0.0,
        "box should have nonzero width, got {}",
        child.layout_rect.width
    );
}

/// Verify that OS input injection functions compile and are callable.
/// This doesn't assert behavior (it would move the real cursor), just
/// confirms the FFI bindings link correctly.
#[test]
fn os_input_compiles() {
    // These are no-ops on non-Windows, and on Windows they call FFI
    // but we don't assert behavior since it would move the real cursor.
    let (x, y) = unshit_test::os_input::get_cursor_pos();
    // On Windows this returns real coords; on other platforms (0, 0).
    let _ = (x, y);
}
