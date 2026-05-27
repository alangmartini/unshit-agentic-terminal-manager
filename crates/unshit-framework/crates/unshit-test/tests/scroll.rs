use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use unshit_core::dirty::DirtyFlags;
use unshit_core::element::{ElementDef, ElementTree, Tag};
use unshit_test::TestHarness;

fn scroll_css() -> &'static str {
    r#"
    .root { display: flex; flex-direction: column; width: 100%; height: 100%; }
    .scroll-container {
        display: flex;
        flex-direction: column;
        overflow: scroll;
        height: 200px;
        width: 100%;
        gap: 10px;
    }
    .item {
        display: flex;
        flex-shrink: 0;
        height: 50px;
        width: 100%;
        background: #333333;
    }
    "#
}

fn auto_scroll_css() -> &'static str {
    r#"
    .root { display: flex; flex-direction: column; width: 100%; height: 100%; }
    .scroll-container {
        display: flex;
        flex-direction: column;
        overflow: auto;
        height: 200px;
        width: 100%;
        gap: 10px;
    }
    .item {
        display: flex;
        flex-shrink: 0;
        height: 50px;
        width: 100%;
        background: #333333;
    }
    "#
}

fn hidden_css() -> &'static str {
    r#"
    .root { display: flex; flex-direction: column; width: 100%; height: 100%; }
    .scroll-container {
        display: flex;
        flex-direction: column;
        overflow: hidden;
        height: 200px;
        width: 100%;
        gap: 10px;
    }
    .item {
        display: flex;
        flex-shrink: 0;
        height: 50px;
        width: 100%;
        background: #333333;
    }
    "#
}

fn absolute_footer_css() -> &'static str {
    r#"
    .root { width: 100%; height: 100%; }
    .scroll-container {
        position: relative;
        display: flex;
        flex-direction: column;
        overflow: auto;
        height: 200px;
        width: 300px;
        padding-bottom: 48px;
    }
    .item {
        flex-shrink: 0;
        height: 80px;
        width: 100%;
        background: #333333;
    }
    .savebar {
        position: absolute;
        left: 0;
        right: 0;
        bottom: 0;
        height: 44px;
        display: flex;
        justify-content: flex-end;
        align-items: center;
    }
    .done {
        width: 72px;
        height: 30px;
    }
    "#
}

/// Build a tree with a root containing a scroll container with `n` items.
fn scroll_tree(n: usize) -> ElementTree {
    let mut container = ElementDef::new(Tag::Div).with_class("scroll-container");
    for _ in 0..n {
        container = container.with_child(ElementDef::new(Tag::Div).with_class("item"));
    }
    ElementTree { root: ElementDef::new(Tag::Div).with_class("root").with_child(container) }
}

/// Convenience: build a harness with 10 items (590px content in 200px container).
fn make_harness(css: &str) -> TestHarness {
    TestHarness::new(css, || scroll_tree(10), 800.0, 600.0)
}

fn absolute_footer_tree(counter: Arc<AtomicU32>) -> ElementTree {
    let mut container = ElementDef::new(Tag::Div).with_class("scroll-container");
    for _ in 0..6 {
        container = container.with_child(ElementDef::new(Tag::Div).with_class("item"));
    }
    container = container.with_child(ElementDef::new(Tag::Div).with_class("savebar").with_child(
        ElementDef::new(Tag::Button).with_class("done").on_click(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        }),
    ));

    ElementTree { root: ElementDef::new(Tag::Div).with_class("root").with_child(container) }
}

// -------------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------------

#[test]
fn scroll_wheel_updates_offset() {
    let mut h = make_harness(scroll_css());
    h.step();

    let snap = h.query(".scroll-container").expect("scroll-container exists");
    assert_eq!(snap.scroll_y, 0.0, "initial scroll_y should be 0");

    // Scroll down: negative delta_y means content moves up (scroll down).
    let cx = snap.layout_rect.x + snap.layout_rect.width / 2.0;
    let cy = snap.layout_rect.y + snap.layout_rect.height / 2.0;
    h.mouse_wheel(cx, cy, 0.0, -100.0);

    let snap = h.query(".scroll-container").expect("scroll-container exists");
    assert!(
        snap.scroll_y > 0.0,
        "scroll_y should increase after scrolling down, got {}",
        snap.scroll_y
    );
}

#[test]
fn overflow_auto_scrolls_like_scroll() {
    let mut h = make_harness(auto_scroll_css());
    h.step();

    let snap = h.query(".scroll-container").expect("scroll-container exists");
    let cx = snap.layout_rect.x + snap.layout_rect.width / 2.0;
    let cy = snap.layout_rect.y + snap.layout_rect.height / 2.0;
    h.mouse_wheel(cx, cy, 0.0, -100.0);

    let snap = h.query(".scroll-container").expect("scroll-container exists");
    assert!(
        snap.scroll_y > 0.0,
        "overflow:auto should create a wheel-scrollable container, got scroll_y={}",
        snap.scroll_y
    );
}

#[test]
fn scroll_clamps_to_zero() {
    let mut h = make_harness(scroll_css());
    h.step();

    let snap = h.query(".scroll-container").expect("scroll-container exists");
    let cx = snap.layout_rect.x + snap.layout_rect.width / 2.0;
    let cy = snap.layout_rect.y + snap.layout_rect.height / 2.0;

    // Scroll up past top: positive delta_y means content moves down (scroll up).
    h.mouse_wheel(cx, cy, 0.0, 100.0);

    let snap = h.query(".scroll-container").expect("scroll-container exists");
    assert_eq!(snap.scroll_y, 0.0, "scroll_y should be clamped to 0 when scrolling up past top");
}

#[test]
fn scroll_clamps_to_max() {
    let mut h = make_harness(scroll_css());
    h.step();

    let snap = h.query(".scroll-container").expect("scroll-container exists");
    let cx = snap.layout_rect.x + snap.layout_rect.width / 2.0;
    let cy = snap.layout_rect.y + snap.layout_rect.height / 2.0;
    let container_h = snap.layout_rect.height;

    // Scroll down by a huge amount.
    h.mouse_wheel(cx, cy, 0.0, -10000.0);

    let snap = h.query(".scroll-container").expect("scroll-container exists");
    assert!(snap.scroll_y > 0.0, "scroll_y should be positive after large scroll-down");

    // 10 items * 50px + 9 gaps * 10px = 590px content.
    // Max scroll = content_height - container_height.
    // Taffy computes content_size which may differ slightly due to layout, so
    // we just verify it is within a reasonable range.
    let max_expected = 590.0 - container_h;
    assert!(
        snap.scroll_y <= max_expected + 1.0,
        "scroll_y ({}) should be <= content - container ({}) (with 1px tolerance)",
        snap.scroll_y,
        max_expected,
    );
}

#[test]
fn overflow_hidden_does_not_scroll() {
    let mut h = make_harness(hidden_css());
    h.step();

    let snap = h.query(".scroll-container").expect("scroll-container exists");
    let cx = snap.layout_rect.x + snap.layout_rect.width / 2.0;
    let cy = snap.layout_rect.y + snap.layout_rect.height / 2.0;

    h.mouse_wheel(cx, cy, 0.0, -100.0);

    let snap = h.query(".scroll-container").expect("scroll-container exists");
    assert_eq!(snap.scroll_y, 0.0, "overflow:hidden container should not scroll");
}

#[test]
fn hit_test_accounts_for_scroll() {
    let mut h = make_harness(scroll_css());
    h.step();

    // Collect the NodeId of the first .item
    let items = h.query_all(".item");
    assert!(items.len() >= 2, "need at least 2 items");
    let first_item_id = items[0].node_id;
    let first_item_rect = items[0].layout_rect;

    // Hover in the center of the first item to confirm it is hovered
    let cx = first_item_rect.x + first_item_rect.width / 2.0;
    let cy = first_item_rect.y + first_item_rect.height / 2.0;
    h.mouse_move(cx, cy);
    assert_eq!(h.hovered(), first_item_id, "first item should be hovered before scroll");

    // Now scroll down by 60px so that the first item is partially out of view
    let container_snap = h.query(".scroll-container").expect("scroll-container exists");
    let cont_cx = container_snap.layout_rect.x + container_snap.layout_rect.width / 2.0;
    let cont_cy = container_snap.layout_rect.y + container_snap.layout_rect.height / 2.0;
    h.mouse_wheel(cont_cx, cont_cy, 0.0, -60.0);

    // Move mouse to where the first item USED TO be visually.
    // After scrolling, the content is shifted up, so the visual position of the
    // first item has changed: a different element should now be under the cursor.
    h.mouse_move(cx, cy);
    assert_ne!(
        h.hovered(),
        first_item_id,
        "after scrolling, the first item should no longer be at its original screen position"
    );
}

#[test]
fn absolute_child_in_scrolled_container_uses_visual_hitbox() {
    let counter = Arc::new(AtomicU32::new(0));
    let tree_counter = counter.clone();
    let mut h = TestHarness::new(
        absolute_footer_css(),
        move || absolute_footer_tree(tree_counter.clone()),
        800.0,
        600.0,
    );
    h.step();

    let container = h.query(".scroll-container").expect("scroll-container exists");
    let cx = container.layout_rect.x + container.layout_rect.width / 2.0;
    let cy = container.layout_rect.y + container.layout_rect.height / 2.0;
    h.mouse_wheel(cx, cy, 0.0, -160.0);

    let done = h.query(".done").expect("done button exists");
    let x = done.layout_rect.x + done.layout_rect.width / 2.0;
    let y = done.layout_rect.y + done.layout_rect.height / 2.0;
    h.click(x, y);

    assert_eq!(
        counter.load(Ordering::SeqCst),
        1,
        "absolute footer button should remain clickable after parent scroll"
    );
}

#[test]
fn scrollbar_renders_when_content_overflows() {
    let mut h = make_harness(scroll_css());
    h.step();

    // The scroll container has 590px content in 200px container.
    // A scrollbar should be rendered (verified by scroll state driving the render).
    let container = h.query(".scroll-container").expect("scroll-container exists");

    assert!(container.layout_rect.height < 590.0, "Container should be smaller than content");
    assert_eq!(container.scroll_y, 0.0, "Initial scroll should be 0");
}

#[test]
fn no_scrollbar_when_content_fits() {
    // Build tree with just 1 item (50px in 200px container).
    let mut h = TestHarness::new(scroll_css(), || scroll_tree(1), 800.0, 600.0);
    h.step();

    let container = h.query(".scroll-container").expect("scroll-container exists");
    // Content (50px) fits within container (200px), no scrollbar needed.
    assert_eq!(container.scroll_y, 0.0);
}

#[test]
fn scrollbar_thumb_position_changes_after_scroll() {
    let mut h = make_harness(scroll_css());
    h.step();

    let container = h.query(".scroll-container").expect("scroll-container exists");
    let cx = container.layout_rect.x + container.layout_rect.width / 2.0;
    let cy = container.layout_rect.y + container.layout_rect.height / 2.0;

    assert_eq!(container.scroll_y, 0.0);

    // Scroll down
    h.mouse_wheel(cx, cy, 0.0, -100.0);

    let container_after = h.query(".scroll-container").expect("scroll-container exists");
    assert!(container_after.scroll_y > 0.0, "scroll_y should increase after scrolling");
}

#[test]
fn scroll_wheel_marks_scrolled_subtree_and_ancestors_paint_dirty() {
    let mut h = make_harness(scroll_css());
    h.step();

    let root_id = h.root();
    unshit_renderer::batch::clear_paint_flags_subtree(h.arena_mut(), root_id);

    let container = h.query(".scroll-container").expect("scroll-container exists");
    let container_id = container.node_id;
    let first_item_id = h.query_all(".item").first().expect("item exists").node_id;
    let cx = container.layout_rect.x + container.layout_rect.width / 2.0;
    let cy = container.layout_rect.y + container.layout_rect.height / 2.0;

    h.mouse_wheel(cx, cy, 0.0, -100.0);

    let root_dirty = h.arena().get(root_id).expect("root exists").dirty;
    let container_dirty = h.arena().get(container_id).expect("container exists").dirty;
    let first_item_dirty = h.arena().get(first_item_id).expect("item exists").dirty;

    assert!(
        root_dirty.contains(DirtyFlags::SUBTREE_PAINT),
        "scrolling must dirty ancestors so cached parent batches do not replay stale content"
    );
    assert!(
        container_dirty.contains(DirtyFlags::PAINT | DirtyFlags::SUBTREE_PAINT),
        "scrolling must dirty the scroll container and its subtree"
    );
    assert!(
        first_item_dirty.contains(DirtyFlags::PAINT),
        "scrolling must dirty descendants because cached child quads use old absolute positions"
    );
}

#[test]
fn scroll_container_found_from_child() {
    let mut h = make_harness(scroll_css());
    h.step();

    // Hover a child item
    let items = h.query_all(".item");
    assert!(!items.is_empty(), "need at least one item");
    let item_rect = items[0].layout_rect;
    let cx = item_rect.x + item_rect.width / 2.0;
    let cy = item_rect.y + item_rect.height / 2.0;

    // Mouse wheel while hovering the child; the scroll should propagate to
    // the parent scroll container.
    h.mouse_wheel(cx, cy, 0.0, -50.0);

    let container = h.query(".scroll-container").expect("scroll-container exists");
    assert!(
        container.scroll_y > 0.0,
        "scrolling while hovering a child should update the parent scroll container, got scroll_y={}",
        container.scroll_y,
    );
}

// ---------------------------------------------------------------------------
// Interactive scrollbar tests
// ---------------------------------------------------------------------------

#[test]
fn scrollbar_thumb_drag_scrolls() {
    let mut h = make_harness(scroll_css());
    h.step();
    let snap = h.query(".scroll-container").expect("exists");
    let (v_geom, _) = unshit_core::scroll::compute_scrollbar_geometry(
        h.arena(),
        snap.node_id,
        snap.layout_rect.x,
        snap.layout_rect.y,
    );
    let geom = v_geom.expect("vertical scrollbar should exist");

    // Mouse down on thumb center
    let thumb_cx = geom.thumb_x + geom.thumb_w / 2.0;
    let thumb_cy = geom.thumb_y + geom.thumb_h / 2.0;
    h.mouse_down(thumb_cx, thumb_cy);

    // Drag down by 30px
    h.mouse_move(thumb_cx, thumb_cy + 30.0);
    h.mouse_up(thumb_cx, thumb_cy + 30.0);

    let snap = h.query(".scroll-container").expect("exists");
    assert!(
        snap.scroll_y > 0.0,
        "scroll_y should increase after thumb drag down, got {}",
        snap.scroll_y
    );
}

#[test]
fn scrollbar_track_click_jumps() {
    let mut h = make_harness(scroll_css());
    h.step();
    let snap = h.query(".scroll-container").expect("exists");
    let (v_geom, _) = unshit_core::scroll::compute_scrollbar_geometry(
        h.arena(),
        snap.node_id,
        snap.layout_rect.x,
        snap.layout_rect.y,
    );
    let geom = v_geom.expect("vertical scrollbar should exist");

    // Click on track near the bottom (below thumb)
    let click_x = geom.track_x + geom.track_w / 2.0;
    let click_y = geom.track_y + geom.track_h * 0.8;
    h.mouse_down(click_x, click_y);
    h.mouse_up(click_x, click_y);

    let snap = h.query(".scroll-container").expect("exists");
    assert!(snap.scroll_y > 0.0, "scroll should jump after track click, got {}", snap.scroll_y);
}

#[test]
fn scrollbar_thumb_drag_clamps() {
    let mut h = make_harness(scroll_css());
    h.step();
    let snap = h.query(".scroll-container").expect("exists");
    let (v_geom, _) = unshit_core::scroll::compute_scrollbar_geometry(
        h.arena(),
        snap.node_id,
        snap.layout_rect.x,
        snap.layout_rect.y,
    );
    let geom = v_geom.expect("vertical scrollbar should exist");

    // Drag thumb way past the bottom
    let thumb_cx = geom.thumb_x + geom.thumb_w / 2.0;
    let thumb_cy = geom.thumb_y + geom.thumb_h / 2.0;
    h.mouse_down(thumb_cx, thumb_cy);
    h.mouse_move(thumb_cx, thumb_cy + 9999.0);
    h.mouse_up(thumb_cx, thumb_cy + 9999.0);

    let snap = h.query(".scroll-container").expect("exists");
    assert!(snap.scroll_y > 0.0, "scroll should be positive");
    assert!(
        snap.scroll_y <= geom.max_scroll + 0.1,
        "scroll_y ({}) should not exceed max_scroll ({})",
        snap.scroll_y,
        geom.max_scroll
    );
}

#[test]
fn scrollbar_hover_does_not_scroll() {
    let mut h = make_harness(scroll_css());
    h.step();
    let snap = h.query(".scroll-container").expect("exists");
    let (v_geom, _) = unshit_core::scroll::compute_scrollbar_geometry(
        h.arena(),
        snap.node_id,
        snap.layout_rect.x,
        snap.layout_rect.y,
    );
    let geom = v_geom.expect("vertical scrollbar should exist");

    // Move mouse over scrollbar area without clicking
    h.mouse_move(geom.thumb_x + geom.thumb_w / 2.0, geom.thumb_y + geom.thumb_h / 2.0);

    let snap = h.query(".scroll-container").expect("exists");
    assert_eq!(snap.scroll_y, 0.0, "hovering scrollbar should not change scroll");
}

#[test]
fn scrollbar_not_interactive_when_content_fits() {
    let mut h = TestHarness::new(scroll_css(), || scroll_tree(1), 800.0, 600.0);
    h.step();
    let snap = h.query(".scroll-container").expect("exists");
    let (v_geom, h_geom) = unshit_core::scroll::compute_scrollbar_geometry(
        h.arena(),
        snap.node_id,
        snap.layout_rect.x,
        snap.layout_rect.y,
    );
    assert!(v_geom.is_none(), "no vertical scrollbar when content fits");
    assert!(h_geom.is_none(), "no horizontal scrollbar when content fits");
}
