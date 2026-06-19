use std::sync::{Arc, Mutex};
use unshit_core::element::*;
use unshit_core::event::Key;
use unshit_test::TestHarness;

const CSS: &str = r#"
    .root { width: 100%; height: 100%; flex-direction: column; }
    .my-input { width: 300px; height: 40px; padding: 8px; font-size: 14px; }
    .other { width: 100px; height: 40px; }
"#;

fn make_tree() -> ElementTree {
    ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(
                ElementDef::new(Tag::Input).with_class("my-input").with_placeholder("Type here..."),
            )
            .with_child(ElementDef::new(Tag::Button).with_class("other").with_text("OK")),
    }
}

fn focused_harness() -> TestHarness {
    let mut h = TestHarness::new(CSS, make_tree, 800.0, 600.0);
    h.step();
    // Click inside the input to focus it
    let input = h.query(".my-input").unwrap();
    let x = input.layout_rect.x + 10.0;
    let y = input.layout_rect.y + 10.0;
    h.click(x, y);
    h.step();
    h
}

#[test]
fn type_characters() {
    let mut h = focused_harness();
    h.type_text("hello");
    h.step();
    assert_eq!(h.input_value(), Some("hello".to_string()));
    assert_eq!(h.input_cursor_pos(), Some(5));
}

#[test]
fn backspace_deletes() {
    let mut h = focused_harness();
    h.type_text("hello");
    h.press_key(Key::Backspace);
    h.step();
    assert_eq!(h.input_value(), Some("hell".to_string()));
    assert_eq!(h.input_cursor_pos(), Some(4));
}

#[test]
fn delete_key() {
    let mut h = focused_harness();
    h.type_text("hello");
    h.press_key(Key::Home);
    h.press_key(Key::Delete);
    h.step();
    assert_eq!(h.input_value(), Some("ello".to_string()));
    assert_eq!(h.input_cursor_pos(), Some(0));
}

#[test]
fn arrow_navigation() {
    let mut h = focused_harness();
    h.type_text("abc");
    h.press_key(Key::ArrowLeft);
    h.press_key(Key::ArrowLeft);
    assert_eq!(h.input_cursor_pos(), Some(1));
    h.press_key(Key::ArrowRight);
    assert_eq!(h.input_cursor_pos(), Some(2));
}

#[test]
fn home_end() {
    let mut h = focused_harness();
    h.type_text("abc");
    h.press_key(Key::Home);
    assert_eq!(h.input_cursor_pos(), Some(0));
    h.press_key(Key::End);
    assert_eq!(h.input_cursor_pos(), Some(3));
}

#[test]
fn on_change_fires() {
    let changes: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let changes_clone = changes.clone();

    let mut h = TestHarness::new(
        CSS,
        move || ElementTree {
            root: ElementDef::new(Tag::Div).with_class("root").with_child({
                let cc = changes_clone.clone();
                ElementDef::new(Tag::Input).with_class("my-input").on_change(move |val: &str| {
                    cc.lock().unwrap().push(val.to_string());
                })
            }),
        },
        800.0,
        600.0,
    );
    h.step();

    let input = h.query(".my-input").unwrap();
    h.click(input.layout_rect.x + 10.0, input.layout_rect.y + 10.0);
    h.step();

    h.type_text("hi");
    let log = changes.lock().unwrap();
    assert_eq!(log.len(), 2);
    assert_eq!(log[0], "h");
    assert_eq!(log[1], "hi");
}

#[test]
fn on_submit_fires() {
    let submitted: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let submitted_clone = submitted.clone();

    let mut h = TestHarness::new(
        CSS,
        move || ElementTree {
            root: ElementDef::new(Tag::Div).with_class("root").with_child({
                let sc = submitted_clone.clone();
                ElementDef::new(Tag::Input).with_class("my-input").on_submit(move |val: &str| {
                    sc.lock().unwrap().push(val.to_string());
                })
            }),
        },
        800.0,
        600.0,
    );
    h.step();

    let input = h.query(".my-input").unwrap();
    h.click(input.layout_rect.x + 10.0, input.layout_rect.y + 10.0);
    h.step();

    h.type_text("hello");
    h.press_key(Key::Enter);

    let log = submitted.lock().unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0], "hello");
}

#[test]
fn seeds_initial_value_and_autofocuses() {
    let mut h = TestHarness::new(
        CSS,
        || ElementTree {
            root: ElementDef::new(Tag::Div).with_class("root").with_child(
                ElementDef::new(Tag::Input)
                    .with_class("my-input")
                    .with_value("session-1")
                    .with_autofocus(true),
            ),
        },
        800.0,
        600.0,
    );
    h.step();

    // The buffer is seeded with the current name, cursor at the end, ready
    // to type into immediately without a click.
    let input = h.query(".my-input").unwrap();
    assert_eq!(input.input_value.as_deref(), Some("session-1"));
    assert_eq!(h.focused(), input.node_id);

    // Typing appends at the seeded cursor position (end of the value).
    h.type_text("!");
    h.step();
    assert_eq!(h.input_value(), Some("session-1!".to_string()));
}

#[test]
fn placeholder_in_snapshot() {
    let mut h = TestHarness::new(CSS, make_tree, 800.0, 600.0);
    h.step();
    let snap = h.query(".my-input").unwrap();
    assert_eq!(snap.placeholder.as_deref(), Some("Type here..."));
    assert_eq!(snap.input_value.as_deref(), Some(""));
}

#[test]
fn reconciliation_preserves_input_state() {
    let mut h = focused_harness();
    h.type_text("hello");
    h.step();
    assert_eq!(h.input_value(), Some("hello".to_string()));

    // Rebuild the tree (simulating a state change)
    h.rebuild(make_tree);

    // Input state should survive
    let snap = h.query(".my-input").unwrap();
    assert_eq!(snap.input_value.as_deref(), Some("hello"));
}

#[test]
fn tab_moves_focus_out() {
    let mut h = focused_harness();
    // Verify input is focused
    let input = h.query(".my-input").unwrap();
    assert_eq!(h.focused(), input.node_id);

    // Tab should move focus to the button
    h.tab();
    h.step();

    let button = h.query(".other").unwrap();
    assert_eq!(h.focused(), button.node_id);
}

#[test]
fn unicode_input() {
    let mut h = focused_harness();
    h.type_text("\u{00e9}"); // e-acute (2 bytes in UTF-8)
    assert_eq!(h.input_value(), Some("\u{00e9}".to_string()));
    assert_eq!(h.input_cursor_pos(), Some(2)); // 2 bytes

    h.press_key(Key::ArrowLeft);
    assert_eq!(h.input_cursor_pos(), Some(0));

    h.press_key(Key::ArrowRight);
    assert_eq!(h.input_cursor_pos(), Some(2));
}

#[test]
fn insert_in_middle() {
    let mut h = focused_harness();
    h.type_text("hllo");
    // Move cursor to position 1 (after 'h')
    h.press_key(Key::Home);
    h.press_key(Key::ArrowRight);
    assert_eq!(h.input_cursor_pos(), Some(1));

    h.type_char('e');
    assert_eq!(h.input_value(), Some("hello".to_string()));
    assert_eq!(h.input_cursor_pos(), Some(2));
}
