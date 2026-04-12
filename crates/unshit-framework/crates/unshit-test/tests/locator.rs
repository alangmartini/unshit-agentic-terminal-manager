use unshit_core::element::{ElementDef, ElementTree, Tag};
use unshit_test::TestHarness;

// ---------------------------------------------------------------------------
// Test tree builders
// ---------------------------------------------------------------------------

fn card_tree() -> ElementTree {
    ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(
                ElementDef::new(Tag::Div)
                    .with_class("card")
                    .with_child(
                        ElementDef::new(Tag::Span)
                            .with_class("title")
                            .with_text("Free"),
                    )
                    .with_child(
                        ElementDef::new(Tag::Span)
                            .with_class("body")
                            .with_text("Basic plan"),
                    ),
            )
            .with_child(
                ElementDef::new(Tag::Div)
                    .with_class("card")
                    .with_child(
                        ElementDef::new(Tag::Span)
                            .with_class("title")
                            .with_text("Premium"),
                    )
                    .with_child(
                        ElementDef::new(Tag::Span)
                            .with_class("body")
                            .with_text("Full access"),
                    ),
            )
            .with_child(
                ElementDef::new(Tag::Div)
                    .with_class("card")
                    .with_child(
                        ElementDef::new(Tag::Span)
                            .with_class("title")
                            .with_text("Enterprise"),
                    ),
            ),
    }
}

fn button_tree() -> ElementTree {
    ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(
                ElementDef::new(Tag::Div)
                    .with_class("btn")
                    .with_text("Submit"),
            )
            .with_child(
                ElementDef::new(Tag::Div)
                    .with_class("btn")
                    .with_class("secondary")
                    .with_text("Cancel"),
            ),
    }
}

fn input_tree() -> ElementTree {
    ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(
                ElementDef::new(Tag::Input)
                    .with_class("name-input"),
            ),
    }
}

const CSS: &str = "
    .root { width: 100%; display: flex; flex-direction: column; }
    .card { padding: 10px; width: 200px; height: 100px; }
    .title { font-size: 16px; }
    .body { font-size: 14px; }
    .btn { width: 100px; height: 40px; }
    .name-input { width: 200px; height: 30px; }
";

// ---------------------------------------------------------------------------
// Basic resolution
// ---------------------------------------------------------------------------

#[test]
fn locator_basic_resolution() {
    let mut h = TestHarness::new(CSS, card_tree, 800.0, 600.0);
    let count = h.locator(".card").count();
    assert_eq!(count, 3, "should find 3 cards");
}

#[test]
fn locator_single_resolution() {
    let mut h = TestHarness::new(CSS, card_tree, 800.0, 600.0);
    let snap = h.locator(".root").snapshot();
    assert!(snap.classes.contains(&"root".to_string()));
}

// ---------------------------------------------------------------------------
// Chaining
// ---------------------------------------------------------------------------

#[test]
fn locator_chaining() {
    let mut h = TestHarness::new(CSS, card_tree, 800.0, 600.0);
    let count = h.locator(".card").locator(".title").count();
    assert_eq!(count, 3, "each card has a .title");
}

#[test]
fn locator_chaining_narrows_scope() {
    let mut h = TestHarness::new(CSS, card_tree, 800.0, 600.0);
    // Only 2 cards have a .body
    let count = h.locator(".card").locator(".body").count();
    assert_eq!(count, 2, "only 2 cards have a .body");
}

// ---------------------------------------------------------------------------
// nth filter
// ---------------------------------------------------------------------------

#[test]
fn locator_nth() {
    let mut h = TestHarness::new(CSS, card_tree, 800.0, 600.0);
    let text = h.locator(".card").nth(0).locator(".title").text();
    assert_eq!(text, "Free");
}

#[test]
fn locator_nth_second() {
    let mut h = TestHarness::new(CSS, card_tree, 800.0, 600.0);
    let text = h.locator(".card").nth(1).locator(".title").text();
    assert_eq!(text, "Premium");
}

#[test]
fn locator_nth_out_of_bounds() {
    let mut h = TestHarness::new(CSS, card_tree, 800.0, 600.0);
    let count = h.locator(".card").nth(99).count();
    assert_eq!(count, 0);
}

// ---------------------------------------------------------------------------
// filter_by_text
// ---------------------------------------------------------------------------

#[test]
fn locator_filter_by_text() {
    let mut h = TestHarness::new(CSS, card_tree, 800.0, 600.0);
    let count = h.locator(".card").filter_by_text("Premium").count();
    assert_eq!(count, 1, "only one card contains 'Premium'");
}

#[test]
fn locator_filter_by_exact_text() {
    let mut h = TestHarness::new(CSS, card_tree, 800.0, 600.0);
    let titles = h.locator(".title").all_snapshots();
    assert_eq!(titles.len(), 3);
}

// ---------------------------------------------------------------------------
// locator_by_text
// ---------------------------------------------------------------------------

#[test]
fn locator_by_text_finds_exact() {
    let mut h = TestHarness::new(CSS, card_tree, 800.0, 600.0);
    let snap = h.locator_by_text("Premium").snapshot();
    assert!(snap.classes.contains(&"title".to_string()));
}

#[test]
fn locator_by_text_contains() {
    let mut h = TestHarness::new(CSS, card_tree, 800.0, 600.0);
    let count = h.locator_by_text_contains("plan").count();
    assert_eq!(count, 1, "'Basic plan' contains 'plan'");
}

// ---------------------------------------------------------------------------
// Zero-match panic
// ---------------------------------------------------------------------------

#[test]
#[should_panic(expected = "locator resolved to 0 elements")]
fn locator_zero_match_panics_on_action() {
    let mut h = TestHarness::new(CSS, card_tree, 800.0, 600.0);
    h.locator(".nonexistent").click();
}

#[test]
#[should_panic(expected = "locator resolved to 0 elements")]
fn locator_zero_match_panics_on_snapshot() {
    let mut h = TestHarness::new(CSS, card_tree, 800.0, 600.0);
    h.locator(".nonexistent").snapshot();
}

// ---------------------------------------------------------------------------
// Multi-match panic (for single-element actions)
// ---------------------------------------------------------------------------

#[test]
#[should_panic(expected = "locator resolved to 3 elements")]
fn locator_multi_match_panics_on_click() {
    let mut h = TestHarness::new(CSS, card_tree, 800.0, 600.0);
    h.locator(".card").click();
}

#[test]
#[should_panic(expected = "locator resolved to 3 elements")]
fn locator_multi_match_panics_on_text() {
    let mut h = TestHarness::new(CSS, card_tree, 800.0, 600.0);
    let _ = h.locator(".card").text();
}

// ---------------------------------------------------------------------------
// count
// ---------------------------------------------------------------------------

#[test]
fn locator_count_zero() {
    let mut h = TestHarness::new(CSS, card_tree, 800.0, 600.0);
    assert_eq!(h.locator(".nothing").count(), 0);
}

#[test]
fn locator_count_multiple() {
    let mut h = TestHarness::new(CSS, card_tree, 800.0, 600.0);
    assert_eq!(h.locator(".title").count(), 3);
}

// ---------------------------------------------------------------------------
// all_snapshots
// ---------------------------------------------------------------------------

#[test]
fn locator_all_snapshots() {
    let mut h = TestHarness::new(CSS, card_tree, 800.0, 600.0);
    let snaps = h.locator(".title").all_snapshots();
    assert_eq!(snaps.len(), 3);
    let texts: Vec<_> = snaps
        .iter()
        .filter_map(|s| {
            if let unshit_core::element::ElementContent::Text(ref t) = s.content {
                Some(t.clone())
            } else {
                None
            }
        })
        .collect();
    assert!(texts.contains(&"Free".to_string()));
    assert!(texts.contains(&"Premium".to_string()));
    assert!(texts.contains(&"Enterprise".to_string()));
}

// ---------------------------------------------------------------------------
// Re-resolution after rebuild
// ---------------------------------------------------------------------------

#[test]
fn locator_re_resolves_after_rebuild() {
    let mut h = TestHarness::new(CSS, card_tree, 800.0, 600.0);

    // Before rebuild: 3 cards
    assert_eq!(h.locator(".card").count(), 3);

    // Rebuild with fewer cards
    h.rebuild(|| ElementTree {
        root: ElementDef::new(Tag::Div)
            .with_class("root")
            .with_child(
                ElementDef::new(Tag::Div)
                    .with_class("card")
                    .with_child(
                        ElementDef::new(Tag::Span)
                            .with_class("title")
                            .with_text("Only one"),
                    ),
            ),
    });

    // After rebuild: 1 card
    assert_eq!(h.locator(".card").count(), 1);
    assert_eq!(h.locator(".title").text(), "Only one");
}

// ---------------------------------------------------------------------------
// Assertions
// ---------------------------------------------------------------------------

#[test]
fn locator_expect_text() {
    let mut h = TestHarness::new(CSS, card_tree, 800.0, 600.0);
    h.locator(".card").nth(0).locator(".title").expect_text("Free");
}

#[test]
fn locator_expect_text_contains() {
    let mut h = TestHarness::new(CSS, card_tree, 800.0, 600.0);
    h.locator(".card").nth(1).locator(".body").expect_text_contains("access");
}

#[test]
fn locator_expect_count() {
    let mut h = TestHarness::new(CSS, card_tree, 800.0, 600.0);
    h.locator(".card").expect_count(3);
}

#[test]
fn locator_expect_visible() {
    let mut h = TestHarness::new(CSS, card_tree, 800.0, 600.0);
    h.locator(".root").expect_visible();
}

#[test]
fn locator_expect_class() {
    let mut h = TestHarness::new(CSS, card_tree, 800.0, 600.0);
    h.locator(".card").nth(0).expect_class("card");
}

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

#[test]
fn locator_click_works() {
    let mut h = TestHarness::new(CSS, button_tree, 800.0, 600.0);
    // Clicking should not panic; we just verify it resolves and executes
    h.locator(".btn").nth(0).click();
}

#[test]
fn locator_hover_works() {
    let mut h = TestHarness::new(CSS, button_tree, 800.0, 600.0);
    h.locator(".btn").nth(0).hover();
}

#[test]
fn locator_fill_works() {
    let mut h = TestHarness::new(CSS, input_tree, 800.0, 600.0);
    h.locator(".name-input").fill("hello");
    h.locator(".name-input").expect_value("hello");
}

#[test]
fn locator_bounding_box() {
    let mut h = TestHarness::new(CSS, card_tree, 800.0, 600.0);
    let rect = h.locator(".root").bounding_box();
    assert!(rect.width > 0.0);
    assert!(rect.height > 0.0);
}

#[test]
fn locator_input_value() {
    let mut h = TestHarness::new(CSS, input_tree, 800.0, 600.0);
    h.locator(".name-input").fill("test");
    let val = h.locator(".name-input").input_value();
    assert_eq!(val, Some("test".to_string()));
}

// ---------------------------------------------------------------------------
// TestApp locator delegation
// ---------------------------------------------------------------------------

#[test]
fn test_app_locator_works() {
    std::env::remove_var("UNSHIT_TEST_HEADED");
    let mut app = unshit_test::TestApp::new(CSS, card_tree, 800.0, 600.0);
    assert_eq!(app.locator(".card").count(), 3);
}
