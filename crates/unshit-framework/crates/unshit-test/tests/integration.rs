//! End-to-end integration tests that exercise the full unshit-test framework
//! as a real user would: building apps, finding elements, performing actions,
//! and asserting on state changes.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use unshit_core::element::*;
use unshit_test::{TestApp, TestHarness, TraceAction};

// ---------------------------------------------------------------------------
// Shared CSS
// ---------------------------------------------------------------------------

const APP_CSS: &str = "
    .app { width: 100%; height: 100%; display: flex; flex-direction: column; }
    .header { width: 100%; height: 40px; }
    .counter-display { width: 200px; height: 40px; }
    .btn { width: 100px; height: 40px; }
    .card { width: 200px; height: 120px; padding: 10px; }
    .card .title { font-size: 16px; }
    .card .body { font-size: 14px; }
    .card .action { width: 80px; height: 30px; }
    .form { display: flex; flex-direction: column; width: 300px; }
    .field { width: 280px; height: 30px; }
    .checkbox { width: 20px; height: 20px; }
    .select { width: 200px; height: 30px; }
    .hoverable { width: 100px; height: 100px; }
    .hoverable:hover { background: red; }
    input { width: 200px; height: 30px; }
";

// ===========================================================================
// 1. Full user flow: counter app
// ===========================================================================

fn counter_tree(
    count: &Arc<AtomicU32>,
    inc: &Arc<dyn Fn() + Send + Sync>,
    dec: &Arc<dyn Fn() + Send + Sync>,
) -> ElementTree {
    let val = count.load(Ordering::SeqCst);
    let inc = inc.clone();
    let dec = dec.clone();
    ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("app")
            .with_child(
                ElementDef::new(Tag::Span)
                    .with_class("counter-display")
                    .with_text(format!("Count: {}", val)),
            )
            .with_child(
                ElementDef::new(Tag::Button)
                    .with_class("btn")
                    .with_class("inc")
                    .with_text("+")
                    .on_click(move || inc()),
            )
            .with_child(
                ElementDef::new(Tag::Button)
                    .with_class("btn")
                    .with_class("dec")
                    .with_text("-")
                    .on_click(move || dec()),
            ),
    }
}

#[test]
fn full_counter_flow() {
    let click_count = Arc::new(AtomicU32::new(0));

    let count_for_inc = click_count.clone();
    let inc_cb: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
        count_for_inc.fetch_add(1, Ordering::SeqCst);
    });

    let count_for_dec = click_count.clone();
    let dec_cb: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
        let current = count_for_dec.load(Ordering::SeqCst);
        if current > 0 {
            count_for_dec.fetch_sub(1, Ordering::SeqCst);
        }
    });

    let c = click_count.clone();
    let i = inc_cb.clone();
    let d = dec_cb.clone();
    let mut h = TestHarness::new(
        APP_CSS,
        move || counter_tree(&c, &i, &d),
        400.0,
        300.0,
    );

    h.expect_text(".counter-display", "Count: 0");
    h.expect_visible(".btn.inc");
    h.expect_visible(".btn.dec");

    h.click_on(".btn.inc");
    assert_eq!(click_count.load(Ordering::SeqCst), 1);

    let c = click_count.clone();
    let i = inc_cb.clone();
    let d = dec_cb.clone();
    h.rebuild(move || counter_tree(&c, &i, &d));

    h.expect_text(".counter-display", "Count: 1");

    h.locator(".btn.inc").click();
    assert_eq!(click_count.load(Ordering::SeqCst), 2);
}

// ===========================================================================
// 2. Form interaction
// ===========================================================================

#[test]
fn form_interaction_fill_and_assert() {
    let mut h = TestHarness::new(
        APP_CSS,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("form")
                .with_child(
                    ElementDef::new(Tag::Input)
                        .with_class("field")
                        .with_class("name")
                        .with_tab_index(1)
                        .with_placeholder("Name"),
                )
                .with_child(
                    ElementDef::new(Tag::Input)
                        .with_class("field")
                        .with_class("email")
                        .with_tab_index(2)
                        .with_placeholder("Email"),
                )
                .with_child(
                    ElementDef::new(Tag::Input)
                        .with_class("checkbox")
                        .with_class("agree")
                        .with_input_type(InputType::Checkbox)
                        .with_tab_index(3),
                )
                .with_child(
                    ElementDef::new(Tag::Select)
                        .with_class("select")
                        .with_class("role")
                        .with_options(vec![
                            ("admin".into(), "Admin".into()),
                            ("user".into(), "User".into()),
                            ("guest".into(), "Guest".into()),
                        ])
                        .with_selected_index(0),
                ),
        },
        400.0,
        400.0,
    );

    // Fill text inputs using locator API
    h.locator(".field.name").fill("Alice");
    h.locator(".field.name").expect_value("Alice");

    h.locator(".field.email").fill("alice@example.com");
    h.locator(".field.email").expect_value("alice@example.com");

    // Fill using the action API (selector-based)
    h.clear(".field.name");
    h.fill(".field.name", "Bob");
    h.expect_value(".field.name", "Bob");

    // Check checkbox
    h.click_on("input[type=\"checkbox\"]");
    h.expect_checked("input[type=\"checkbox\"]");

    // Uncheck checkbox
    h.click_on("input[type=\"checkbox\"]");
    h.expect_not_checked("input[type=\"checkbox\"]");

    // Select dropdown option
    h.select_option_on(".select.role", "guest");
    let role_node = h.query(".select.role").unwrap().node_id;
    let selected_val = h.select_selected_value(role_node);
    assert_eq!(selected_val, Some("guest".to_string()));
}

#[test]
fn form_tab_navigation() {
    let mut h = TestHarness::new(
        APP_CSS,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("form")
                .with_child(
                    ElementDef::new(Tag::Input)
                        .with_class("field")
                        .with_class("first")
                        .with_tab_index(1),
                )
                .with_child(
                    ElementDef::new(Tag::Input)
                        .with_class("field")
                        .with_class("second")
                        .with_tab_index(2),
                )
                .with_child(
                    ElementDef::new(Tag::Input)
                        .with_class("field")
                        .with_class("third")
                        .with_tab_index(3),
                ),
        },
        400.0,
        300.0,
    );

    // Focus the first field
    h.click_on(".field.first");
    h.expect_focused(".field.first");

    // Tab to the second
    h.tab();
    h.step();
    h.expect_focused(".field.second");

    // Tab to the third
    h.tab();
    h.step();
    h.expect_focused(".field.third");

    // Shift+Tab back to second
    h.shift_tab();
    h.step();
    h.expect_focused(".field.second");
}

// ===========================================================================
// 3. Selector + Locator chaining (nested cards)
// ===========================================================================

#[test]
fn selector_locator_chaining_nested_cards() {
    let mut h = TestHarness::new(
        APP_CSS,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("app")
                .with_child(
                    ElementDef::new(Tag::Div)
                        .with_class("card")
                        .with_child(
                            ElementDef::new(Tag::Span)
                                .with_class("title")
                                .with_text("Card A"),
                        )
                        .with_child(
                            ElementDef::new(Tag::Button)
                                .with_class("action")
                                .with_text("Edit"),
                        ),
                )
                .with_child(
                    ElementDef::new(Tag::Div)
                        .with_class("card")
                        .with_child(
                            ElementDef::new(Tag::Span)
                                .with_class("title")
                                .with_text("Card B"),
                        )
                        .with_child(
                            ElementDef::new(Tag::Button)
                                .with_class("action")
                                .with_text("Delete"),
                        ),
                )
                .with_child(
                    ElementDef::new(Tag::Div)
                        .with_class("card")
                        .with_child(
                            ElementDef::new(Tag::Span)
                                .with_class("title")
                                .with_text("Card C"),
                        ),
                ),
        },
        600.0,
        500.0,
    );

    // Compound selector: descendant
    let titles = h.query_all(".card .title");
    assert_eq!(titles.len(), 3);

    // Locator chaining: card -> title
    h.locator(".card").locator(".title").expect_count(3);

    // nth + chaining: second card's title
    h.locator(".card").nth(1).locator(".title").expect_text("Card B");

    // text-based locator
    let snap = h.locator_by_text("Card A").snapshot();
    assert!(snap.classes.contains(&"title".to_string()));

    // text-contains locator
    h.locator_by_text_contains("Card").expect_count(3);

    // Element count assertions
    h.expect_count(".card", 3);
    h.expect_count(".action", 2);

    // filter_by_text on cards
    let count = h.locator(".card").filter_by_text("Card B").count();
    assert_eq!(count, 1);

    // Chaining with nth to access action button
    let action_text = h.locator(".card").nth(0).locator(".action").text();
    assert_eq!(action_text, "Edit");
}

// ===========================================================================
// 4. Hover and style state
// ===========================================================================

#[test]
fn hover_and_style_state() {
    let mut h = TestHarness::new(
        APP_CSS,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("app")
                .with_child(
                    ElementDef::new(Tag::Div)
                        .with_class("hoverable")
                        .with_class("box-a")
                        .with_text("Hover me"),
                )
                .with_child(
                    ElementDef::new(Tag::Div)
                        .with_class("hoverable")
                        .with_class("box-b")
                        .with_text("Or me"),
                ),
        },
        400.0,
        300.0,
    );

    // Hover over box-a using the action API
    h.hover_on(".hoverable.box-a");
    let classes = h.hovered_classes();
    assert!(
        classes.contains(&"box-a".to_string()),
        "hovered element should be box-a, got: {:?}",
        classes
    );

    // Hover is stable
    h.assert_hover_stable(3);

    // Move to box-b
    h.hover_on(".hoverable.box-b");
    let classes = h.hovered_classes();
    assert!(
        classes.contains(&"box-b".to_string()),
        "hovered element should be box-b, got: {:?}",
        classes
    );

    // Hover via locator
    h.locator(".hoverable.box-a").hover();
    let classes = h.hovered_classes();
    assert!(
        classes.contains(&"box-a".to_string()),
        "locator hover should target box-a, got: {:?}",
        classes
    );

    // Move away (to empty space)
    h.mouse_move(399.0, 299.0);
    h.step();
    let classes = h.hovered_classes();
    // Should no longer be on box-a or box-b (may be on root or nothing)
    assert!(
        !classes.contains(&"box-a".to_string()) && !classes.contains(&"box-b".to_string()),
        "after moving away, neither box should be hovered, got: {:?}",
        classes
    );
}

// ===========================================================================
// 5. Trace recording integration
// ===========================================================================

#[test]
fn trace_recording_captures_actions() {
    let mut h = TestHarness::new(
        APP_CSS,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("app")
                .with_child(
                    ElementDef::new(Tag::Button)
                        .with_class("btn")
                        .with_class("submit")
                        .with_text("Submit"),
                )
                .with_child(
                    ElementDef::new(Tag::Input)
                        .with_class("field")
                        .with_class("name")
                        .with_tab_index(1),
                ),
        },
        400.0,
        300.0,
    );

    // Enable trace
    h.enable_trace();
    assert!(h.trace().is_enabled());

    // Perform actions that get traced
    h.click_on(".btn.submit");
    h.hover_on(".btn.submit");
    h.fill(".field.name", "traced-value");

    // Verify trace captured all steps
    let steps = h.trace().steps();
    assert!(steps.len() >= 3, "expected at least 3 traced steps, got {}", steps.len());

    // Verify action types in order
    assert!(
        matches!(steps[0].action, TraceAction::Click { .. }),
        "first step should be Click"
    );
    assert!(
        matches!(steps[1].action, TraceAction::Hover { .. }),
        "second step should be Hover"
    );
    assert!(
        matches!(steps[2].action, TraceAction::Fill { .. }),
        "third step should be Fill"
    );

    // Verify selectors are recorded
    if let TraceAction::Click { ref selector, .. } = steps[0].action {
        assert_eq!(selector, ".btn.submit");
    }
    if let TraceAction::Fill { ref text, .. } = steps[2].action {
        assert_eq!(text, "traced-value");
    }
}

#[test]
fn trace_disabled_records_nothing() {
    let mut h = TestHarness::new(
        APP_CSS,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("app")
                .with_child(
                    ElementDef::new(Tag::Button)
                        .with_class("btn")
                        .with_text("Click"),
                ),
        },
        400.0,
        300.0,
    );

    // Trace is disabled by default
    assert!(!h.trace().is_enabled());

    h.click_on(".btn");
    assert!(h.trace().steps().is_empty(), "disabled trace should record nothing");
}

// ===========================================================================
// 6. Assertion retry after rebuild
// ===========================================================================

fn value_display_tree(counter: &Arc<AtomicU32>) -> ElementTree {
    let val = counter.load(Ordering::SeqCst);
    ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("app")
            .with_child(
                ElementDef::new(Tag::Span)
                    .with_class("counter-display")
                    .with_text(format!("Value: {}", val)),
            ),
    }
}

#[test]
fn assertion_retry_after_rebuild() {
    let counter = Arc::new(AtomicU32::new(0));

    let c = counter.clone();
    let mut h = TestHarness::new(
        APP_CSS,
        move || value_display_tree(&c),
        400.0,
        300.0,
    );

    h.expect_text(".counter-display", "Value: 0");

    counter.store(42, Ordering::SeqCst);
    let c = counter.clone();
    h.rebuild(move || value_display_tree(&c));

    h.expect_text(".counter-display", "Value: 42");
}

// ===========================================================================
// 7. TestApp integration (validates unified API)
// ===========================================================================

#[test]
fn test_app_full_flow() {
    std::env::remove_var("UNSHIT_TEST_HEADED");
    std::env::remove_var("UNSHIT_TEST_SLOW_MO");

    let mut app = TestApp::new(
        APP_CSS,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("app")
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("header")
                        .with_text("My App"),
                )
                .with_child(
                    ElementDef::new(Tag::Input)
                        .with_class("field")
                        .with_tab_index(1),
                )
                .with_child(
                    ElementDef::new(Tag::Button)
                        .with_class("btn")
                        .with_text("Go"),
                ),
        },
        400.0,
        300.0,
    );

    // Basic queries through TestApp
    assert!(app.query(".header").is_some());
    assert!(app.query(".missing").is_none());
    assert_eq!(app.query_all(".btn").len(), 1);

    // Locator through TestApp
    assert_eq!(app.locator(".btn").count(), 1);
    assert_eq!(app.locator(".btn").text(), "Go");

    // Input via TestApp
    app.click(100.0, 60.0);
    app.step();

    // State access
    assert!(!app.is_headed());
    assert!(app.as_harness().is_some());
    assert!(app.as_windowed().is_none());
}

// ===========================================================================
// 8. ui_test macro validation
// ===========================================================================

#[unshit_macros::ui_test]
fn ui_test_macro_integration() {
    std::env::remove_var("UNSHIT_TEST_HEADED");

    let mut h = TestHarness::new(
        APP_CSS,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("app")
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("title")
                        .with_text("Hello from ui_test"),
                ),
        },
        400.0,
        200.0,
    );

    h.expect_text(".title", "Hello from ui_test");
    h.expect_visible(".app");
    h.expect_exists(".title");
}

#[unshit_macros::ui_test(headed = false, slow_mo = 0)]
fn ui_test_macro_with_config_integration() {
    let h = TestHarness::new(
        APP_CSS,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("app")
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("label")
                        .with_text("configured"),
                ),
        },
        400.0,
        200.0,
    );

    let snap = h.query(".label").unwrap();
    assert_eq!(
        snap.content,
        ElementContent::Text("configured".to_string())
    );
}

// ===========================================================================
// 9. Select dropdown interaction
// ===========================================================================

#[test]
fn select_dropdown_full_flow() {
    let selected = Arc::new(std::sync::Mutex::new("small".to_string()));
    let selected_cb = selected.clone();

    let mut h = TestHarness::new(
        APP_CSS,
        move || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("app")
                .with_child(
                    ElementDef::new(Tag::Select)
                        .with_class("select")
                        .with_class("size")
                        .with_options(vec![
                            ("small".into(), "Small".into()),
                            ("medium".into(), "Medium".into()),
                            ("large".into(), "Large".into()),
                        ])
                        .with_selected_index(0)
                        .on_change({
                            let s = selected_cb.clone();
                            move |val| {
                                *s.lock().unwrap() = val.to_string();
                            }
                        }),
                ),
        },
        400.0,
        300.0,
    );

    // Initial state
    let node_id = h.query(".select.size").unwrap().node_id;
    assert_eq!(h.select_selected_value(node_id), Some("small".to_string()));
    assert_eq!(h.select_selected_index(node_id), Some(0));

    // Select by value
    h.select_option_on(".select.size", "medium");
    assert_eq!(*selected.lock().unwrap(), "medium");
    assert_eq!(h.select_selected_value(node_id), Some("medium".to_string()));

    // Select by index
    h.select_option_by_index_on(".select.size", 2);
    assert_eq!(*selected.lock().unwrap(), "large");
    assert_eq!(h.select_selected_index(node_id), Some(2));

    // Keyboard navigation
    h.click_select(node_id);
    assert!(h.select_is_open(node_id));
    h.select_move_highlight(node_id, -1); // move up
    h.select_confirm_highlight(node_id);
    assert!(!h.select_is_open(node_id));
}

// ===========================================================================
// 10. Existence and hidden assertions
// ===========================================================================

#[test]
fn existence_and_visibility_assertions() {
    let mut h = TestHarness::new(
        "
            .app { width: 100%; height: 100%; }
            .visible-box { width: 100px; height: 100px; }
            .zero-box { width: 0px; height: 0px; }
        ",
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("app")
                .with_child(
                    ElementDef::new(Tag::Div)
                        .with_class("visible-box"),
                )
                .with_child(
                    ElementDef::new(Tag::Div)
                        .with_class("zero-box"),
                ),
        },
        400.0,
        300.0,
    );

    // Exists
    h.expect_exists(".visible-box");
    h.expect_exists(".zero-box");

    // Not exists
    h.expect_not_exists(".nonexistent");

    // Visible
    h.expect_visible(".visible-box");

    // Hidden (zero dimensions)
    h.expect_hidden(".zero-box");
    h.expect_hidden(".nonexistent");
}

// ===========================================================================
// 11. Input type_text and press_key
// ===========================================================================

#[test]
fn input_type_and_key_press() {
    let submitted = Arc::new(std::sync::Mutex::new(String::new()));
    let submitted_cb = submitted.clone();

    let mut h = TestHarness::new(
        APP_CSS,
        move || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("app")
                .with_child(
                    ElementDef::new(Tag::Input)
                        .with_class("field")
                        .with_class("search")
                        .with_tab_index(1)
                        .on_submit({
                            let s = submitted_cb.clone();
                            move |val| {
                                *s.lock().unwrap() = val.to_string();
                            }
                        }),
                ),
        },
        400.0,
        200.0,
    );

    // Click to focus and type
    h.click_on(".field.search");
    h.type_text("hello world");
    assert_eq!(h.input_value(), Some("hello world".to_string()));

    // Press Enter triggers on_submit
    h.press_key(unshit_core::event::Key::Enter);
    assert_eq!(*submitted.lock().unwrap(), "hello world");

    // Clear with press_on
    h.press_on(".field.search", "Ctrl+A");
    h.expect_value(".field.search", "");
}

// ===========================================================================
// 12. Query by various selector types
// ===========================================================================

#[test]
fn query_selectors_comprehensive() {
    let mut h = TestHarness::new(
        "
            .app { width: 100%; height: 100%; display: flex; flex-direction: column; }
            .item { width: 100px; height: 30px; }
            input { width: 200px; height: 30px; }
        ",
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("app")
                .with_id("root")
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("item")
                        .with_class("first")
                        .with_text("Alpha"),
                )
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("item")
                        .with_class("second")
                        .with_text("Beta"),
                )
                .with_child(
                    ElementDef::new(Tag::Input)
                        .with_class("searchbox")
                        .with_placeholder("Search here")
                        .with_input_type(InputType::Text)
                        .with_tab_index(1),
                ),
        },
        400.0,
        300.0,
    );

    // By class
    assert!(h.query(".item").is_some());
    assert_eq!(h.query_all(".item").len(), 2);

    // By ID
    let root = h.query("#root").unwrap();
    assert_eq!(root.id.as_deref(), Some("root"));

    // By tag
    assert!(h.query("span").is_some());

    // Compound: tag + class
    assert!(h.query("span.first").is_some());

    // Descendant combinator
    assert_eq!(h.query_all(".app .item").len(), 2);

    // Child combinator
    assert_eq!(h.query_all(".app > .item").len(), 2);

    // Attribute selector
    let searchbox = h.query("[placeholder=\"Search here\"]").unwrap();
    assert!(searchbox.classes.contains(&"searchbox".to_string()));

    // Pseudo-class :first-child
    let first = h.query(":first-child").unwrap();
    assert!(first.classes.contains(&"first".to_string()));

    // Pseudo-class :last-child (the input is the actual last child, not .item.second)
    let last_child = h.query(":last-child").unwrap();
    assert!(last_child.classes.contains(&"searchbox".to_string()));

    // Text locator
    let alpha = h.locator_by_text("Alpha").snapshot();
    assert!(alpha.classes.contains(&"first".to_string()));

    // Text contains locator
    h.locator_by_text_contains("eta").expect_count(1);
}

// ===========================================================================
// 13. Rebuild re-resolves locators
// ===========================================================================

#[test]
fn rebuild_re_resolves_locators() {
    let mut h = TestHarness::new(
        APP_CSS,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("app")
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("title")
                        .with_text("Before"),
                ),
        },
        400.0,
        200.0,
    );

    h.locator(".title").expect_text("Before");
    assert_eq!(h.locator(".card").count(), 0);

    // Rebuild with different content
    h.rebuild(|| ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("app")
            .with_child(
                ElementDef::new(Tag::Span)
                    .with_class("title")
                    .with_text("After"),
            )
            .with_child(
                ElementDef::new(Tag::Div)
                    .with_class("card")
                    .with_text("New card"),
            ),
    });

    // Locators re-resolve against current tree
    h.locator(".title").expect_text("After");
    assert_eq!(h.locator(".card").count(), 1);
}
