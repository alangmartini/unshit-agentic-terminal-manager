use std::sync::{Arc, Mutex};
use unshit_core::element::*;
use unshit_test::TestHarness;

const CSS: &str = r#"
    .root { width: 100%; height: 100%; flex-direction: column; padding: 10px; }
    .my-select { width: 200px; height: 36px; padding: 4px; font-size: 14px; }
    .other { width: 100px; height: 36px; }
"#;

fn make_options() -> Vec<(String, String)> {
    vec![
        ("a".to_string(), "Alpha".to_string()),
        ("b".to_string(), "Beta".to_string()),
        ("c".to_string(), "Gamma".to_string()),
    ]
}

fn make_tree() -> ElementTree {
    ElementTree {
        root: ElementDef::new(Tag::Div).with_class("root").with_child(
            ElementDef::new(Tag::Select).with_class("my-select").with_options(make_options()),
        ),
    }
}

#[test]
fn select_renders_first_option_as_selected() {
    let mut h = TestHarness::new(CSS, make_tree, 400.0, 300.0);
    h.step();

    let snap = h.query("select").unwrap();
    // Select must exist in the arena
    assert_eq!(snap.tag, Tag::Select);

    // SelectState must be initialized with options and selected_index = 0
    let node_id = snap.node_id;
    let ss = h.select_state(node_id).unwrap();
    assert_eq!(ss.selected_index, 0);
    assert_eq!(ss.options.len(), 3);
    assert_eq!(ss.options[0].label, "Alpha");
    assert!(!ss.open);
}

#[test]
fn select_click_opens_dropdown() {
    let mut h = TestHarness::new(CSS, make_tree, 400.0, 300.0);
    h.step();

    let snap = h.query("select").unwrap();
    let node_id = snap.node_id;

    assert!(!h.select_is_open(node_id));

    h.click_select(node_id);

    assert!(h.select_is_open(node_id));
    // highlighted_index should be set to selected_index (0)
    assert_eq!(h.select_state(node_id).unwrap().highlighted_index, Some(0));
}

#[test]
fn select_click_option_selects_and_closes() {
    let mut h = TestHarness::new(CSS, make_tree, 400.0, 300.0);
    h.step();

    let snap = h.query("select").unwrap();
    let node_id = snap.node_id;

    // Open dropdown
    h.click_select(node_id);
    assert!(h.select_is_open(node_id));

    // Click on second option (index 1)
    h.click_select_option(node_id, 1);

    assert_eq!(h.select_selected_index(node_id), Some(1));
    assert_eq!(h.select_selected_value(node_id), Some("b".to_string()));
    assert!(!h.select_is_open(node_id));
}

#[test]
fn select_on_change_fires_with_correct_value() {
    let fired_values: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let fired_clone = fired_values.clone();

    let tree_fn = move || ElementTree {
        root: ElementDef::new(Tag::Div).with_class("root").with_child(
            ElementDef::new(Tag::Select)
                .with_class("my-select")
                .with_options(make_options())
                .on_change({
                    let c = fired_clone.clone();
                    move |v| {
                        c.lock().unwrap().push(v.to_string());
                    }
                }),
        ),
    };

    let mut h = TestHarness::new(CSS, tree_fn, 400.0, 300.0);
    h.step();

    let snap = h.query("select").unwrap();
    let node_id = snap.node_id;

    // Choose the third option
    h.select_choose(node_id, 2);

    let values = fired_values.lock().unwrap();
    assert_eq!(values.len(), 1);
    assert_eq!(values[0], "c");
}

#[test]
fn keyboard_arrows_move_highlight() {
    let mut h = TestHarness::new(CSS, make_tree, 400.0, 300.0);
    h.step();

    let snap = h.query("select").unwrap();
    let node_id = snap.node_id;

    // Open dropdown
    h.select_open(node_id);
    assert_eq!(h.select_state(node_id).unwrap().highlighted_index, Some(0));

    // Move down
    h.select_move_highlight(node_id, 1);
    assert_eq!(h.select_state(node_id).unwrap().highlighted_index, Some(1));

    // Move down again
    h.select_move_highlight(node_id, 1);
    assert_eq!(h.select_state(node_id).unwrap().highlighted_index, Some(2));

    // Cannot move past last item
    h.select_move_highlight(node_id, 1);
    assert_eq!(h.select_state(node_id).unwrap().highlighted_index, Some(2));

    // Move up
    h.select_move_highlight(node_id, -1);
    assert_eq!(h.select_state(node_id).unwrap().highlighted_index, Some(1));
}

#[test]
fn keyboard_enter_selects_highlighted() {
    let fired: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let fired_clone = fired.clone();

    let tree_fn = move || ElementTree {
        root: ElementDef::new(Tag::Div).with_class("root").with_child(
            ElementDef::new(Tag::Select)
                .with_class("my-select")
                .with_options(make_options())
                .on_change({
                    let c = fired_clone.clone();
                    move |v| {
                        c.lock().unwrap().push(v.to_string());
                    }
                }),
        ),
    };

    let mut h = TestHarness::new(CSS, tree_fn, 400.0, 300.0);
    h.step();

    let snap = h.query("select").unwrap();
    let node_id = snap.node_id;

    // Open, move to second item, confirm
    h.select_open(node_id);
    h.select_move_highlight(node_id, 1);
    h.select_confirm_highlight(node_id);

    assert_eq!(h.select_selected_index(node_id), Some(1));
    assert!(!h.select_is_open(node_id));

    let values = fired.lock().unwrap();
    assert_eq!(values.len(), 1);
    assert_eq!(values[0], "b");
}

#[test]
fn escape_closes_without_changing_selection() {
    let mut h = TestHarness::new(CSS, make_tree, 400.0, 300.0);
    h.step();

    let snap = h.query("select").unwrap();
    let node_id = snap.node_id;

    // Select second option first
    h.select_choose(node_id, 1);
    assert_eq!(h.select_selected_index(node_id), Some(1));

    // Open and move highlight to third item
    h.select_open(node_id);
    h.select_move_highlight(node_id, 1); // highlight is now at index 2

    // Close without selecting
    h.select_close(node_id);

    // Selection should remain at 1, not 2
    assert_eq!(h.select_selected_index(node_id), Some(1));
    assert!(!h.select_is_open(node_id));
}

#[test]
fn tab_focus_works() {
    let css = r#"
        .root { width: 100%; height: 100%; flex-direction: column; }
        .btn { width: 100px; height: 36px; }
        .sel { width: 200px; height: 36px; }
    "#;
    let tree_fn = || ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(ElementDef::new(Tag::Button).with_class("btn").with_text("Go"))
            .with_child(
                ElementDef::new(Tag::Select).with_class("sel").with_options(make_options()),
            ),
    };

    let mut h = TestHarness::new(css, tree_fn, 400.0, 300.0);
    h.step();

    // Tab from no focus -> first focusable (button)
    h.tab();
    let focused = h.focused();
    let tag = h.arena().get(focused).map(|e| e.tag);
    assert_eq!(tag, Some(Tag::Button));

    // Tab again -> select
    h.tab();
    let focused = h.focused();
    let tag = h.arena().get(focused).map(|e| e.tag);
    assert_eq!(tag, Some(Tag::Select));
}

#[test]
fn options_update_on_reconciliation_preserves_selection() {
    // Phase 1: build with original options, select option at index 1
    let mut h = TestHarness::new(CSS, make_tree, 400.0, 300.0);
    h.step();

    let snap = h.query("select").unwrap();
    let node_id = snap.node_id;

    h.select_choose(node_id, 1);
    assert_eq!(h.select_selected_index(node_id), Some(1));

    // Phase 2: rebuild with updated options (different labels/values)
    let new_opts =
        vec![("x".to_string(), "X-ray".to_string()), ("y".to_string(), "Yankee".to_string())];
    h.rebuild(move || ElementTree {
        root: ElementDef::new(Tag::Div).with_class("root").with_child(
            ElementDef::new(Tag::Select).with_class("my-select").with_options(new_opts.clone()),
        ),
    });
    h.step();

    // Selected index should be preserved (reconciler preserves existing state)
    assert_eq!(h.select_selected_index(node_id), Some(1));
    // Options list should be updated to the new set
    let ss = h.select_state(node_id).unwrap();
    assert_eq!(ss.options.len(), 2);
    assert_eq!(ss.options[0].label, "X-ray");
}

#[test]
fn select_no_arena_children() {
    // Option children must NOT appear as arena nodes
    let mut h = TestHarness::new(CSS, make_tree, 400.0, 300.0);
    h.step();

    let option_nodes: Vec<_> = h.arena().iter().filter(|(_, el)| el.tag == Tag::Option).collect();

    assert!(
        option_nodes.is_empty(),
        "Option elements should not be stored as arena nodes, found: {}",
        option_nodes.len()
    );
}

#[test]
fn select_with_initial_selected_index() {
    let tree_fn = || ElementTree {
        root: ElementDef::new(Tag::Div).with_class("root").with_child(
            ElementDef::new(Tag::Select)
                .with_class("my-select")
                .with_options(make_options())
                .with_selected_index(2),
        ),
    };

    let mut h = TestHarness::new(CSS, tree_fn, 400.0, 300.0);
    h.step();

    let snap = h.query("select").unwrap();
    let ss = h.select_state(snap.node_id).unwrap();
    assert_eq!(ss.selected_index, 2);
    assert_eq!(h.select_selected_value(snap.node_id), Some("c".to_string()));
}
