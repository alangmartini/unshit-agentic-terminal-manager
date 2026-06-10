//! Anonymous text box tests.
//!
//! When an element has both non-empty text content and element children
//! (typically because a ::before/::after pseudo node was injected), taffy
//! never calls its measure function, so historically the text contributed
//! nothing to layout while still painting — boxes collapsed and text
//! overflowed. The framework now synthesizes a browser-style anonymous text
//! child that owns the text's measurement, painting, and hit testing.
//! These tests pin that machinery end to end.

use unshit_core::dirty::DirtyFlags;
use unshit_core::element::*;
use unshit_core::style::parse::PseudoElement;
use unshit_core::style::types::Color;
use unshit_test::TestHarness;

fn css() -> &'static str {
    r#"
    .root {
        display: flex;
        flex-direction: row;
        align-items: flex-start;
        width: 800px;
        height: 400px;
        padding: 0;
        margin: 0;
    }
    .pill {
        display: flex;
        flex-direction: row;
        padding: 4px;
        font-size: 16px;
        white-space: nowrap;
        margin: 0;
    }
    .with-plus::before {
        content: "+";
        font-size: 16px;
        padding: 0;
        margin: 0;
    }
    .with-empty::before {
        content: "";
    }
    .with-after::after {
        content: ">";
        font-size: 16px;
        padding: 0;
        margin: 0;
    }
    .hoverable:hover::before {
        content: "+";
        font-size: 16px;
        padding: 0;
        margin: 0;
    }
    .fixed {
        width: 300px;
    }
    span, div {
        margin: 0;
    }
    .user-box {
        width: 50px;
        height: 20px;
        padding: 0;
    }
    "#
}

fn pill_tree(classes: &'static [&'static str], text: &'static str) -> impl Fn() -> ElementTree {
    move || {
        let mut pill = ElementDef::new(Tag::Div).with_class("pill").with_text(text);
        for c in classes {
            pill = pill.with_class(*c);
        }
        ElementTree { root: ElementDef::new(Tag::Div).with_class("root").with_child(pill) }
    }
}

/// Find the anonymous text child of a host, if any.
fn anon_of(h: &TestHarness, host: unshit_core::id::NodeId) -> Option<unshit_core::id::NodeId> {
    let elem = h.arena().get(host)?;
    let anon = elem.anon_text_child?;
    assert!(
        h.arena().get(anon).map(|e| e.anonymous && e.synthetic).unwrap_or(false),
        "anon_text_child must point at a live anonymous synthetic node"
    );
    Some(anon)
}

// -- The original bug -------------------------------------------------------

/// A text-bearing host that gains a ::before pseudo child must still have
/// its text measured: an anonymous text box appears after the pseudo node
/// and the host grows to fit pseudo + text + padding.
#[test]
fn text_with_pseudo_content_is_measured() {
    let mut h = TestHarness::new(css(), pill_tree(&["with-plus"], "Shift"), 800.0, 400.0);
    h.step();

    let control = {
        let mut hc = TestHarness::new(css(), pill_tree(&[], "Shift"), 800.0, 400.0);
        hc.step();
        hc.query(".pill").expect("control pill").layout_rect
    };

    let pill = h.query(".pill").expect("pill");
    let children = h.arena().children(pill.node_id);
    assert_eq!(children.len(), 2, "::before + anonymous text box: {children:?}");

    let before = h.arena().get(children[0]).unwrap();
    assert!(before.synthetic);
    assert_eq!(before.pseudo_slot, Some(PseudoElement::Before));
    assert_eq!(before.content, ElementContent::Text("+".into()));

    let anon_id = anon_of(&h, pill.node_id).expect("anonymous text box");
    assert_eq!(anon_id, children[1], "anon box must follow ::before");
    let anon = h.arena().get(anon_id).unwrap();
    assert_eq!(anon.content, ElementContent::Text("Shift".into()));
    assert!(anon.layout_rect.width > 0.0, "anon box must measure: {:?}", anon.layout_rect);

    // The host must be wider than the control (it gained the "+" pseudo) —
    // before the fix it collapsed to padding-only width.
    assert!(
        pill.layout_rect.width > control.width + 1.0,
        "host must fit pseudo + text: with-plus={:?} control={control:?}",
        pill.layout_rect
    );
}

/// An EMPTY ::before (content: "") also disqualified the host from text
/// measurement before the fix. With the anonymous box, the host's geometry
/// must match a plain childless text leaf exactly.
#[test]
fn empty_pseudo_geometry_matches_plain_leaf() {
    let mut h = TestHarness::new(css(), pill_tree(&["with-empty"], "Count 42"), 800.0, 400.0);
    h.step();
    let mut hc = TestHarness::new(css(), pill_tree(&[], "Count 42"), 800.0, 400.0);
    hc.step();

    let subject = h.query(".pill").expect("pill").layout_rect;
    let control = hc.query(".pill").expect("pill").layout_rect;

    assert!(
        (subject.width - control.width).abs() <= 1.0
            && (subject.height - control.height).abs() <= 1.0,
        "empty-pseudo host must lay out like a plain text leaf: subject={subject:?} control={control:?}"
    );
}

/// In a fixed-width host the anonymous box stretches to the remaining
/// content width (flex_grow: 1), so text-align math against the box equals
/// today's math against the host content box.
#[test]
fn anon_box_fills_fixed_width_host_content_box() {
    let mut h = TestHarness::new(css(), pill_tree(&["with-plus", "fixed"], "Hi"), 800.0, 400.0);
    h.step();

    let pill = h.query(".pill").expect("pill");
    let children = h.arena().children(pill.node_id);
    let before = h.arena().get(children[0]).unwrap().layout_rect;
    let anon = h.arena().get(children[1]).unwrap().layout_rect;

    let content_right = pill.layout_rect.x + pill.layout_rect.width - 4.0; // padding.right
    assert!(
        (anon.x + anon.width - content_right).abs() <= 1.0,
        "anon box must stretch to the host's content edge: anon={anon:?} host={:?} before={before:?}",
        pill.layout_rect
    );
}

// -- Updates and teardown ---------------------------------------------------

/// A rebuild that changes ONLY the host's text (no style/class change, so
/// the cascade skips the host) must update the anonymous box's text in
/// place, preserving its NodeId.
#[test]
fn content_only_rebuild_updates_anon_text() {
    let mut h = TestHarness::new(css(), pill_tree(&["with-plus"], "Hi"), 800.0, 400.0);
    h.step();

    let pill_id = h.query(".pill").expect("pill").node_id;
    let anon_before = anon_of(&h, pill_id).expect("anon");
    let width_before = h.query(".pill").unwrap().layout_rect.width;

    h.rebuild(pill_tree(&["with-plus"], "Hi there, much longer"));

    let anon_after = anon_of(&h, pill_id).expect("anon survives");
    assert_eq!(anon_before, anon_after, "anon NodeId must be stable across content updates");
    assert_eq!(
        h.arena().get(anon_after).unwrap().content,
        ElementContent::Text("Hi there, much longer".into())
    );
    let width_after = h.query(".pill").unwrap().layout_rect.width;
    assert!(
        width_after > width_before + 1.0,
        "host must grow with its text: before={width_before} after={width_after}"
    );
}

/// When the pseudo child disappears (hover released), the host reverts to a
/// plain childless text leaf: the anonymous box is deallocated.
#[test]
fn teardown_when_pseudo_removed() {
    let mut h = TestHarness::new(css(), pill_tree(&["hoverable"], "Hover me"), 800.0, 400.0);
    h.step();

    let pill = h.query(".pill").expect("pill");
    let pill_id = pill.node_id;
    assert_eq!(h.arena().children(pill_id).len(), 0, "no children before hover");
    assert!(anon_of(&h, pill_id).is_none());

    let rect = h.query(".pill").unwrap().layout_rect;
    h.mouse_move(rect.x + rect.width * 0.5, rect.y + rect.height * 0.5);
    h.step();

    let anon_id = anon_of(&h, pill_id).expect("anon while hovered");
    assert_eq!(h.arena().children(pill_id).len(), 2, "::before + anon while hovered");

    h.mouse_move(700.0, 350.0);
    h.step();

    assert_eq!(h.arena().children(pill_id).len(), 0, "host reverts to childless leaf");
    assert!(h.arena().get(pill_id).unwrap().anon_text_child.is_none());
    assert!(h.arena().get(anon_id).is_none(), "anon box must be deallocated, not just unlinked");

    // And the leaf must still measure: same width as a never-hovered pill.
    let mut hc = TestHarness::new(css(), pill_tree(&[], "Hover me"), 800.0, 400.0);
    hc.step();
    let control = hc.query(".pill").unwrap().layout_rect;
    let subject = h.query(".pill").unwrap().layout_rect;
    assert!(
        (subject.width - control.width).abs() <= 1.0,
        "reverted host must measure like a plain leaf: subject={subject:?} control={control:?}"
    );
}

/// When the host's text empties, the anonymous box is removed even though
/// the pseudo child remains.
#[test]
fn teardown_when_text_emptied() {
    let mut h = TestHarness::new(css(), pill_tree(&["with-plus"], "Hi"), 800.0, 400.0);
    h.step();

    let pill_id = h.query(".pill").expect("pill").node_id;
    assert!(anon_of(&h, pill_id).is_some());

    h.rebuild(pill_tree(&["with-plus"], ""));

    assert!(anon_of(&h, pill_id).is_none(), "no anon box for empty text");
    let children = h.arena().children(pill_id);
    assert_eq!(children.len(), 1, "only the ::before remains: {children:?}");
    assert!(h.arena().get(children[0]).unwrap().pseudo_slot == Some(PseudoElement::Before));
}

// -- Ordering ----------------------------------------------------------------

/// Full ordering contract: ::before, anonymous text, user children, ::after.
#[test]
fn ordering_before_anon_user_after() {
    let tree = || ElementTree {
        root: ElementDef::new(Tag::Div).with_class("root").with_child(
            ElementDef::new(Tag::Div)
                .with_class("pill")
                .with_class("with-plus")
                .with_class("with-after")
                .with_text("mid")
                .with_child(ElementDef::new(Tag::Span).with_class("user-box")),
        ),
    };
    let mut h = TestHarness::new(css(), tree, 800.0, 400.0);
    h.step();

    let pill_id = h.query(".pill").expect("pill").node_id;
    let children = h.arena().children(pill_id);
    assert_eq!(children.len(), 4, "before + anon + user + after: {children:?}");

    let c0 = h.arena().get(children[0]).unwrap();
    let c1 = h.arena().get(children[1]).unwrap();
    let c2 = h.arena().get(children[2]).unwrap();
    let c3 = h.arena().get(children[3]).unwrap();
    assert_eq!(c0.pseudo_slot, Some(PseudoElement::Before), "first child is ::before");
    assert!(c1.anonymous, "second child is the anonymous text box");
    assert_eq!(c1.content, ElementContent::Text("mid".into()));
    assert!(!c2.synthetic && c2.classes.contains(&"user-box".to_string()), "third is user child");
    assert_eq!(c3.pseudo_slot, Some(PseudoElement::After), "last child is ::after");
}

/// An anonymous box created BEFORE a ::before exists must end up after the
/// ::before once it appears (the pseudo resolver links ::before as first
/// child; reconcile preserves leading-synthetic relative order).
#[test]
fn late_before_is_linked_ahead_of_anon() {
    let tree = || ElementTree {
        root: ElementDef::new(Tag::Div).with_class("root").with_child(
            ElementDef::new(Tag::Div)
                .with_class("pill")
                .with_class("hoverable")
                .with_text("text")
                .with_child(ElementDef::new(Tag::Span).with_class("user-box")),
        ),
    };
    let mut h = TestHarness::new(css(), tree, 800.0, 400.0);
    h.step();

    let pill_id = h.query(".pill").expect("pill").node_id;
    let children = h.arena().children(pill_id);
    assert_eq!(children.len(), 2, "anon + user before hover: {children:?}");
    assert!(h.arena().get(children[0]).unwrap().anonymous, "anon leads before hover");
    let anon_id = children[0];

    let rect = h.query(".pill").unwrap().layout_rect;
    h.mouse_move(rect.x + rect.width * 0.5, rect.y + rect.height * 0.5);
    h.step();

    let children = h.arena().children(pill_id);
    assert_eq!(children.len(), 3, "before + anon + user while hovered: {children:?}");
    assert_eq!(
        h.arena().get(children[0]).unwrap().pseudo_slot,
        Some(PseudoElement::Before),
        "::before takes the lead"
    );
    assert_eq!(children[1], anon_id, "anon keeps its NodeId and slots in after ::before");
}

// -- Events, selection, queries ----------------------------------------------

/// Hit testing is transparent to the anonymous box: the host stays the
/// hover/click target exactly as when it painted its own text.
#[test]
fn hit_test_returns_host_not_anon() {
    let mut h = TestHarness::new(css(), pill_tree(&["with-plus"], "Click me"), 800.0, 400.0);
    h.step();

    let pill_id = h.query(".pill").expect("pill").node_id;
    let anon_id = anon_of(&h, pill_id).expect("anon");
    let anon_rect = h.arena().get(anon_id).unwrap().layout_rect;

    let hit = unshit_core::event::hit_test(
        h.arena(),
        h.root(),
        anon_rect.x + anon_rect.width * 0.5,
        anon_rect.y + anon_rect.height * 0.5,
    );
    assert_eq!(hit, Some(pill_id), "host must be the hit target over its text");
}

/// Text selection anchors to the anonymous box (the node whose text arm
/// paints the highlight), via the text_hit_at redirect.
#[test]
fn selection_anchors_to_anon_box() {
    let mut h =
        TestHarness::new(css(), pill_tree(&["with-plus"], "Select this text"), 800.0, 400.0);
    h.step();

    let pill_id = h.query(".pill").expect("pill").node_id;
    let anon_id = anon_of(&h, pill_id).expect("anon");
    let r = h.arena().get(anon_id).unwrap().layout_rect;

    h.select_text(r.x + 2.0, r.y + r.height * 0.5, r.x + r.width - 2.0, r.y + r.height * 0.5);

    let sel = h.text_selection().expect("selection exists");
    assert_eq!(sel.anchor_element, anon_id, "selection anchor must be the anonymous box");
    assert_eq!(sel.focus_element, anon_id, "selection focus must be the anonymous box");
    assert!(sel.focus_offset > sel.anchor_offset, "drag must select a forward range: {sel:?}");
}

/// Text locators must resolve to the host exactly once — the anonymous box
/// mirrors the host's text and would otherwise double-match.
#[test]
fn selector_text_matches_host_once() {
    let mut h = TestHarness::new(css(), pill_tree(&["with-plus"], "Unique label"), 800.0, 400.0);
    h.step();

    let pill_id = h.query(".pill").expect("pill").node_id;
    let matches = h.query_all(r#"text("Unique label")"#);
    assert_eq!(matches.len(), 1, "exactly one text match");
    assert_eq!(matches[0].node_id, pill_id, "the host is the match, not the anon box");
}

// -- Style derivation ---------------------------------------------------------

/// DPI scaling must affect the anonymous box exactly once: its style is
/// derived from the host's already-scaled style, and the scale pass skips
/// anonymous nodes (no double-scale across repeated restyles).
#[test]
fn dpi_scale_applies_exactly_once() {
    let mut h = TestHarness::new(css(), pill_tree(&["with-plus"], "Scaled"), 800.0, 400.0);
    h.set_scale_factor(2.0);
    h.step();

    let pill_id = h.query(".pill").expect("pill").node_id;
    let host_size = h.arena().get(pill_id).unwrap().computed_style.font_size;
    assert!((host_size - 32.0).abs() < 0.01, "host font scales 16 -> 32, got {host_size}");

    let anon_id = anon_of(&h, pill_id).expect("anon");
    let anon_size = h.arena().get(anon_id).unwrap().computed_style.font_size;
    assert!((anon_size - 32.0).abs() < 0.01, "anon font matches host, got {anon_size}");

    // Force two more full restyle frames; the anon's stored style must not
    // accumulate scale.
    let rect = h.arena().get(pill_id).unwrap().layout_rect;
    h.mouse_move(rect.x + 1.0, rect.y + 1.0);
    h.step();
    h.mouse_move(700.0, 390.0);
    h.step();

    let anon_size = h.arena().get(anon_id).unwrap().computed_style.font_size;
    assert!((anon_size - 32.0).abs() < 0.01, "no double-scale across restyles, got {anon_size}");
}

/// Transition/animation ticks mutate host styles on PAINT-only frames where
/// the layout sync never runs; `refresh_anon_text_style` is the hook those
/// tick loops call to keep the anonymous box's copy live.
#[test]
fn tick_refresh_keeps_anon_style_live() {
    let mut h = TestHarness::new(css(), pill_tree(&["with-plus"], "Animated"), 800.0, 400.0);
    h.step();

    let pill_id = h.query(".pill").expect("pill").node_id;
    let anon_id = anon_of(&h, pill_id).expect("anon");

    let arena = h.arena_mut();
    let new_color = Color::rgb(0x12, 0x34, 0x56);
    arena.get_mut(pill_id).unwrap().computed_style.color = new_color;

    // Simulating tick_all_transitions' per-ticked-node hook:
    let refreshed = unshit_core::layout::refresh_anon_text_style(arena, pill_id);
    assert_eq!(refreshed, Some(anon_id), "refresh must report the updated anon box");
    let anon = arena.get(anon_id).unwrap();
    assert_eq!(anon.computed_style.color, new_color, "anon color must follow the host");
    assert!(anon.dirty.contains(DirtyFlags::PAINT), "anon must be repainted");

    // Idempotent: a second refresh with no host change is a no-op.
    assert_eq!(unshit_core::layout::refresh_anon_text_style(arena, pill_id), None);
}

/// The anonymous box carries zero box-insets and the painter-read props the
/// inheritance pass misses (opacity, text_overflow, text_shadow).
#[test]
fn anon_style_is_clean_derivation() {
    let mut h = TestHarness::new(css(), pill_tree(&["with-plus"], "Styled"), 800.0, 400.0);
    h.step();

    let pill_id = h.query(".pill").expect("pill").node_id;
    let anon_id = anon_of(&h, pill_id).expect("anon");
    let host = h.arena().get(pill_id).unwrap().computed_style.clone();
    let anon = h.arena().get(anon_id).unwrap().computed_style.clone();

    assert_eq!(anon.font_size, host.font_size);
    assert_eq!(anon.font_family, host.font_family);
    assert_eq!(anon.color, host.color);
    assert_eq!(anon.white_space, host.white_space);
    assert_eq!(anon.opacity, host.opacity);
    assert_eq!(anon.text_overflow, host.text_overflow);
    assert_eq!(anon.padding.left, 0.0, "anon box must add no padding of its own");
    assert_eq!(anon.padding.right, 0.0);
    assert_eq!(anon.border_width.left, 0.0, "anon box must add no border of its own");
    assert_eq!(anon.flex_grow, 1.0, "anon box fills the host content box");
}

/// After a settled frame, no layout-phase dirty flags remain on host or
/// anon box — synthesis must not leave residual per-frame work behind.
#[test]
fn no_residual_layout_dirty_after_settled_frame() {
    let mut h = TestHarness::new(css(), pill_tree(&["with-plus"], "Settled"), 800.0, 400.0);
    h.step();

    let pill_id = h.query(".pill").expect("pill").node_id;
    let anon_id = anon_of(&h, pill_id).expect("anon");

    let layout_phase = DirtyFlags::STYLE
        | DirtyFlags::LAYOUT
        | DirtyFlags::CHILDREN
        | DirtyFlags::CONTENT
        | DirtyFlags::SUBTREE_STYLE
        | DirtyFlags::SUBTREE_LAYOUT;

    for id in [pill_id, anon_id] {
        let dirty = h.arena().get(id).unwrap().dirty;
        assert!(
            !dirty.intersects(layout_phase),
            "node {id:?} keeps layout-phase dirt after a settled frame: {dirty:?}"
        );
    }
}
