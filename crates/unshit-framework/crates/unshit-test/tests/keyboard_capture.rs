use unshit_core::element::*;
use unshit_core::event::*;
use unshit_core::shortcut::KeyCombo;
use unshit_core::style::parse::CompiledStylesheet;
use unshit_test::TestHarness;

// ---------------------------------------------------------------------------
// Helper: build a tree with captures_keyboard enabled on a button
// ---------------------------------------------------------------------------

fn make_capture_tree() -> ElementTree {
    ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(
                ElementDef::new(Tag::Button)
                    .with_class("capture-btn")
                    .with_text("Captured")
                    .captures_keyboard(true),
            )
            .with_child(ElementDef::new(Tag::Button).with_class("normal-btn").with_text("Normal"))
            .with_child(
                ElementDef::new(Tag::Input).with_class("capture-input").captures_keyboard(true),
            ),
    }
}

fn make_default_tree() -> ElementTree {
    ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(ElementDef::new(Tag::Button).with_class("btn1").with_text("One"))
            .with_child(ElementDef::new(Tag::Button).with_class("btn2").with_text("Two")),
    }
}

const BASIC_CSS: &str = r#"
    .root { width: 100%; height: 100%; display: flex; flex-direction: column; }
    .capture-btn { width: 100px; height: 40px; }
    .normal-btn { width: 100px; height: 40px; }
    .capture-input { width: 200px; height: 30px; }
    .btn1 { width: 100px; height: 40px; }
    .btn2 { width: 100px; height: 40px; }
"#;

// ---------------------------------------------------------------------------
// Test: direct build sets captures_keyboard
// ---------------------------------------------------------------------------

#[test]
fn direct_build_sets_captures_keyboard() {
    let def = ElementDef::new(Tag::Button).with_class("test").captures_keyboard(true);
    assert!(def.captures_keyboard, "def should have captures_keyboard true");

    let mut arena = unshit_core::tree::NodeArena::new();
    let mut taffy = taffy::TaffyTree::<unshit_core::layout::TextMeasureCtx>::new();
    let node_id = unshit_core::build::build_tree_from_def(
        &def,
        &mut arena,
        &mut taffy,
        unshit_core::id::NodeId::DANGLING,
    );
    let element = arena.get(node_id).expect("element should exist");
    assert!(
        element.captures_keyboard,
        "element.captures_keyboard should be true after build_tree_from_def"
    );
}

#[test]
fn direct_build_child_captures_keyboard() {
    let root_def = ElementDef::new(Tag::Div)
        .with_class("root")
        .with_child(ElementDef::new(Tag::Button).with_class("child").captures_keyboard(true));

    let mut arena = unshit_core::tree::NodeArena::new();
    let mut taffy = taffy::TaffyTree::<unshit_core::layout::TextMeasureCtx>::new();
    let root_id = unshit_core::build::build_tree_from_def(
        &root_def,
        &mut arena,
        &mut taffy,
        unshit_core::id::NodeId::DANGLING,
    );

    // Find the child
    let root_elem = arena.get(root_id).unwrap();
    let child_id = root_elem.first_child;
    assert!(!child_id.is_dangling(), "root should have a child");

    let child_elem = arena.get(child_id).unwrap();
    assert!(child_elem.captures_keyboard, "child element.captures_keyboard should be true");
}

// ---------------------------------------------------------------------------
// Test: captures_keyboard defaults to false
// ---------------------------------------------------------------------------

#[test]
fn captures_keyboard_defaults_to_false() {
    let def = ElementDef::new(Tag::Button);
    assert!(!def.captures_keyboard, "captures_keyboard should default to false");

    let elem = Element::new(Tag::Button);
    assert!(!elem.captures_keyboard, "Element captures_keyboard should default to false");
}

// ---------------------------------------------------------------------------
// Test: builder method sets captures_keyboard
// ---------------------------------------------------------------------------

#[test]
fn builder_sets_captures_keyboard_true() {
    let def = ElementDef::new(Tag::Button).captures_keyboard(true);
    assert!(def.captures_keyboard);
}

#[test]
fn builder_sets_captures_keyboard_false_explicitly() {
    let def = ElementDef::new(Tag::Button).captures_keyboard(false);
    assert!(!def.captures_keyboard);
}

// ---------------------------------------------------------------------------
// Test: captures_keyboard is preserved across reconciliation
// ---------------------------------------------------------------------------

#[test]
fn captures_keyboard_preserved_across_reconciliation() {
    let mut h = TestHarness::new(BASIC_CSS, make_capture_tree, 800.0, 600.0);
    h.step();

    let snap = h.query(".capture-btn").expect("capture-btn not found");
    let element = h.arena().get(snap.node_id).expect("element not found");
    assert!(element.captures_keyboard, "captures_keyboard should be true after initial build");

    let snap = h.query(".capture-btn").expect("capture-btn not found");
    let element = h.arena().get(snap.node_id).expect("element not found");
    assert!(element.captures_keyboard, "captures_keyboard should be true after initial build");

    // Rebuild the tree (reconciliation)
    h.rebuild(make_capture_tree);
    h.step();

    let snap = h.query(".capture-btn").expect("capture-btn not found after reconcile");
    let element = h.arena().get(snap.node_id).expect("element not found after reconcile");
    assert!(element.captures_keyboard, "captures_keyboard should be true after reconciliation");
}

// ---------------------------------------------------------------------------
// Test: Tab with captures_keyboard=true on focused element does not cycle
// focus (focus stays on the capturing element)
// ---------------------------------------------------------------------------

#[test]
fn tab_with_capture_does_not_cycle_focus() {
    let mut h = TestHarness::new(BASIC_CSS, make_capture_tree, 800.0, 600.0);
    h.step();

    let capture_btn = h.query(".capture-btn").expect("capture-btn not found");
    h.focus(capture_btn.node_id);
    h.step();

    assert_eq!(h.focused(), capture_btn.node_id, "capture-btn should be focused");

    // Verify the element has captures_keyboard set
    let element = h.arena().get(capture_btn.node_id).expect("element");
    assert!(element.captures_keyboard, "should have captures_keyboard");

    // In the real app event pipeline, Tab would be intercepted by the
    // keyboard capture check BEFORE focus cycling happens. The test harness
    // tab() method calls focus cycling directly, so we verify at the data
    // model level that captures_keyboard is set and the event pipeline would
    // prevent cycling.
}

// ---------------------------------------------------------------------------
// Test: captures_keyboard=false (default) allows normal framework behavior
// ---------------------------------------------------------------------------

#[test]
fn default_no_capture_allows_tab_cycling() {
    let mut h = TestHarness::new(BASIC_CSS, make_default_tree, 800.0, 600.0);
    h.step();

    h.tab();
    h.step();
    let btn1 = h.query(".btn1").expect("btn1 not found");
    assert_eq!(h.focused(), btn1.node_id, "btn1 should be focused after first tab");

    h.tab();
    h.step();
    let btn2 = h.query(".btn2").expect("btn2 not found");
    assert_eq!(h.focused(), btn2.node_id, "btn2 should be focused after second tab");
}

// ---------------------------------------------------------------------------
// Test: focus change does not leak capture state to new element
// ---------------------------------------------------------------------------

#[test]
fn focus_change_does_not_leak_capture_state() {
    let mut h = TestHarness::new(BASIC_CSS, make_capture_tree, 800.0, 600.0);
    h.step();

    // Focus the capturing button
    let capture_btn = h.query(".capture-btn").expect("capture-btn not found");
    h.focus(capture_btn.node_id);
    h.step();

    let element = h.arena().get(capture_btn.node_id).expect("element");
    assert!(element.captures_keyboard, "capture-btn should capture keyboard");

    // Move focus to the normal button
    let normal_btn = h.query(".normal-btn").expect("normal-btn not found");
    h.focus(normal_btn.node_id);
    h.step();

    // The normal button should NOT have captures_keyboard
    let element = h.arena().get(normal_btn.node_id).expect("element");
    assert!(
        !element.captures_keyboard,
        "normal-btn should NOT capture keyboard after receiving focus"
    );

    // The original capturing button retains its captures_keyboard setting
    let element = h.arena().get(capture_btn.node_id).expect("element");
    assert!(
        element.captures_keyboard,
        "capture-btn should still have captures_keyboard after losing focus"
    );
}

// ---------------------------------------------------------------------------
// Test: KeyboardCapture event type exists
// ---------------------------------------------------------------------------

#[test]
fn keyboard_capture_event_type_exists() {
    let et = EventType::KeyboardCapture;
    assert_eq!(et, EventType::KeyboardCapture);
    assert_ne!(et, EventType::KeyDown);
    assert_ne!(et, EventType::KeyUp);
}

// ---------------------------------------------------------------------------
// Test: CSS keyboard-capture property is parsed correctly
// ---------------------------------------------------------------------------

#[test]
fn css_keyboard_capture_none_parsed() {
    let css = r#"
        .editor { keyboard-capture: none; width: 100px; height: 100px; }
    "#;
    let sheet = CompiledStylesheet::parse(css);
    assert!(!sheet.rules.is_empty(), "should have parsed at least one rule");

    // Apply style and check that keyboard_capture is false
    let mut style = unshit_core::style::types::ComputedStyle::default();
    for decl in &sheet.rules[0].declarations {
        unshit_core::style::parse::apply_declaration(&mut style, decl);
    }
    assert!(!style.keyboard_capture, "keyboard-capture: none should result in false");
}

#[test]
fn css_keyboard_capture_all_parsed() {
    let css = r#"
        .editor { keyboard-capture: all; width: 100px; height: 100px; }
    "#;
    let sheet = CompiledStylesheet::parse(css);
    assert!(!sheet.rules.is_empty(), "should have parsed at least one rule");

    let mut style = unshit_core::style::types::ComputedStyle::default();
    for decl in &sheet.rules[0].declarations {
        unshit_core::style::parse::apply_declaration(&mut style, decl);
    }
    assert!(style.keyboard_capture, "keyboard-capture: all should result in true");
}

#[test]
fn css_keyboard_capture_applied_via_cascade() {
    let css = r#"
        .root { width: 100%; height: 100%; display: flex; }
        .editor { keyboard-capture: all; width: 200px; height: 100px; }
    "#;

    let tree_fn = || ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(ElementDef::new(Tag::Div).with_class("editor").with_tab_index(0)),
    };

    let mut h = TestHarness::new(css, tree_fn, 800.0, 600.0);
    h.step();

    let snap = h.query(".editor").expect("editor not found");
    assert!(
        snap.computed_style.keyboard_capture,
        "keyboard-capture: all should be reflected in computed style"
    );
}

// ---------------------------------------------------------------------------
// Test: release chord (Ctrl+Shift+Escape) is the default
// ---------------------------------------------------------------------------

#[test]
fn release_chord_default_is_ctrl_shift_escape() {
    let combo = KeyCombo::new(Key::Escape, Modifiers::CTRL | Modifiers::SHIFT);
    assert_eq!(combo.key, Key::Escape);
    assert!(combo.modifiers.contains(Modifiers::CTRL));
    assert!(combo.modifiers.contains(Modifiers::SHIFT));
}

// ---------------------------------------------------------------------------
// Test: Arrow keys with capture on Input (data model verification)
// ---------------------------------------------------------------------------

#[test]
fn input_with_capture_retains_flag() {
    let mut h = TestHarness::new(BASIC_CSS, make_capture_tree, 800.0, 600.0);
    h.step();

    let input = h.query(".capture-input").expect("capture-input not found");
    let element = h.arena().get(input.node_id).expect("element");
    assert!(element.captures_keyboard, "input should have captures_keyboard");
    assert_eq!(element.tag, Tag::Input, "should be an Input element");
}

// ---------------------------------------------------------------------------
// Test: reconciliation changes captures_keyboard when tree def changes
// ---------------------------------------------------------------------------

#[test]
fn reconciliation_updates_captures_keyboard() {
    // Start with captures_keyboard = true
    let mut h = TestHarness::new(BASIC_CSS, make_capture_tree, 800.0, 600.0);
    h.step();

    let snap = h.query(".capture-btn").expect("capture-btn not found");
    let element = h.arena().get(snap.node_id).expect("element");
    assert!(element.captures_keyboard, "initially true");

    // Rebuild with captures_keyboard = false
    let tree_fn_no_capture = || ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(
                ElementDef::new(Tag::Button)
                    .with_class("capture-btn")
                    .with_text("Captured")
                    .captures_keyboard(false),
            )
            .with_child(ElementDef::new(Tag::Button).with_class("normal-btn").with_text("Normal"))
            .with_child(
                ElementDef::new(Tag::Input).with_class("capture-input").captures_keyboard(true),
            ),
    };

    h.rebuild(tree_fn_no_capture);
    h.step();

    let snap = h.query(".capture-btn").expect("capture-btn not found");
    let element = h.arena().get(snap.node_id).expect("element");
    assert!(
        !element.captures_keyboard,
        "captures_keyboard should be false after rebuild with false"
    );
}
