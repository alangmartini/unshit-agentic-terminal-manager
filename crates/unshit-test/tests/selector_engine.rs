use unshit_core::element::*;
use unshit_test::TestHarness;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn simple_css() -> &'static str {
    r#"
        .root { display: flex; flex-direction: column; width: 100%; height: 100%; }
        .row  { display: flex; width: 100%; height: 50px; }
        div   { width: 50px; height: 50px; }
    "#
}

/// Build a tree:
///   root.root
///     div.sidebar
///       div.menu-item.first
///       div.menu-item.second
///       div.menu-item.third
///     div.content
///       button#submit.primary
///       span.label (text: "Click me")
///       input[placeholder="Search", type=text]
///       input[type=checkbox, checked]
fn make_tree() -> ElementTree {
    ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(
                ElementDef::new(Tag::Div)
                    .with_class("sidebar")
                    .with_child(
                        ElementDef::new(Tag::Div)
                            .with_class("menu-item")
                            .with_class("first"),
                    )
                    .with_child(
                        ElementDef::new(Tag::Div)
                            .with_class("menu-item")
                            .with_class("second"),
                    )
                    .with_child(
                        ElementDef::new(Tag::Div)
                            .with_class("menu-item")
                            .with_class("third"),
                    ),
            )
            .with_child(
                ElementDef::new(Tag::Div)
                    .with_class("content")
                    .with_child(
                        ElementDef::new(Tag::Button)
                            .with_id("submit")
                            .with_class("primary"),
                    )
                    .with_child(
                        ElementDef::new(Tag::Span)
                            .with_class("label")
                            .with_text("Click me"),
                    )
                    .with_child(
                        ElementDef::new(Tag::Input)
                            .with_class("search-box")
                            .with_placeholder("Search")
                            .with_input_type(InputType::Text),
                    )
                    .with_child(
                        ElementDef::new(Tag::Input)
                            .with_class("toggle")
                            .with_input_type(InputType::Checkbox)
                            .with_checked(true),
                    ),
            ),
    }
}

// ---------------------------------------------------------------------------
// Simple selectors (backward compatibility)
// ---------------------------------------------------------------------------

#[test]
fn simple_class_selector() {
    let h = TestHarness::new(simple_css(), make_tree, 800.0, 600.0);
    let snap = h.query(".sidebar").expect("should find .sidebar");
    assert!(snap.classes.contains(&"sidebar".to_string()));
}

#[test]
fn simple_id_selector() {
    let h = TestHarness::new(simple_css(), make_tree, 800.0, 600.0);
    let snap = h.query("#submit").expect("should find #submit");
    assert_eq!(snap.id, Some("submit".to_string()));
}

#[test]
fn simple_tag_selector() {
    let h = TestHarness::new(simple_css(), make_tree, 800.0, 600.0);
    let all_buttons = h.query_all("button");
    assert_eq!(all_buttons.len(), 1);
    assert_eq!(all_buttons[0].tag, Tag::Button);
}

// ---------------------------------------------------------------------------
// Compound selectors
// ---------------------------------------------------------------------------

#[test]
fn compound_tag_and_class() {
    let h = TestHarness::new(simple_css(), make_tree, 800.0, 600.0);
    let snap = h.query("div.sidebar").expect("should find div.sidebar");
    assert!(snap.classes.contains(&"sidebar".to_string()));
    assert_eq!(snap.tag, Tag::Div);
}

#[test]
fn compound_tag_id_class() {
    let h = TestHarness::new(simple_css(), make_tree, 800.0, 600.0);
    let snap = h.query("button#submit.primary").expect("should find button#submit.primary");
    assert_eq!(snap.tag, Tag::Button);
    assert_eq!(snap.id, Some("submit".to_string()));
    assert!(snap.classes.contains(&"primary".to_string()));
}

#[test]
fn compound_no_match_wrong_tag() {
    let h = TestHarness::new(simple_css(), make_tree, 800.0, 600.0);
    // There is no span.sidebar
    assert!(h.query("span.sidebar").is_none());
}

// ---------------------------------------------------------------------------
// Descendant combinator
// ---------------------------------------------------------------------------

#[test]
fn descendant_finds_nested() {
    let h = TestHarness::new(simple_css(), make_tree, 800.0, 600.0);
    let items = h.query_all(".sidebar .menu-item");
    assert_eq!(items.len(), 3, "should find 3 menu items inside .sidebar");
}

#[test]
fn descendant_does_not_match_outside() {
    let h = TestHarness::new(simple_css(), make_tree, 800.0, 600.0);
    // .content does not contain .menu-item
    let items = h.query_all(".content .menu-item");
    assert!(items.is_empty());
}

#[test]
fn descendant_multi_level() {
    let h = TestHarness::new(simple_css(), make_tree, 800.0, 600.0);
    // .root .menu-item should still find all menu items (grandchildren)
    let items = h.query_all(".root .menu-item");
    assert_eq!(items.len(), 3);
}

// ---------------------------------------------------------------------------
// Child combinator
// ---------------------------------------------------------------------------

#[test]
fn child_direct_match() {
    let h = TestHarness::new(simple_css(), make_tree, 800.0, 600.0);
    let items = h.query_all(".sidebar > .menu-item");
    assert_eq!(items.len(), 3);
}

#[test]
fn child_does_not_match_grandchildren() {
    let h = TestHarness::new(simple_css(), make_tree, 800.0, 600.0);
    // .root > .menu-item should fail because menu-items are grandchildren
    let items = h.query_all(".root > .menu-item");
    assert!(items.is_empty());
}

#[test]
fn child_combined_with_descendant() {
    let h = TestHarness::new(simple_css(), make_tree, 800.0, 600.0);
    // .root > .sidebar .menu-item: root direct-child sidebar, then descendant menu-item
    let items = h.query_all(".root > .sidebar .menu-item");
    assert_eq!(items.len(), 3);
}

// ---------------------------------------------------------------------------
// Attribute selectors
// ---------------------------------------------------------------------------

#[test]
fn attribute_placeholder() {
    let h = TestHarness::new(simple_css(), make_tree, 800.0, 600.0);
    let snap = h.query("[placeholder=\"Search\"]").expect("should find by placeholder");
    assert_eq!(snap.placeholder, Some("Search".to_string()));
}

#[test]
fn attribute_type_checkbox() {
    let h = TestHarness::new(simple_css(), make_tree, 800.0, 600.0);
    let items = h.query_all("input[type=\"checkbox\"]");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].input_type, Some(InputType::Checkbox));
}

#[test]
fn attribute_type_text() {
    let h = TestHarness::new(simple_css(), make_tree, 800.0, 600.0);
    let items = h.query_all("input[type=\"text\"]");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].placeholder, Some("Search".to_string()));
}

// ---------------------------------------------------------------------------
// Pseudo-classes
// ---------------------------------------------------------------------------

#[test]
fn pseudo_first_child() {
    let h = TestHarness::new(simple_css(), make_tree, 800.0, 600.0);
    let snap = h.query(".menu-item:first-child").expect("should find first menu-item");
    assert!(snap.classes.contains(&"first".to_string()));
}

#[test]
fn pseudo_last_child() {
    let h = TestHarness::new(simple_css(), make_tree, 800.0, 600.0);
    let snap = h.query(".menu-item:last-child").expect("should find last menu-item");
    assert!(snap.classes.contains(&"third".to_string()));
}

#[test]
fn pseudo_nth_child() {
    let h = TestHarness::new(simple_css(), make_tree, 800.0, 600.0);
    let snap = h.query(".menu-item:nth-child(2)").expect("should find 2nd menu-item");
    assert!(snap.classes.contains(&"second".to_string()));
}

#[test]
fn pseudo_nth_child_out_of_range() {
    let h = TestHarness::new(simple_css(), make_tree, 800.0, 600.0);
    assert!(h.query(".menu-item:nth-child(99)").is_none());
}

#[test]
fn pseudo_checked() {
    let h = TestHarness::new(simple_css(), make_tree, 800.0, 600.0);
    let snap = h.query(":checked").expect("should find checked element");
    assert_eq!(snap.checked, Some(true));
    assert_eq!(snap.input_type, Some(InputType::Checkbox));
}

#[test]
fn pseudo_checked_with_tag() {
    let h = TestHarness::new(simple_css(), make_tree, 800.0, 600.0);
    let snap = h.query("input:checked").expect("should find checked input");
    assert_eq!(snap.tag, Tag::Input);
}

// ---------------------------------------------------------------------------
// Text content matching
// ---------------------------------------------------------------------------

#[test]
fn text_exact_match() {
    let h = TestHarness::new(simple_css(), make_tree, 800.0, 600.0);
    let snap = h.query("text(\"Click me\")").expect("should find text node");
    assert_eq!(snap.content, ElementContent::Text("Click me".into()));
}

#[test]
fn text_contains_match() {
    let h = TestHarness::new(simple_css(), make_tree, 800.0, 600.0);
    let snap = h.query("has_text(\"Click\")").expect("should find containing text");
    assert_eq!(snap.content, ElementContent::Text("Click me".into()));
}

#[test]
fn text_exact_no_match() {
    let h = TestHarness::new(simple_css(), make_tree, 800.0, 600.0);
    assert!(h.query("text(\"Does not exist\")").is_none());
}

#[test]
fn text_contains_no_match() {
    let h = TestHarness::new(simple_css(), make_tree, 800.0, 600.0);
    assert!(h.query("has_text(\"zzz_no_match\")").is_none());
}

// ---------------------------------------------------------------------------
// query_all returns correct count
// ---------------------------------------------------------------------------

#[test]
fn query_all_returns_all_matches() {
    let h = TestHarness::new(simple_css(), make_tree, 800.0, 600.0);
    let divs = h.query_all("div");
    // root + sidebar + 3 menu-items + content = 6 divs
    assert_eq!(divs.len(), 6);
}

#[test]
fn query_all_no_match_returns_empty() {
    let h = TestHarness::new(simple_css(), make_tree, 800.0, 600.0);
    let items = h.query_all(".nonexistent");
    assert!(items.is_empty());
}

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

#[test]
fn multiple_classes_on_compound() {
    let h = TestHarness::new(simple_css(), make_tree, 800.0, 600.0);
    let snap = h.query(".menu-item.second").expect("should find .menu-item.second");
    assert!(snap.classes.contains(&"second".to_string()));
}

#[test]
fn child_combinator_no_space() {
    // Should still parse correctly even with less spacing
    let h = TestHarness::new(simple_css(), make_tree, 800.0, 600.0);
    let items = h.query_all(".sidebar>.menu-item");
    assert_eq!(items.len(), 3);
}

#[test]
fn deep_descendant_chain() {
    let h = TestHarness::new(simple_css(), make_tree, 800.0, 600.0);
    // .root .content button should match the submit button
    let snap = h.query(".root .content button").expect("should find deeply nested button");
    assert_eq!(snap.tag, Tag::Button);
}

#[test]
fn mixed_child_and_descendant() {
    let h = TestHarness::new(simple_css(), make_tree, 800.0, 600.0);
    // .root > .content > button#submit
    let snap =
        h.query(".root > .content > button#submit").expect("should find via mixed combinator chain");
    assert_eq!(snap.id, Some("submit".to_string()));
}
