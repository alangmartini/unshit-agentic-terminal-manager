/// Test hover behavior when cursor moves WITHIN a button containing a text span.
///
/// The hit_test returns the DEEPEST element. So as cursor moves across a button:
/// - Over padding: hit = button Div
/// - Over text: hit = child Span
///
/// This causes the hovered NodeId to change constantly, triggering restyle
/// every time. The :hover cascade uses is_or_ancestor_of, so the button should
/// stay visually hovered regardless. But does the visual output actually stay stable?
use unshit_core::element::*;
use unshit_core::style::types::Background;
use unshit_test::TestHarness;

const CSS: &str = r#"
    .root {
        display: flex;
        flex-direction: column;
        width: 100%;
        height: 100%;
        padding: 50px;
        background: #0d1117;
    }
    .btn {
        display: flex;
        align-items: center;
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
fn hover_deepest_element_oscillation() {
    let mut h = TestHarness::new(CSS, make_tree, 800.0, 600.0);
    h.set_scale_factor(1.5);
    h.step();

    let btn = h.query(".btn").unwrap();
    let btn_rect = btn.layout_rect;
    eprintln!(
        "btn rect: ({:.0}, {:.0}, {:.0}x{:.0})",
        btn_rect.x, btn_rect.y, btn_rect.width, btn_rect.height
    );

    // Move cursor across the button from left to right in small steps
    // This should cross padding -> text -> padding areas
    let y = btn_rect.y + btn_rect.height / 2.0;
    let mut hovered_changes = 0;
    let mut prev = h.hovered();

    eprintln!("\n=== Cursor sweep across button ===");
    for x_offset in 0..((btn_rect.width as i32) + 1) {
        let x = btn_rect.x + x_offset as f32;
        h.mouse_move(x, y);
        h.step();

        let current = h.hovered();
        if current != prev {
            hovered_changes += 1;
            let classes = h.hovered_classes();
            eprintln!("  x={:.0}: hovered changed to {:?} (classes: {:?})", x, current, classes);
            prev = current;
        }
    }

    eprintln!("Total hovered NodeId changes during sweep: {}", hovered_changes);

    // The hovered element WILL change (between button div and text span).
    // That's expected. What matters is: does the BUTTON's computed style stay in :hover state?
    // Check: move to center of text (should hit span), verify button still has hover background.

    let text_x = btn_rect.x + btn_rect.width / 2.0;
    h.mouse_move(text_x, y);
    h.step();

    let hovered_node = h.hovered();
    let hovered_classes = h.hovered_classes();
    eprintln!("\nAt text center: hovered = {:?} classes = {:?}", hovered_node, hovered_classes);

    // The button should STILL have the :hover background even though the deepest hit is the span
    let btn_after = h.query(".btn").unwrap();
    let Background::Color(hover_bg) = btn_after.computed_style.background else {
        panic!("expected solid color background");
    };

    eprintln!(
        "Button background after hovering text: rgb({}, {}, {})",
        hover_bg.r, hover_bg.g, hover_bg.b
    );

    // #14d892 = rgb(20, 216, 146) is the hover color
    // #10b981 = rgb(16, 185, 129) is the base color
    // At 1.5x scale, colors don't change (only dimensions scale)
    assert_eq!(
        (hover_bg.r, hover_bg.g, hover_bg.b),
        (20, 216, 146),
        "Button should have :hover background (#14d892) even when text span is the deepest hit"
    );
}

#[test]
fn restyle_count_during_button_sweep() {
    let mut h = TestHarness::new(CSS, make_tree, 800.0, 600.0);
    h.set_scale_factor(1.5);
    h.step();

    let btn = h.query(".btn").unwrap();
    let y = btn.layout_rect.y + btn.layout_rect.height / 2.0;

    // Count how many times the hovered element changes during a sweep.
    // Each change triggers a restyle. Too many restyles = visual lag.
    let mut restyle_count = 0;
    let mut prev = h.hovered();

    for x_offset in 0..((btn.layout_rect.width as i32) + 1) {
        let x = btn.layout_rect.x + x_offset as f32;
        h.mouse_move(x, y);

        if h.hovered() != prev {
            restyle_count += 1;
            prev = h.hovered();
        }
        h.step();
    }

    eprintln!("Restyles during {:.0}px sweep: {}", btn.layout_rect.width, restyle_count);

    // With a button containing text, we expect at most a few transitions:
    // padding -> text -> padding (entering, crossing text, exiting)
    // If there are many more, the hover is thrashing.
    assert!(
        restyle_count <= 6,
        "Too many hover transitions ({}) during button sweep. Expected <= 6 (padding/text boundaries).",
        restyle_count
    );
}

#[test]
fn button_visual_stable_during_sweep_gpu() {
    let h = TestHarness::new(CSS, make_tree, 800.0, 600.0);
    let Ok(mut h) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| h.with_gpu())) else {
        eprintln!("Skipping: no GPU");
        return;
    };
    h.set_scale_factor(1.5);
    h.step();

    let btn = h.query(".btn").unwrap();
    let y = btn.layout_rect.y + btn.layout_rect.height / 2.0;
    let btn_center_x = (btn.layout_rect.x + btn.layout_rect.width / 2.0) as u32;
    let btn_center_y = y as u32;

    // Move to button center (over text), step + render
    h.mouse_move(btn.layout_rect.x + btn.layout_rect.width / 2.0, y);
    h.step();
    let reference = h.render();

    // Now sweep across the button. Each position should produce the SAME pixel at button center.
    // The button background should stay the hover color regardless of which child is deepest.
    let ref_idx = ((btn_center_y as usize) * 800 + btn_center_x as usize) * 4;
    let ref_pixel = &reference[ref_idx..ref_idx + 4];
    eprintln!("Reference pixel at button center: {:?}", ref_pixel);

    for x_offset in (0..((btn.layout_rect.width as i32) + 1)).step_by(5) {
        let x = btn.layout_rect.x + x_offset as f32;
        h.mouse_move(x, y);
        h.step();
        let pixels = h.render();
        let pixel = &pixels[ref_idx..ref_idx + 4];

        // The button center pixel should always be the hover background color
        for ch in 0..4 {
            let diff = (pixel[ch] as i16 - ref_pixel[ch] as i16).unsigned_abs();
            assert!(
                diff <= 2,
                "Pixel blink at x_offset={}: channel {} changed from {} to {} (diff {})",
                x_offset,
                ch,
                ref_pixel[ch],
                pixel[ch],
                diff
            );
        }
    }
    eprintln!("Button center pixel stable through entire sweep");
}
