use unshit_core::dirty::DirtyFlags;
use unshit_core::element::*;
use unshit_core::style::types::{Background, Color};
use unshit_test::TestHarness;

fn try_with_gpu(h: TestHarness) -> Option<TestHarness> {
    // GPU might not be available in CI; skip gracefully
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| h.with_gpu())).ok()
}

#[test]
fn btn_hover_pixels_stable() {
    let css = r#"
        .root { display: flex; flex-direction: column; width: 100%; height: 100%; padding: 50px; background: #0d1117; }
        .btn { display: flex; padding: 14px 28px; background: #10b981; border-radius: 14px; box-shadow: 0px 8px 24px rgba(16, 185, 129, 0.25); }
        .btn:hover { background: #14d892; box-shadow: 0px 10px 30px rgba(16, 185, 129, 0.35); }
    "#;

    let h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div).with_class("root").with_child(
                ElementDef::new(Tag::Div)
                    .with_class("btn")
                    .with_child(ElementDef::new(Tag::Span).with_text("Click me")),
            ),
        },
        800.0,
        600.0,
    );

    let Some(mut h) = try_with_gpu(h) else {
        eprintln!("Skipping: no GPU");
        return;
    };
    h.step();

    let btn = h.query(".btn").unwrap();
    let cx = btn.layout_rect.x + btn.layout_rect.width / 2.0;
    let cy = btn.layout_rect.y + btn.layout_rect.height / 2.0;

    h.mouse_move(cx, cy);
    h.step();

    // Render 10 frames; all should produce identical pixels
    h.assert_render_stable(10);
}

#[test]
fn feature_card_hover_pixels_stable() {
    let css = r#"
        .root { display: flex; width: 100%; height: 100%; gap: 16px; padding: 32px; background: #0d1117; }
        .card { display: flex; flex-direction: column; flex-grow: 1; background: rgba(13, 17, 23, 0.4); border-radius: 20px; border-width: 1px; border-color: rgba(16, 185, 129, 0.12); padding: 24px; }
        .card:hover { border-color: rgba(16, 185, 129, 0.25); background: rgba(13, 17, 23, 0.55); }
        .label { color: #e6edf3; font-size: 16px; }
    "#;

    let h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div).with_class("root").with_child(
                ElementDef::new(Tag::Div)
                    .with_class("card")
                    .with_child(ElementDef::new(Tag::Span).with_class("label").with_text("GPU")),
            ),
        },
        800.0,
        600.0,
    );

    let Some(mut h) = try_with_gpu(h) else {
        eprintln!("Skipping: no GPU");
        return;
    };
    h.step();

    let card = h.query(".card").unwrap();
    let cx = card.layout_rect.x + card.layout_rect.width / 2.0;
    let cy = card.layout_rect.y + card.layout_rect.height / 2.0;

    h.mouse_move(cx, cy);
    h.step();
    h.assert_render_stable(10);
}

#[test]
fn hover_pixel_color_matches_style() {
    // Verify the rendered pixel at element center matches the CSS hover color
    let css = r#"
        .root { display: flex; width: 100%; height: 100%; padding: 50px; background: #000000; }
        .box { width: 200px; height: 100px; background: #ff0000; }
        .box:hover { background: #00ff00; }
    "#;

    let h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(ElementDef::new(Tag::Div).with_class("box")),
        },
        800.0,
        600.0,
    );

    let Some(mut h) = try_with_gpu(h) else {
        eprintln!("Skipping: no GPU");
        return;
    };
    h.step();

    let b = h.query(".box").unwrap();
    let cx = (b.layout_rect.x + b.layout_rect.width / 2.0) as u32;
    let cy = (b.layout_rect.y + b.layout_rect.height / 2.0) as u32;

    // Before hover: should be red
    let pixels = h.render();
    let idx = ((cy * 800 + cx) * 4) as usize;
    // Check red channel is high, green is low (with tolerance for GPU differences)
    assert!(pixels[idx] > 200, "expected red channel > 200, got {}", pixels[idx]);
    assert!(pixels[idx + 1] < 50, "expected green channel < 50, got {}", pixels[idx + 1]);

    // After hover: should be green
    h.mouse_move(
        b.layout_rect.x + b.layout_rect.width / 2.0,
        b.layout_rect.y + b.layout_rect.height / 2.0,
    );
    h.step();
    let after_hover = h.query(".box").unwrap();
    assert_eq!(
        after_hover.computed_style.background,
        Background::Color(Color::rgb(0, 255, 0)),
        "computed hover style should be green before pixel assertion"
    );
    let dirty = h.arena().get(after_hover.node_id).unwrap().dirty;
    assert!(
        dirty.contains(DirtyFlags::PAINT),
        "hovered box should be paint dirty before render, got {dirty:?}"
    );
    let pixels = h.render();
    assert!(pixels[idx] < 50, "expected red channel < 50 after hover, got {}", pixels[idx]);
    assert!(
        pixels[idx + 1] > 200,
        "expected green channel > 200 after hover, got {}",
        pixels[idx + 1]
    );
}

#[test]
fn no_hover_renders_base_color() {
    let css = r#"
        .root { display: flex; width: 100%; height: 100%; background: #0d1117; }
        .box { width: 100px; height: 100px; background: #333333; }
    "#;

    let h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(ElementDef::new(Tag::Div).with_class("box")),
        },
        400.0,
        400.0,
    );

    let Some(mut h) = try_with_gpu(h) else {
        eprintln!("Skipping: no GPU");
        return;
    };
    h.step();

    // Render without any hover -- should show base background color
    let pixels = h.render();
    // Just verify it rendered something (not all zeros)
    assert!(pixels.iter().any(|&p| p > 0), "rendered frame is all black");
    // Verify render is stable without hover too
    h.assert_render_stable(5);
}
