//! Headed mode demo: run this to watch the test framework interact with a
//! real window.
//!
//! Run with:
//!   cargo test -p unshit-test --test headed_demo -- --ignored --nocapture
//!
//! The test opens a window, clicks around, hovers elements, and pauses
//! so you can inspect the result before it closes.

use unshit_core::element::*;
use unshit_test::WindowedTest;

#[test]
#[ignore]
fn watch_test_interact_with_window() {
    let css = r#"
        .app {
            display: flex;
            flex-direction: column;
            width: 100%;
            height: 100%;
            background: #0f0f23;
            padding: 20px;
            gap: 16px;
        }
        .title {
            width: 100%;
            height: 50px;
            color: #ccccff;
            font-size: 24px;
        }
        .counter-row {
            display: flex;
            width: 100%;
            height: 60px;
            gap: 12px;
        }
        .count {
            width: 200px;
            height: 50px;
            color: #00cc7a;
            font-size: 32px;
        }
        .inc-btn, .dec-btn {
            width: 120px;
            height: 50px;
            background: #1a1a3e;
            color: #ccccff;
            font-size: 18px;
        }
        .inc-btn:hover, .dec-btn:hover {
            background: #e94560;
            color: #ffffff;
        }
        .inc-btn:active, .dec-btn:active {
            background: #ff6b81;
        }
        .status {
            width: 100%;
            height: 30px;
            color: #666680;
            font-size: 14px;
        }
    "#;

    let mut wt = WindowedTest::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("app")
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("title")
                        .with_text("unshit-test headed demo"),
                )
                .with_child(
                    ElementDef::new(Tag::Div)
                        .with_class("counter-row")
                        .with_child(
                            ElementDef::new(Tag::Span).with_class("count").with_text("Count: 0"),
                        )
                        .with_child(
                            ElementDef::new(Tag::Button)
                                .with_class("inc-btn")
                                .with_text("+ Increment"),
                        )
                        .with_child(
                            ElementDef::new(Tag::Button)
                                .with_class("dec-btn")
                                .with_text("- Decrement"),
                        ),
                )
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("status")
                        .with_text("Test framework is driving this window..."),
                ),
        },
        800,
        600,
    );

    eprintln!("\n=== Headed Demo: Watch the test interact with the window ===\n");

    // Render a few frames so the window shows up
    eprintln!("  [1/6] Window opened, rendering initial state...");
    wt.pump(30);
    sleep_ms(800);

    // Hover over the increment button
    let inc_btn = wt.query(".inc-btn").expect("increment button exists");
    let cx = inc_btn.layout_rect.x + inc_btn.layout_rect.width / 2.0;
    let cy = inc_btn.layout_rect.y + inc_btn.layout_rect.height / 2.0;

    eprintln!("  [2/6] Moving mouse to increment button...");
    wt.inject_mouse_move(cx, cy);
    wt.pump(20);
    sleep_ms(600);

    // Click it
    eprintln!("  [3/6] Clicking increment button...");
    wt.inject_click(cx, cy);
    wt.pump(10);
    sleep_ms(500);

    // Hover over the decrement button
    let dec_btn = wt.query(".dec-btn").expect("decrement button exists");
    let dx = dec_btn.layout_rect.x + dec_btn.layout_rect.width / 2.0;
    let dy = dec_btn.layout_rect.y + dec_btn.layout_rect.height / 2.0;

    eprintln!("  [4/6] Moving mouse to decrement button...");
    wt.inject_mouse_move(dx, dy);
    wt.pump(20);
    sleep_ms(600);

    // Click decrement
    eprintln!("  [5/6] Clicking decrement button...");
    wt.inject_click(dx, dy);
    wt.pump(10);
    sleep_ms(500);

    // Move mouse around to show hover effects
    eprintln!("  [6/6] Sweeping mouse across buttons to show hover effects...");
    for i in 0..30 {
        let t = i as f32 / 29.0;
        let mx = cx + (dx - cx) * t;
        let my = cy + (dy - cy) * t;
        wt.inject_mouse_move(mx, my);
        wt.pump(2);
        sleep_ms(30);
    }

    sleep_ms(500);

    // Verify the tree is queryable
    let title = wt.query(".title").expect("title exists");
    assert!(title.layout_rect.width > 0.0);

    eprintln!("\n  Done! Window will close now.");
    eprintln!("  All assertions passed.\n");
    sleep_ms(1000);
}

fn sleep_ms(ms: u64) {
    std::thread::sleep(std::time::Duration::from_millis(ms));
}
