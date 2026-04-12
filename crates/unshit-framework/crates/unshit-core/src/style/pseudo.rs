//! Resolver for `::before` and `::after` pseudo elements.
//!
//! Design summary (see issue #121):
//! - Pseudo nodes are real arena entries flagged with `Element::synthetic = true`.
//! - They are linked as normal first or last children of the host so layout,
//!   paint, hit testing, and batch emission need no pseudo aware code paths.
//! - The reconciler (`reconcile_children`) skips synthetic children during
//!   positional matching and always re stitches them to the ends of the
//!   sibling chain, so user tree diffing never sees pseudo nodes.
//! - A side table keyed by host `NodeId` records the before and after
//!   synthetic node ids so this resolver can tear down stale entries when a
//!   rule stops matching, and so it can update existing nodes in place when
//!   the content value changes between frames.
//!
//! Scope: only `content: "literal"` and `content: attr(name)` produce a
//! visible pseudo box. `content: none`, `content: normal`, and the absence
//! of a content declaration all result in no synthetic node being allocated.

use rustc_hash::{FxHashMap, FxHashSet};
use taffy::TaffyTree;

use crate::dirty::DirtyFlags;
use crate::element::{Element, ElementContent, Tag};
use crate::id::NodeId;
use crate::layout::TextMeasureCtx;
use crate::style::cascade;
use crate::style::parse::{CompiledStylesheet, PseudoElement};
use crate::style::types::ContentValue;
use crate::tree::NodeArena;

/// Side table mapping a host `NodeId` to the before and after pseudo node
/// ids that have been synthesized for it.
#[derive(Debug, Default, Clone)]
pub struct PseudoSideTable {
    entries: FxHashMap<NodeId, PseudoSlot>,
    /// Tracks which `attr(name)` misses have already been warned about, so a
    /// missing attribute produces one debug message instead of a per frame
    /// firehose.
    warned_missing_attrs: FxHashSet<(NodeId, String)>,
}

#[derive(Debug, Default, Clone, Copy)]
struct PseudoSlot {
    before: Option<NodeId>,
    after: Option<NodeId>,
    placeholder: Option<NodeId>,
}

impl PseudoSideTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the synthetic node id stored for the given host and slot, if any.
    pub fn get(&self, host: NodeId, slot: PseudoElement) -> Option<NodeId> {
        let entry = self.entries.get(&host)?;
        match slot {
            PseudoElement::Before => entry.before,
            PseudoElement::After => entry.after,
            PseudoElement::Selection => None,
            PseudoElement::Placeholder => entry.placeholder,
        }
    }

    /// Returns true if the table has any entry for the given host.
    pub fn contains_host(&self, host: NodeId) -> bool {
        self.entries.contains_key(&host)
    }

    /// Returns the total number of synthetic nodes currently tracked.
    pub fn len(&self) -> usize {
        self.entries
            .values()
            .map(|e| {
                e.before.is_some() as usize
                    + e.after.is_some() as usize
                    + e.placeholder.is_some() as usize
            })
            .sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn insert(&mut self, host: NodeId, slot: PseudoElement, node: NodeId) {
        let entry = self.entries.entry(host).or_default();
        match slot {
            PseudoElement::Before => entry.before = Some(node),
            PseudoElement::After => entry.after = Some(node),
            PseudoElement::Selection => {}
            PseudoElement::Placeholder => entry.placeholder = Some(node),
        }
    }

    fn clear_slot(&mut self, host: NodeId, slot: PseudoElement) {
        if let Some(entry) = self.entries.get_mut(&host) {
            match slot {
                PseudoElement::Before => entry.before = None,
                PseudoElement::After => entry.after = None,
                PseudoElement::Selection => {}
                PseudoElement::Placeholder => entry.placeholder = None,
            }
            if entry.before.is_none() && entry.after.is_none() && entry.placeholder.is_none() {
                self.entries.remove(&host);
            }
        }
    }

    /// Drop any host entries whose arena slot has been deallocated. Called at
    /// the start of a resolver pass so rebuilt hosts never inherit stale
    /// pseudo metadata from a previous generation.
    fn prune_dead_hosts(&mut self, arena: &NodeArena) {
        self.entries.retain(|host, _| arena.get(*host).is_some());
        self.warned_missing_attrs.retain(|(host, _)| arena.get(*host).is_some());
    }
}

/// Resolve `::before` and `::after` pseudo elements for every node in the
/// subtree rooted at `root`.
///
/// Walks the arena, and for each non synthetic host asks the cascade to
/// resolve a pseudo computed style for the before and after slots. If the
/// resolved `content` value produces a visible box (literal or attr), the
/// resolver either updates an existing synthetic node in place or allocates
/// a new one and links it into the host's child list at the correct end.
/// If the content value no longer produces a box, any previously allocated
/// synthetic node is torn down cleanly.
pub fn resolve_pseudo_elements(
    arena: &mut NodeArena,
    taffy: &mut TaffyTree<TextMeasureCtx>,
    stylesheet: &CompiledStylesheet,
    root: NodeId,
    hovered: NodeId,
    active: Option<NodeId>,
    focused: NodeId,
    table: &mut PseudoSideTable,
) {
    table.prune_dead_hosts(arena);

    // Snapshot the set of hosts with existing entries so we can detect any
    // hosts that stopped matching entirely.
    let existing_hosts: Vec<NodeId> = table.entries.keys().copied().collect();
    let mut visited_hosts: FxHashSet<NodeId> = FxHashSet::default();

    resolve_pseudo_walk(
        arena,
        taffy,
        stylesheet,
        root,
        hovered,
        active,
        focused,
        table,
        &mut visited_hosts,
    );

    // Tear down entries for hosts that no longer matched any pseudo rule.
    for host in existing_hosts {
        if !visited_hosts.contains(&host) {
            // Host still exists but was never visited, which means its subtree
            // is disconnected from the walked root. Leave the entry alone so
            // another walk rooted elsewhere can pick it up.
            if arena.get(host).is_none() {
                table.entries.remove(&host);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn resolve_pseudo_walk(
    arena: &mut NodeArena,
    taffy: &mut TaffyTree<TextMeasureCtx>,
    stylesheet: &CompiledStylesheet,
    node_id: NodeId,
    hovered: NodeId,
    active: Option<NodeId>,
    focused: NodeId,
    table: &mut PseudoSideTable,
    visited_hosts: &mut FxHashSet<NodeId>,
) {
    // Synthetic nodes themselves cannot host pseudo elements (a follow up
    // issue would be required, and the first pass scopes this out).
    let is_host_eligible = arena.get(node_id).map(|e| !e.synthetic).unwrap_or(false);

    if !is_host_eligible {
        return;
    }

    // Snapshot children first so the walk sees a stable view of user children.
    // The synthetic children (if any) are included here; `resolve_pseudo_walk`
    // short circuits for synthetic nodes, so visiting them is harmless.
    let children = arena.children(node_id);

    visited_hosts.insert(node_id);

    // Resolve before and after in turn. Order matters: before is linked as
    // first child, after is linked as last child.
    resolve_pseudo_slot(
        arena,
        taffy,
        stylesheet,
        node_id,
        PseudoElement::Before,
        hovered,
        active,
        focused,
        table,
    );
    resolve_pseudo_slot(
        arena,
        taffy,
        stylesheet,
        node_id,
        PseudoElement::After,
        hovered,
        active,
        focused,
        table,
    );
    resolve_pseudo_slot(
        arena,
        taffy,
        stylesheet,
        node_id,
        PseudoElement::Placeholder,
        hovered,
        active,
        focused,
        table,
    );

    for child_id in children {
        resolve_pseudo_walk(
            arena,
            taffy,
            stylesheet,
            child_id,
            hovered,
            active,
            focused,
            table,
            visited_hosts,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn resolve_pseudo_slot(
    arena: &mut NodeArena,
    taffy: &mut TaffyTree<TextMeasureCtx>,
    stylesheet: &CompiledStylesheet,
    host: NodeId,
    slot: PseudoElement,
    hovered: NodeId,
    active: Option<NodeId>,
    focused: NodeId,
    table: &mut PseudoSideTable,
) {
    let pseudo_style = cascade::resolve_style_with_pseudo(
        arena,
        stylesheet,
        host,
        hovered,
        active,
        focused,
        Some(slot),
    );

    // If the cascade did not produce a content value with a visible box, tear
    // down any previously allocated synthetic node for this slot.
    let content_value = pseudo_style.content.clone();
    if !content_value.produces_box() {
        if let Some(old_id) = table.get(host, slot) {
            remove_pseudo_node(arena, taffy, host, old_id);
            table.clear_slot(host, slot);
        }
        return;
    }

    // Evaluate the content value against the host's attributes to get the
    // actual text to render. The evaluation happens before the mutable
    // allocation so we do not hold two borrows on the arena.
    let text = evaluate_content(arena, host, &content_value, table);
    let mut element_content = ElementContent::Text(text.clone());

    // An empty string still allocates the pseudo node so layout remains
    // stable. Taffy's text measure requires a non empty string for non
    // trivial sizing, but an empty string is accepted and resolves to zero
    // dimensions, which matches the spec for an empty content box.
    if let ElementContent::Text(ref t) = element_content {
        if t.is_empty() {
            element_content = ElementContent::Text(String::new());
        }
    }

    match table.get(host, slot) {
        Some(existing_id) => {
            // Update the existing synthetic node in place so its NodeId is
            // preserved across frames, which keeps damage tracking and any
            // downstream caches stable.
            if let Some(elem) = arena.get_mut(existing_id) {
                let content_changed = elem.content != element_content;
                if content_changed {
                    elem.content = element_content;
                    elem.dirty |= DirtyFlags::LAYOUT | DirtyFlags::CONTENT | DirtyFlags::PAINT;
                }
                // Always update computed style: it may have changed because
                // of a hover or focus transition, even if the content did not.
                elem.computed_style = pseudo_style;
            }
        }
        None => {
            let mut elem = Element::new(Tag::Span);
            elem.parent = host;
            elem.content = element_content;
            elem.computed_style = pseudo_style;
            elem.synthetic = true;
            let new_id = arena.alloc(elem);

            match slot {
                PseudoElement::Before => link_pseudo_before(arena, host, new_id),
                PseudoElement::After => link_pseudo_after(arena, host, new_id),
                PseudoElement::Selection => unreachable!(),
                PseudoElement::Placeholder => link_pseudo_before(arena, host, new_id),
            }

            // The host's child list changed, so its layout sync needs to
            // pick up new taffy children on the next pass.
            if let Some(host_elem) = arena.get_mut(host) {
                host_elem.dirty |= DirtyFlags::CHILDREN | DirtyFlags::LAYOUT;
            }

            table.insert(host, slot, new_id);
        }
    }
}

/// Link a ::before synthetic node as the host's first child, pushing any
/// existing first child down the sibling chain.
fn link_pseudo_before(arena: &mut NodeArena, host: NodeId, new_id: NodeId) {
    let existing_first = arena.get(host).map(|e| e.first_child).unwrap_or(NodeId::DANGLING);

    if existing_first.is_dangling() {
        if let Some(h) = arena.get_mut(host) {
            h.first_child = new_id;
            h.last_child = new_id;
        }
        if let Some(n) = arena.get_mut(new_id) {
            n.parent = host;
            n.prev_sibling = NodeId::DANGLING;
            n.next_sibling = NodeId::DANGLING;
        }
    } else {
        arena.insert_child_before(host, new_id, existing_first);
    }
}

/// Link an ::after synthetic node as the host's last child, appending it
/// after all existing user children.
fn link_pseudo_after(arena: &mut NodeArena, host: NodeId, new_id: NodeId) {
    // If the host already has a trailing synthetic after node, append_child
    // would push this node past the existing one. The resolver ensures a slot
    // holds at most one node, so this branch is only reached when slot was
    // empty, which means append_child is the correct action.
    arena.append_child(host, new_id);
}

/// Unlink and deallocate a previously synthesized pseudo node.
fn remove_pseudo_node(
    arena: &mut NodeArena,
    taffy: &mut TaffyTree<TextMeasureCtx>,
    host: NodeId,
    pseudo_id: NodeId,
) {
    // Free the taffy node first so no stale handle is left behind.
    if let Some(elem) = arena.get(pseudo_id) {
        if let Some(tn) = elem.taffy_node {
            let _ = taffy.remove(tn);
        }
    }

    arena.remove_child(host, pseudo_id);
    arena.dealloc(pseudo_id);

    if let Some(host_elem) = arena.get_mut(host) {
        host_elem.dirty |= DirtyFlags::CHILDREN | DirtyFlags::LAYOUT;
    }
}

/// Evaluate a content value against the host element's attributes.
///
/// Literal strings resolve to themselves. `attr(id)` and `attr(class)`
/// resolve against the well known attributes on `Element`. Any other
/// `attr(name)` resolves to the empty string with a one shot debug warning.
fn evaluate_content(
    arena: &NodeArena,
    host: NodeId,
    value: &ContentValue,
    table: &mut PseudoSideTable,
) -> String {
    match value {
        ContentValue::Literal(s) => s.clone(),
        ContentValue::Attr(name) => {
            let host_elem = match arena.get(host) {
                Some(e) => e,
                None => return String::new(),
            };
            match name.as_str() {
                "id" => host_elem.id.clone().unwrap_or_default(),
                "class" => host_elem.classes.join(" "),
                _ => {
                    let key = (host, name.clone());
                    if table.warned_missing_attrs.insert(key) {
                        // A one shot debug trace keeps logs quiet across
                        // repeating frames while still calling out the miss
                        // the first time it happens per host.
                        #[cfg(debug_assertions)]
                        eprintln!(
                            "unshit-core: attr({name}) missing on host {host:?}, resolving to empty string"
                        );
                    }
                    String::new()
                }
            }
        }
        ContentValue::None | ContentValue::Normal => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::{ElementDef, ElementTree};
    use crate::layout::TextMeasureCtx;
    use crate::style::parse::CompiledStylesheet;

    fn build_simple(
        css: &str,
        def: ElementDef,
    ) -> (NodeArena, TaffyTree<TextMeasureCtx>, NodeId, CompiledStylesheet) {
        let stylesheet = CompiledStylesheet::parse(css);
        let mut arena = NodeArena::new();
        let mut taffy = TaffyTree::<TextMeasureCtx>::new();
        let tree = ElementTree { root: def };
        let root =
            crate::build::build_tree_from_def(&tree.root, &mut arena, &mut taffy, NodeId::DANGLING);
        crate::build::resolve_all_styles(
            &mut arena,
            &stylesheet,
            root,
            NodeId::DANGLING,
            None,
            NodeId::DANGLING,
        );
        (arena, taffy, root, stylesheet)
    }

    #[test]
    fn test_resolver_creates_before_node() {
        let css = r#".card::before { content: "*"; }"#;
        let def = ElementDef::new(Tag::Div).with_class("card");
        let (mut arena, mut taffy, root, stylesheet) = build_simple(css, def);
        let mut table = PseudoSideTable::new();
        resolve_pseudo_elements(
            &mut arena,
            &mut taffy,
            &stylesheet,
            root,
            NodeId::DANGLING,
            None,
            NodeId::DANGLING,
            &mut table,
        );

        let before_id = table.get(root, PseudoElement::Before).expect("before node should exist");

        let before_elem = arena.get(before_id).expect("before element");
        assert!(before_elem.synthetic);
        assert_eq!(before_elem.parent, root);
        assert_eq!(arena.get(root).unwrap().first_child, before_id);

        match &before_elem.content {
            ElementContent::Text(t) => assert_eq!(t, "*"),
            other => panic!("expected Text content, got {:?}", other),
        }
    }

    #[test]
    fn test_resolver_creates_after_node() {
        let css = r#".card::after { content: "!"; }"#;
        let def = ElementDef::new(Tag::Div).with_class("card");
        let (mut arena, mut taffy, root, stylesheet) = build_simple(css, def);
        let mut table = PseudoSideTable::new();
        resolve_pseudo_elements(
            &mut arena,
            &mut taffy,
            &stylesheet,
            root,
            NodeId::DANGLING,
            None,
            NodeId::DANGLING,
            &mut table,
        );

        let after_id = table.get(root, PseudoElement::After).expect("after node should exist");
        assert_eq!(arena.get(root).unwrap().last_child, after_id);

        match &arena.get(after_id).unwrap().content {
            ElementContent::Text(t) => assert_eq!(t, "!"),
            other => panic!("expected Text content, got {:?}", other),
        }
    }

    #[test]
    fn test_resolver_creates_both_with_user_children_between() {
        let css = r#"
            .card::before { content: "<"; }
            .card::after { content: ">"; }
        "#;
        let def = ElementDef::new(Tag::Div)
            .with_class("card")
            .with_child(ElementDef::new(Tag::Span).with_text("one"))
            .with_child(ElementDef::new(Tag::Span).with_text("two"));
        let (mut arena, mut taffy, root, stylesheet) = build_simple(css, def);

        let mut table = PseudoSideTable::new();
        resolve_pseudo_elements(
            &mut arena,
            &mut taffy,
            &stylesheet,
            root,
            NodeId::DANGLING,
            None,
            NodeId::DANGLING,
            &mut table,
        );

        let before_id = table.get(root, PseudoElement::Before).unwrap();
        let after_id = table.get(root, PseudoElement::After).unwrap();

        let children = arena.children(root);
        assert_eq!(children.len(), 4);
        assert_eq!(children[0], before_id);
        assert_eq!(children[3], after_id);
        // Children in between must be the original user spans, unchanged.
        assert!(!arena.get(children[1]).unwrap().synthetic);
        assert!(!arena.get(children[2]).unwrap().synthetic);
    }

    #[test]
    fn test_resolver_content_literal_sets_text() {
        let css = r#".badge::before { content: "tag"; }"#;
        let def = ElementDef::new(Tag::Div).with_class("badge");
        let (mut arena, mut taffy, root, stylesheet) = build_simple(css, def);
        let mut table = PseudoSideTable::new();
        resolve_pseudo_elements(
            &mut arena,
            &mut taffy,
            &stylesheet,
            root,
            NodeId::DANGLING,
            None,
            NodeId::DANGLING,
            &mut table,
        );

        let before_id = table.get(root, PseudoElement::Before).unwrap();
        let elem = arena.get(before_id).unwrap();
        assert_eq!(elem.content, ElementContent::Text("tag".into()));
    }

    #[test]
    fn test_resolver_content_attr_id() {
        let css = r#".badge::before { content: attr(id); }"#;
        let def = ElementDef::new(Tag::Div).with_class("badge").with_id("hello");
        let (mut arena, mut taffy, root, stylesheet) = build_simple(css, def);
        let mut table = PseudoSideTable::new();
        resolve_pseudo_elements(
            &mut arena,
            &mut taffy,
            &stylesheet,
            root,
            NodeId::DANGLING,
            None,
            NodeId::DANGLING,
            &mut table,
        );

        let before_id = table.get(root, PseudoElement::Before).unwrap();
        let elem = arena.get(before_id).unwrap();
        assert_eq!(elem.content, ElementContent::Text("hello".into()));
    }

    #[test]
    fn test_resolver_content_attr_missing() {
        let css = r#".badge::before { content: attr(data_foo); }"#;
        let def = ElementDef::new(Tag::Div).with_class("badge");
        let (mut arena, mut taffy, root, stylesheet) = build_simple(css, def);
        let mut table = PseudoSideTable::new();
        resolve_pseudo_elements(
            &mut arena,
            &mut taffy,
            &stylesheet,
            root,
            NodeId::DANGLING,
            None,
            NodeId::DANGLING,
            &mut table,
        );

        let before_id = table.get(root, PseudoElement::Before).unwrap();
        let elem = arena.get(before_id).unwrap();
        assert_eq!(elem.content, ElementContent::Text(String::new()));
    }

    #[test]
    fn test_resolver_no_content_no_node() {
        // A matching pseudo rule without a content declaration must not
        // allocate a synthetic node: nothing to render.
        let css = r#".card::before { color: red; }"#;
        let def = ElementDef::new(Tag::Div).with_class("card");
        let (mut arena, mut taffy, root, stylesheet) = build_simple(css, def);
        let mut table = PseudoSideTable::new();
        resolve_pseudo_elements(
            &mut arena,
            &mut taffy,
            &stylesheet,
            root,
            NodeId::DANGLING,
            None,
            NodeId::DANGLING,
            &mut table,
        );

        assert!(table.get(root, PseudoElement::Before).is_none());
        assert!(arena.get(root).unwrap().first_child.is_dangling());
    }

    #[test]
    fn test_resolver_changing_content_updates_text() {
        let css = r#".card::before { content: "a"; }"#;
        let def = ElementDef::new(Tag::Div).with_class("card");
        let (mut arena, mut taffy, root, stylesheet) = build_simple(css, def);
        let mut table = PseudoSideTable::new();
        resolve_pseudo_elements(
            &mut arena,
            &mut taffy,
            &stylesheet,
            root,
            NodeId::DANGLING,
            None,
            NodeId::DANGLING,
            &mut table,
        );
        let before_id_first = table.get(root, PseudoElement::Before).unwrap();

        // Swap the stylesheet for one with a different literal and re resolve.
        let css2 = r#".card::before { content: "b"; }"#;
        let stylesheet2 = CompiledStylesheet::parse(css2);
        resolve_pseudo_elements(
            &mut arena,
            &mut taffy,
            &stylesheet2,
            root,
            NodeId::DANGLING,
            None,
            NodeId::DANGLING,
            &mut table,
        );

        let before_id_second = table.get(root, PseudoElement::Before).unwrap();
        assert_eq!(
            before_id_first, before_id_second,
            "pseudo node id should be preserved across content updates"
        );
        assert_eq!(arena.get(before_id_second).unwrap().content, ElementContent::Text("b".into()));
    }

    #[test]
    fn test_resolver_rule_stops_matching_deallocates() {
        // First frame matches, second frame does not.
        let css_match = r#".card::before { content: "x"; }"#;
        let def = ElementDef::new(Tag::Div).with_class("card");
        let (mut arena, mut taffy, root, stylesheet) = build_simple(css_match, def);
        let mut table = PseudoSideTable::new();
        resolve_pseudo_elements(
            &mut arena,
            &mut taffy,
            &stylesheet,
            root,
            NodeId::DANGLING,
            None,
            NodeId::DANGLING,
            &mut table,
        );
        let before_id = table.get(root, PseudoElement::Before).expect("initially allocated");

        // Re resolve with a stylesheet that no longer declares content.
        let css_miss = r#".card::before { color: red; }"#;
        let stylesheet2 = CompiledStylesheet::parse(css_miss);
        resolve_pseudo_elements(
            &mut arena,
            &mut taffy,
            &stylesheet2,
            root,
            NodeId::DANGLING,
            None,
            NodeId::DANGLING,
            &mut table,
        );

        assert!(table.get(root, PseudoElement::Before).is_none());
        // The old node id must be gone from the arena.
        assert!(arena.get(before_id).is_none());
    }

    /// Verify that a ::before pseudo-element with `position: absolute` is
    /// removed from flex flow and does not steal flex space from siblings.
    #[test]
    fn test_absolute_pseudo_element_out_of_flow() {
        use crate::style::types::CssPosition;

        let css = r#"
            .container {
                display: flex;
                flex-direction: column;
                width: 100px;
                height: 100px;
            }
            .container::before {
                content: "";
                position: absolute;
                inset: 0;
            }
            .child {
                flex-grow: 1;
            }
        "#;

        let def = ElementDef::new(Tag::Div)
            .with_class("container")
            .with_child(ElementDef::new(Tag::Div).with_class("child"))
            .with_child(ElementDef::new(Tag::Div).with_class("child"));

        let (mut arena, mut taffy, root, stylesheet) = build_simple(css, def);

        let mut table = PseudoSideTable::new();
        resolve_pseudo_elements(
            &mut arena,
            &mut taffy,
            &stylesheet,
            root,
            NodeId::DANGLING,
            None,
            NodeId::DANGLING,
            &mut table,
        );

        let before_id = table
            .get(root, PseudoElement::Before)
            .expect("before pseudo-element should exist");
        let before_elem = arena.get(before_id).unwrap();
        assert_eq!(
            before_elem.computed_style.position,
            CssPosition::Absolute,
            "pseudo-element should have position: absolute"
        );

        // Run the full layout pipeline.
        let mut font_system = cosmic_text::FontSystem::new();
        crate::layout::sync_element_to_taffy(
            &mut arena, &mut taffy, root, &mut font_system,
        );

        let root_taffy = arena.get(root).unwrap().taffy_node.unwrap();
        let mut cache = crate::layout::TextMeasureCache::default();
        crate::layout::compute_layout(
            &mut taffy, root_taffy, 100.0, 100.0, &mut font_system, &mut cache,
        );
        crate::layout::read_layout_results(&mut arena, &taffy, root, 0.0, 0.0);

        // Collect user (non-synthetic) children.
        let children = arena.children(root);
        let user_children: Vec<NodeId> = children
            .into_iter()
            .filter(|id| !arena.get(*id).unwrap().synthetic)
            .collect();
        assert_eq!(user_children.len(), 2, "should have exactly 2 user children");

        // With the pseudo-element out of flow, each child should get 50px
        // height (100px / 2 children with flex-grow: 1). If the pseudo
        // participates in flow it would steal a slot and children would
        // get ~33px each.
        for (i, child_id) in user_children.iter().enumerate() {
            let child = arena.get(*child_id).unwrap();
            let h = child.layout_rect.height;
            assert!(
                (h - 50.0).abs() < 1.0,
                "child {} should be ~50px tall (got {:.1}px); pseudo-element \
                 is incorrectly participating in flex layout",
                i, h,
            );
        }

        let c0 = arena.get(user_children[0]).unwrap();
        let c1 = arena.get(user_children[1]).unwrap();
        assert!(
            c0.layout_rect.y.abs() < 1.0,
            "first child y should be ~0 (got {:.1})",
            c0.layout_rect.y,
        );
        assert!(
            (c1.layout_rect.y - 50.0).abs() < 1.0,
            "second child y should be ~50 (got {:.1})",
            c1.layout_rect.y,
        );
    }

    #[test]
    fn test_resolver_creates_placeholder_node() {
        let css = r#".input::placeholder { content: "Type here..."; }"#;
        let def = ElementDef::new(Tag::Div).with_class("input");
        let (mut arena, mut taffy, root, stylesheet) = build_simple(css, def);
        let mut table = PseudoSideTable::new();
        resolve_pseudo_elements(
            &mut arena,
            &mut taffy,
            &stylesheet,
            root,
            NodeId::DANGLING,
            None,
            NodeId::DANGLING,
            &mut table,
        );

        let placeholder_id = table
            .get(root, PseudoElement::Placeholder)
            .expect("placeholder node should exist");

        let placeholder_elem = arena.get(placeholder_id).expect("placeholder element");
        assert!(placeholder_elem.synthetic);
        assert_eq!(placeholder_elem.parent, root);

        match &placeholder_elem.content {
            ElementContent::Text(t) => assert_eq!(t, "Type here..."),
            other => panic!("expected Text content, got {:?}", other),
        }
    }
}
