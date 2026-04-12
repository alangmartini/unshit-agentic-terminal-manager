use unshit_core::element::*;
use unshit_test::TestHarness;

fn simple_harness() -> TestHarness {
    let css = r#"
        .root { display: flex; width: 100%; height: 100%; }
        .box { width: 100px; height: 100px; }
        .hidden { width: 0px; height: 0px; }
        .label { width: 200px; height: 24px; }
        .item { width: 50px; height: 20px; }
    "#;
    TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(ElementDef::new(Tag::Div).with_class("box"))
                .with_child(
                    ElementDef::new(Tag::Span)
                        .with_class("label")
                        .with_text("Count: 5"),
                )
                .with_child(ElementDef::new(Tag::Div).with_class("hidden"))
                .with_child(ElementDef::new(Tag::Div).with_class("item").with_class("active"))
                .with_child(ElementDef::new(Tag::Div).with_class("item"))
                .with_child(ElementDef::new(Tag::Div).with_class("item")),
        },
        800.0,
        600.0,
    )
}

// -- expect_visible ----------------------------------------------------------

#[test]
fn expect_visible_passes_immediately() {
    let mut h = simple_harness();
    h.expect_visible(".box");
}

#[test]
#[should_panic(expected = "Assertion failed after")]
fn expect_visible_fails_for_zero_size() {
    let mut h = simple_harness();
    h.expect_visible_with_timeout(".hidden", 3);
}

#[test]
#[should_panic(expected = "element not found")]
fn expect_visible_fails_for_missing() {
    let mut h = simple_harness();
    h.expect_visible_with_timeout(".nonexistent", 3);
}

// -- expect_hidden -----------------------------------------------------------

#[test]
fn expect_hidden_passes_for_zero_size() {
    let mut h = simple_harness();
    h.expect_hidden(".hidden");
}

#[test]
fn expect_hidden_passes_for_missing() {
    let mut h = simple_harness();
    h.expect_hidden(".nonexistent");
}

#[test]
#[should_panic(expected = "Assertion failed after")]
fn expect_hidden_fails_for_visible() {
    let mut h = simple_harness();
    h.expect_hidden_with_timeout(".box", 3);
}

// -- expect_exists / expect_not_exists ---------------------------------------

#[test]
fn expect_exists_passes_immediately() {
    let mut h = simple_harness();
    h.expect_exists(".box");
}

#[test]
#[should_panic(expected = "not found")]
fn expect_exists_fails_for_missing() {
    let mut h = simple_harness();
    h.expect_exists_with_timeout(".nonexistent", 3);
}

#[test]
fn expect_not_exists_passes_for_missing() {
    let mut h = simple_harness();
    h.expect_not_exists(".nonexistent");
}

#[test]
#[should_panic(expected = "element to not exist")]
fn expect_not_exists_fails_for_present() {
    let mut h = simple_harness();
    h.expect_not_exists_with_timeout(".box", 3);
}

// -- expect_text -------------------------------------------------------------

#[test]
fn expect_text_passes_immediately() {
    let mut h = simple_harness();
    h.expect_text(".label", "Count: 5");
}

#[test]
#[should_panic(expected = "Expected text")]
fn expect_text_fails_with_wrong_text() {
    let mut h = simple_harness();
    h.expect_text_with_timeout(".label", "Count: 99", 3);
}

#[test]
fn expect_text_contains_passes() {
    let mut h = simple_harness();
    h.expect_text_contains(".label", "Count");
}

#[test]
#[should_panic(expected = "Expected to contain")]
fn expect_text_contains_fails() {
    let mut h = simple_harness();
    h.expect_text_contains_with_timeout(".label", "MISSING", 3);
}

// -- expect_class / expect_not_class -----------------------------------------

#[test]
fn expect_class_passes() {
    let mut h = simple_harness();
    h.expect_class(".active", "item");
}

#[test]
#[should_panic(expected = "Expected class")]
fn expect_class_fails() {
    let mut h = simple_harness();
    h.expect_class_with_timeout(".box", "nonexistent-class", 3);
}

#[test]
fn expect_not_class_passes() {
    let mut h = simple_harness();
    h.expect_not_class(".box", "active");
}

#[test]
#[should_panic(expected = "Expected NOT to have class")]
fn expect_not_class_fails() {
    let mut h = simple_harness();
    h.expect_not_class_with_timeout(".active", "active", 3);
}

// -- expect_count ------------------------------------------------------------

#[test]
fn expect_count_passes() {
    let mut h = simple_harness();
    h.expect_count(".item", 3);
}

#[test]
#[should_panic(expected = "Expected count: 10")]
fn expect_count_fails() {
    let mut h = simple_harness();
    h.expect_count_with_timeout(".item", 10, 3);
}

// -- expect_element (custom predicate) ---------------------------------------

#[test]
fn expect_element_custom_predicate_passes() {
    let mut h = simple_harness();
    h.expect_element(".box", |snap| {
        snap.layout_rect.width >= 100.0
    });
}

#[test]
#[should_panic(expected = "Custom predicate returned false")]
fn expect_element_custom_predicate_fails() {
    let mut h = simple_harness();
    h.expect_element_with_timeout(".box", |snap| snap.layout_rect.width > 9999.0, 3);
}

// -- expect_checked / expect_not_checked -------------------------------------

#[test]
fn expect_checked_passes() {
    let css = r#"
        .root { display: flex; width: 100%; height: 100%; }
        input { width: 20px; height: 20px; }
    "#;
    let mut h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(
                    ElementDef::new(Tag::Input)
                        .with_class("cb")
                        .with_input_type(InputType::Checkbox)
                        .with_checked(true),
                ),
        },
        800.0,
        600.0,
    );
    h.expect_checked(".cb");
}

#[test]
fn expect_not_checked_passes() {
    let css = r#"
        .root { display: flex; width: 100%; height: 100%; }
        input { width: 20px; height: 20px; }
    "#;
    let mut h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(
                    ElementDef::new(Tag::Input)
                        .with_class("cb")
                        .with_input_type(InputType::Checkbox)
                        .with_checked(false),
                ),
        },
        800.0,
        600.0,
    );
    h.expect_not_checked(".cb");
}

// -- expect_value ------------------------------------------------------------

#[test]
fn expect_value_passes_for_input() {
    let css = r#"
        .root { display: flex; width: 100%; height: 100%; }
        input { width: 200px; height: 30px; }
    "#;
    let mut h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(
                    ElementDef::new(Tag::Input)
                        .with_class("inp")
                        .with_tab_index(0),
                ),
        },
        800.0,
        600.0,
    );
    // Focus and type into the input
    let inp = h.query(".inp").unwrap();
    h.focus(inp.node_id);
    h.type_text("hello");
    h.step();
    h.expect_value(".inp", "hello");
}

// -- expect_focused ----------------------------------------------------------

#[test]
fn expect_focused_passes() {
    let css = r#"
        .root { display: flex; width: 100%; height: 100%; }
        input { width: 200px; height: 30px; }
    "#;
    let mut h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(
                    ElementDef::new(Tag::Input)
                        .with_class("inp")
                        .with_tab_index(0),
                ),
        },
        800.0,
        600.0,
    );
    let inp = h.query(".inp").unwrap();
    h.focus(inp.node_id);
    h.step();
    h.expect_focused(".inp");
}

#[test]
#[should_panic(expected = "Expected: focused")]
fn expect_focused_fails_when_not_focused() {
    let css = r#"
        .root { display: flex; width: 100%; height: 100%; }
        .box { width: 100px; height: 100px; }
    "#;
    let mut h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(ElementDef::new(Tag::Div).with_class("box")),
        },
        800.0,
        600.0,
    );
    h.expect_focused_with_timeout(".box", 3);
}

// -- Text collection from children -------------------------------------------

#[test]
fn expect_text_collects_from_children() {
    let css = r#"
        .root { display: flex; width: 100%; height: 100%; }
        .container { width: 300px; height: 50px; }
        span { width: auto; height: auto; }
    "#;
    let mut h = TestHarness::new(
        css,
        || ElementTree {
            root: ElementDef::new(Tag::Div)
                .with_class("root")
                .with_child(
                    ElementDef::new(Tag::Div)
                        .with_class("container")
                        .with_child(ElementDef::new(Tag::Span).with_text("Hello "))
                        .with_child(ElementDef::new(Tag::Span).with_text("World")),
                ),
        },
        800.0,
        600.0,
    );
    h.expect_text(".container", "Hello World");
}

// -- Retry behavior ----------------------------------------------------------

#[test]
fn expect_text_with_custom_timeout() {
    let mut h = simple_harness();
    // With a large timeout but immediate match, it should return fast
    h.expect_text_with_timeout(".label", "Count: 5", 120);
}

// -- Panic message format ----------------------------------------------------

#[test]
#[should_panic(expected = "Selector: .label")]
fn panic_message_includes_selector() {
    let mut h = simple_harness();
    h.expect_text_with_timeout(".label", "wrong", 3);
}

#[test]
#[should_panic(expected = "Element tag:")]
fn panic_message_includes_tag() {
    let mut h = simple_harness();
    h.expect_text_with_timeout(".label", "wrong", 3);
}
