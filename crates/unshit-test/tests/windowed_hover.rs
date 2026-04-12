/// Windowed integration test for hover.
///
/// Only ONE test function because winit allows only one EventLoop per process.
/// Run with: cargo test -p unshit-test --test windowed_hover -- --ignored
use unshit_core::element::*;
use unshit_test::WindowedTest;

#[test]
#[ignore]
fn windowed_hover_integration() {
    let css = r#"
        .root { display: flex; flex-direction: column; width: 100%; height: 100%; padding: 50px; background: #0d1117; gap: 20px; }
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
        .box { width: 200px; height: 100px; background: #ff0000; }
        .box:hover { background: #00ff00; }
    "#;

    let mut wt = WindowedTest::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(
                    ElementDef::new(Tag::Div)
                        .with_class("btn")
                        .with_child(ElementDef::new(Tag::Span).with_text("Hover me")),
                )
                .with_child(ElementDef::new(Tag::Div).with_class("box")),
        },
        800,
        600,
    );

    // --- Scenario 1: OS-level hover via injection ---
    let btn = wt.query(".btn").expect(".btn not found");
    let btn_cx = btn.layout_rect.x + btn.layout_rect.width / 2.0;
    let btn_cy = btn.layout_rect.y + btn.layout_rect.height / 2.0;

    wt.inject_mouse_move(btn_cx, btn_cy);
    wt.pump(3);
    eprintln!("[windowed] After OS hover on btn: hovered = {:?}", wt.hovered());

    // --- Scenario 2: Rapid in/out ---
    let b = wt.query(".box").expect(".box not found");
    let box_cx = b.layout_rect.x + b.layout_rect.width / 2.0;
    let box_cy = b.layout_rect.y + b.layout_rect.height / 2.0;
    let outside_x = b.layout_rect.x + b.layout_rect.width + 100.0;

    for i in 0..5 {
        wt.inject_mouse_move(box_cx, box_cy);
        wt.pump(1);
        wt.inject_mouse_move(outside_x, box_cy);
        wt.pump(1);
        eprintln!("[windowed] Rapid hover cycle {}: no crash", i);
    }

    // --- Scenario 3: Mouse down/up ---
    wt.inject_mouse_move(btn_cx, btn_cy);
    wt.pump(2);
    wt.inject_mouse_down();
    wt.pump(1);
    wt.inject_mouse_up();
    wt.pump(1);
    eprintln!("[windowed] Mouse down/up complete, no crash");

    // --- Scenario 4: Pump many frames (simulates ControlFlow::Poll) ---
    wt.inject_mouse_move(btn_cx, btn_cy);
    wt.pump(20);
    eprintln!("[windowed] 20 frames of continuous pump after hover: stable");
}
