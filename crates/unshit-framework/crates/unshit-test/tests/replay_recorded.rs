/// Replay a real recorded session and check for hover blink.
///
/// Uses events.json captured from: UNSHIT_RECORD_EVENTS=1 cargo run --example claude_code
/// The recording contains 430 cursor events at 1.5x DPI scale.
use unshit_core::element::*;
use unshit_test::{TestEvent, TestHarness};

/// Build the claude_code example's sidebar (the part with hoverable session items).
/// Simplified version focusing on the hoverable elements.
fn claude_code_tree() -> ElementTree {
    ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(
                ElementDef::new(Tag::Div)
                    .with_class("sidebar")
                    .with_child(
                        ElementDef::new(Tag::Div).with_class("sidebar-header").with_child(
                            ElementDef::new(Tag::Span)
                                .with_class("sidebar-title")
                                .with_text("Sessions"),
                        ),
                    )
                    .with_child(session_item("1", "plane", false))
                    .with_child(session_item("2", "opensessions", true))
                    .with_child(session_item("3", "quiver", false)),
            )
            .with_child(ElementDef::new(Tag::Div).with_class("divider"))
            .with_child(
                ElementDef::new(Tag::Div)
                    .with_class("main")
                    .with_child(
                        ElementDef::new(Tag::Span)
                            .with_class("text-body")
                            .with_text("Some content here"),
                    )
                    .with_child(
                        ElementDef::new(Tag::Div)
                            .with_class("code-badge")
                            .with_child(ElementDef::new(Tag::Span).with_text("README.md")),
                    )
                    .with_child(
                        ElementDef::new(Tag::Span)
                            .with_class("link")
                            .with_text("docs/reference/features.md"),
                    ),
            ),
    }
}

fn session_item(num: &str, name: &str, active: bool) -> ElementDef {
    ElementDef::new(Tag::Div)
        .with_class(if active { "session-active" } else { "session-item" })
        .with_child(
            ElementDef::new(Tag::Div)
                .with_class("session-row")
                .with_child(ElementDef::new(Tag::Span).with_class("session-number").with_text(num))
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class(if active { "session-name-hl" } else { "session-name" })
                        .with_text(name),
                ),
        )
}

/// The actual claude_code CSS (trimmed to the hover-relevant parts).
const CLAUDE_CODE_CSS: &str = r#"
    .root {
        display: flex;
        flex-direction: row;
        width: 100%;
        height: 100%;
        background: rgba(13, 17, 23, 0.95);
    }
    .sidebar {
        display: flex;
        flex-direction: column;
        width: 220px;
        flex-shrink: 0;
        background: rgba(11, 15, 20, 0.85);
    }
    .sidebar-header {
        display: flex;
        align-items: center;
        padding: 14px 16px;
        gap: 8px;
    }
    .sidebar-title { color: #e6edf3; font-size: 14px; font-weight: bold; }
    .session-item {
        display: flex;
        flex-direction: column;
        padding: 10px 16px;
        gap: 2px;
        cursor: pointer;
    }
    .session-active {
        display: flex;
        flex-direction: column;
        padding: 10px 16px;
        gap: 2px;
        background: rgba(16, 185, 129, 0.12);
    }
    .session-row { display: flex; align-items: center; gap: 10px; }
    .session-number { color: #484f58; font-size: 13px; }
    .session-name { color: #8b949e; font-size: 14px; }
    .session-name-hl { color: #10b981; font-size: 14px; font-weight: bold; }
    .divider { width: 1px; background: rgba(16, 185, 129, 0.15); flex-shrink: 0; }
    .main {
        display: flex;
        flex-direction: column;
        flex-grow: 1;
        padding: 20px 28px;
        gap: 4px;
    }
    .text-body { color: #e6edf3; font-size: 14px; line-height: 1.5; }
    .code-badge {
        display: flex;
        align-items: center;
        padding: 1px 6px;
        background: rgba(16, 185, 129, 0.1);
        border-radius: 4px;
        color: #34d399;
        font-size: 14px;
    }
    .link { color: #58a6ff; font-size: 14px; cursor: pointer; }

    .session-item:hover { background: rgba(16, 185, 129, 0.06); }
    .link:hover { color: #79b8ff; }
    .code-badge:hover { background: rgba(16, 185, 129, 0.18); color: #6ee7b7; }
"#;

/// Replay recorded events through the headless harness and detect hover oscillation.
/// The recording was captured at 1.5x DPI scale, so we set scale_factor = 1.5.
#[test]
fn replay_recorded_session_no_blink() {
    let mut h = TestHarness::new(CLAUDE_CODE_CSS, claude_code_tree, 1100.0, 750.0);
    h.set_scale_factor(1.5);
    h.step();

    let events = TestEvent::load_recording("tests/recordings/hover_session.json");
    assert!(!events.is_empty(), "Failed to load recording (check path relative to crate root)");
    eprintln!("Loaded {} events from recording", events.len());

    // Replay with blink detection: after each CursorMoved + step,
    // check if hover oscillates on subsequent steps.
    // True oscillation = A -> B -> A pattern (returning to a previous value quickly).
    // Sequential transitions (A -> B -> C) during fast cursor movement are expected.
    let mut prev_hovered = h.hovered();
    let mut hover_changes = 0;
    let mut rapid_oscillations = 0;
    let mut prev_prev_hovered = prev_hovered;

    for (i, event) in events.iter().enumerate() {
        match event {
            TestEvent::CursorMoved { x, y } => {
                h.mouse_move(*x, *y);
                h.step();

                let current = h.hovered();
                if current != prev_hovered {
                    hover_changes += 1;

                    // Detect A -> B -> A oscillation: current matches the value before last change
                    if current == prev_prev_hovered && current != prev_hovered {
                        rapid_oscillations += 1;
                        eprintln!(
                            "[BLINK?] Frame {}: hover oscillated {:?} -> {:?} -> {:?}",
                            i, prev_prev_hovered, prev_hovered, current
                        );
                    }

                    prev_prev_hovered = prev_hovered;
                    prev_hovered = current;
                }

                // Also check: does a second step (without new input) change hover?
                let hovered_after_step = h.hovered();
                h.step();
                let hovered_after_step2 = h.hovered();
                if hovered_after_step != hovered_after_step2 {
                    eprintln!(
                        "[BLINK!] Frame {}: hover changed between steps! {:?} -> {:?}",
                        i, hovered_after_step, hovered_after_step2,
                    );
                    rapid_oscillations += 10; // severe
                }
            }
            _ => {
                // MouseDown/MouseUp: just process
                h.replay(std::slice::from_ref(event));
            }
        }
    }

    eprintln!("Total hover changes: {}", hover_changes);
    eprintln!("Rapid oscillations: {}", rapid_oscillations);

    // Some hover changes are expected (cursor moves between elements).
    // A single A->B->A can happen when cursor crosses a thin element boundary.
    // The blink bug causes repeated oscillations within one element, so we allow
    // a small number of isolated oscillations from normal cursor movement.
    assert!(
        rapid_oscillations <= 2,
        "Detected {} rapid hover oscillations during replay (max 2 allowed). This indicates the hover blink bug.",
        rapid_oscillations
    );
}

/// Same replay but with GPU rendering to check for pixel-level instability.
#[test]
fn replay_recorded_session_render_stable() {
    let h = TestHarness::new(CLAUDE_CODE_CSS, claude_code_tree, 1100.0, 750.0);

    let Ok(mut h) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| h.with_gpu())) else {
        eprintln!("Skipping: no GPU");
        return;
    };

    h.set_scale_factor(1.5);
    h.step();

    let events = TestEvent::load_recording("tests/recordings/hover_session.json");
    if events.is_empty() {
        eprintln!("Skipping: no recording found");
        return;
    }

    // Find a hover event over a known element and render multiple frames
    for event in &events {
        if let TestEvent::CursorMoved { x, y } = event {
            h.mouse_move(*x, *y);
            h.step();

            // If we're hovering something, check render stability
            if !h.hovered().is_dangling() {
                let reference = h.render();
                for frame in 0..5 {
                    h.step();
                    let current = h.render();
                    assert!(
                        reference
                            .iter()
                            .zip(current.iter())
                            .all(|(a, b)| (*a as i16 - *b as i16).unsigned_abs() <= 2),
                        "Render blinked on frame {} while hovering {:?}",
                        frame,
                        h.hovered_classes()
                    );
                }
                // Only need to test one hover position
                eprintln!("Render stable while hovering {:?}", h.hovered_classes());
                return;
            }
        }
    }
}
