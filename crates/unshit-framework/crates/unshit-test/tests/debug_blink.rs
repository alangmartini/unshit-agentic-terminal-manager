/// Debug: identify which elements are at the blink coordinates.
use unshit_core::element::*;
use unshit_test::TestHarness;

// Same tree and CSS from replay_recorded.rs
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

const CSS: &str = r#"
    .root { display: flex; flex-direction: row; width: 100%; height: 100%; background: rgba(13, 17, 23, 0.95); }
    .sidebar { display: flex; flex-direction: column; width: 220px; flex-shrink: 0; background: rgba(11, 15, 20, 0.85); }
    .sidebar-header { display: flex; align-items: center; padding: 14px 16px; gap: 8px; }
    .sidebar-title { color: #e6edf3; font-size: 14px; font-weight: bold; }
    .session-item { display: flex; flex-direction: column; padding: 10px 16px; gap: 2px; cursor: pointer; }
    .session-active { display: flex; flex-direction: column; padding: 10px 16px; gap: 2px; background: rgba(16, 185, 129, 0.12); }
    .session-row { display: flex; align-items: center; gap: 10px; }
    .session-number { color: #484f58; font-size: 13px; }
    .session-name { color: #8b949e; font-size: 14px; }
    .session-name-hl { color: #10b981; font-size: 14px; font-weight: bold; }
    .divider { width: 1px; background: rgba(16, 185, 129, 0.15); flex-shrink: 0; }
    .main { display: flex; flex-direction: column; flex-grow: 1; padding: 20px 28px; gap: 4px; }
    .text-body { color: #e6edf3; font-size: 14px; line-height: 1.5; }
    .code-badge { display: flex; align-items: center; padding: 1px 6px; background: rgba(16, 185, 129, 0.1); border-radius: 4px; color: #34d399; font-size: 14px; }
    .link { color: #58a6ff; font-size: 14px; cursor: pointer; }
    .session-item:hover { background: rgba(16, 185, 129, 0.06); }
    .link:hover { color: #79b8ff; }
    .code-badge:hover { background: rgba(16, 185, 129, 0.18); color: #6ee7b7; }
"#;

#[test]
fn debug_blink_location() {
    let mut h = TestHarness::new(CSS, claude_code_tree, 1100.0, 750.0);
    h.set_scale_factor(1.5);
    h.step();

    // Print all element layout rects
    eprintln!("\n=== Element layout at 1.5x scale ===");
    for (node_id, element) in h.arena().iter() {
        let r = element.layout_rect;
        let classes: Vec<_> = element.classes.iter().map(std::string::String::as_str).collect();
        let id = element.id.as_deref().unwrap_or("");
        if r.width > 0.0 {
            eprintln!(
                "  {:?} tag={} classes={:?} id={:?} rect=({:.0}, {:.0}, {:.0}x{:.0})",
                node_id,
                element.tag.name(),
                classes,
                id,
                r.x,
                r.y,
                r.width,
                r.height
            );
        }
    }

    // Test the exact blink coordinates
    let blink_coords = [
        (301.0, 254.0, "frame 378"),
        (313.0, 253.0, "frame 379"),
        (331.0, 253.0, "frame 380"),
        (359.0, 251.0, "frame 381 - BLINK"),
        (400.0, 248.0, "frame 382"),
        (454.0, 246.0, "frame 383"),
    ];

    eprintln!("\n=== Hit test at blink coordinates ===");
    for (x, y, label) in &blink_coords {
        h.mouse_move(*x, *y);
        h.step();
        let hovered = h.hovered();
        let classes = h.hovered_classes();
        eprintln!("  ({}, {}) [{}]: hovered={:?} classes={:?}", x, y, label, hovered, classes);
    }
}
